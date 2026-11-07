// lanmsg-ffi：core 的 UniFFI 绑定（Android 经 JNA 调用）。
// 所有能力经 `LanMsgCore::request` 转给 `Service::handle`，事件经监听器回调转出。
// 注释中文，标识符英文。

pub mod adapters;
#[cfg(target_os = "android")]
mod android_log;

use std::sync::{Arc, Mutex};

use lanmsg_api::{ApiError, ErrorCode, Event, Request, Response};
use lanmsg_core::{Service, ServiceConfig};
use lanmsg_proto::consts::{LOCALSEND_PORT, SERVICE_PORT};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use adapters::{platform_hooks, ForeignKeyStore};

uniffi::setup_scaffolding!("lanmsg_ffi");

/// FFI 错误：`ApiError` 是结构体，不能直接作为 UniFFI 错误类型，故在此包一层。
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("{code:?}: {message}")]
    Api { code: ErrorCode, message: String },
}

impl From<ApiError> for FfiError {
    fn from(e: ApiError) -> Self {
        FfiError::Api {
            code: e.code,
            message: e.message,
        }
    }
}

/// 外部回调抛出异常（或返回类型不符）时的兜底转换。
impl From<uniffi::UnexpectedUniFFICallbackError> for FfiError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        FfiError::Api {
            code: ErrorCode::Internal,
            message: e.to_string(),
        }
    }
}

fn internal(message: impl Into<String>) -> FfiError {
    FfiError::Api {
        code: ErrorCode::Internal,
        message: message.into(),
    }
}

/// 外部（Kotlin）提供的密钥库。
#[uniffi::export(with_foreign)]
pub trait FfiKeyStore: Send + Sync {
    fn load(&self, name: String) -> Result<Option<Vec<u8>>, FfiError>;
    fn save(&self, name: String, bytes: Vec<u8>) -> Result<(), FfiError>;
    fn delete(&self, name: String) -> Result<(), FfiError>;
}

/// 外部生成的缩略图。
#[derive(uniffi::Record)]
pub struct ThumbnailResult {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub duration_ms: Option<u64>,
}

/// 外部（Kotlin）提供的平台能力。
#[uniffi::export(with_foreign)]
pub trait FfiPlatform: Send + Sync {
    fn is_metered(&self) -> bool;
    fn is_wifi(&self) -> bool;
    fn thumbnail(&self, path: String, mime: String, max_edge: u32) -> Option<ThumbnailResult>;
}

/// 事件监听器。
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: Event);
    /// 广播通道 Lagged 时调用，客户端应全量重拉。
    fn on_lagged(&self);
}

/// core 句柄：持有自己的 tokio Runtime、Service 与事件转发任务。
#[derive(uniffi::Object)]
pub struct LanMsgCore {
    // 仅在 Drop 中取走并后台关闭，避免在异步上下文里同步 drop Runtime。
    runtime: Option<tokio::runtime::Runtime>,
    service: Mutex<Option<Service>>,
    forwarder: Mutex<Option<JoinHandle<()>>>,
}

impl LanMsgCore {
    fn handle(&self) -> Result<&tokio::runtime::Handle, FfiError> {
        self.runtime
            .as_ref()
            .map(|r| r.handle())
            .ok_or_else(|| internal("已关闭"))
    }

    fn current_service(&self) -> Result<Service, FfiError> {
        let guard = self.service.lock().map_err(|_| internal("状态锁已损坏"))?;
        guard.clone().ok_or_else(|| internal("已关闭"))
    }

    fn abort_forwarder(&self) {
        if let Ok(mut f) = self.forwarder.lock() {
            if let Some(h) = f.take() {
                h.abort();
            }
        }
    }
}

impl Drop for LanMsgCore {
    fn drop(&mut self) {
        self.abort_forwarder();
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl LanMsgCore {
    /// 构造并启动 Service。会阻塞到启动完成，Kotlin 必须在 IO 线程调用。
    #[uniffi::constructor]
    pub fn new(
        data_dir: String,
        keystore: Arc<dyn FfiKeyStore>,
        platform: Arc<dyn FfiPlatform>,
    ) -> Result<Arc<Self>, FfiError> {
        #[cfg(target_os = "android")]
        android_log::init();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lanmsg-core")
            .enable_all()
            .build()
            .map_err(|e| internal(format!("创建运行时失败: {e}")))?;
        let cfg = ServiceConfig {
            data_dir: data_dir.into(),
            listen_port: SERVICE_PORT,
            discovery: lanmsg_core::discovery::DiscoveryMode::Multicast,
            keystore: Arc::new(ForeignKeyStore(keystore)),
            static_peers: vec![],
            platform: platform_hooks(platform),
            localsend_port: LOCALSEND_PORT,
        };
        let service = runtime
            .block_on(Service::start(cfg))
            .map_err(|e| internal(format!("启动失败: {e}")))?;
        tracing::info!("core started");
        Ok(Arc::new(Self {
            runtime: Some(runtime),
            service: Mutex::new(Some(service)),
            forwarder: Mutex::new(None),
        }))
    }

    /// 处理一个请求；在 core 自己的 Runtime 上执行。
    pub async fn request(&self, req: Request) -> Result<Response, FfiError> {
        let service = self.current_service()?;
        let handle = self.handle()?.clone();
        match handle.spawn(async move { service.handle(req).await }).await {
            Ok(r) => r.map_err(FfiError::from),
            Err(e) => Err(internal(format!("请求任务异常: {e}"))),
        }
    }

    /// 设置（或清除）事件监听器；会取消旧的转发任务。
    pub fn set_event_listener(&self, listener: Option<Arc<dyn EventListener>>) {
        self.abort_forwarder();
        let Some(l) = listener else { return };
        let (Ok(service), Ok(handle)) = (self.current_service(), self.handle()) else {
            return;
        };
        let mut rx = service.subscribe();
        let task = handle.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => l.on_event(ev),
                    Err(RecvError::Lagged(_)) => l.on_lagged(),
                    Err(RecvError::Closed) => break,
                }
            }
        });
        if let Ok(mut f) = self.forwarder.lock() {
            *f = Some(task);
        }
    }

    /// 关闭 Service；之后 `request` 返回错误。重复调用无副作用。
    pub fn shutdown(&self) {
        self.abort_forwarder();
        let service = match self.service.lock() {
            Ok(mut g) => g.take(),
            Err(_) => None,
        };
        if let (Some(svc), Some(rt)) = (service, self.runtime.as_ref()) {
            rt.block_on(svc.shutdown());
        }
    }
}
