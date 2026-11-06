// lanmsgd 进程运行时（P3 Task 8）：线程模型、无头判定、缩略图缓存清理。
// 线程模型：tokio 多线程运行时固定 2 个 worker（控制常驻内存），由本模块持有；
// 主线程留给托盘/界面消息循环（Task 9），无头时阻塞等待退出信号。
// 注释中文，标识符英文。

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use lanmsg_api::ipc::{endpoint_name, IpcListener};
use lanmsg_core::discovery::DiscoveryMode;
use lanmsg_core::keystore_os::os_keystore;
use lanmsg_core::thumb_cache::enforce_thumb_cache_limit;
use lanmsg_core::{PlatformHooks, Service, ServiceConfig};
use lanmsg_proto::consts::{LOCALSEND_PORT, THUMB_CACHE_LIMIT};

use crate::thumbnailer::SubprocessThumbnailer;
use crate::{ipc_server, Args};

/// 缩略图缓存清理间隔。
const THUMB_CACHE_SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

/// IPC 端点已被另一个 lanmsgd 占用。
#[derive(Debug)]
pub struct AlreadyRunning;

impl std::fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lanmsgd 已在运行")
    }
}

impl std::error::Error for AlreadyRunning {}

/// 守护进程运行时：Service + 持有它的 tokio 运行时。
pub struct DaemonRuntime {
    pub service: Service,
    rt: tokio::runtime::Runtime,
}

/// 建 tokio 运行时、启动 Service 与 IPC 服务；返回时 IPC 已可用。
/// IPC 端点被占用时返回 `AlreadyRunning`；系统密钥库不可用等错误带上下文返回。
pub fn start_runtime(args: &Args) -> Result<DaemonRuntime> {
    let data_dir = args.resolved_data_dir();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("创建 tokio 运行时失败")?;

    let service = rt.block_on(async {
        // 先占 IPC 端点：已有实例在运行时尽早退出，不去动数据库与端口。
        let endpoint = endpoint_name(&data_dir);
        let listener = match IpcListener::bind(&endpoint) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                return Err(anyhow::Error::new(AlreadyRunning))
            }
            Err(e) => return Err(anyhow::Error::new(e).context("绑定 IPC 端点失败")),
        };

        let cache_dir = data_dir.join("cache");
        let service = Service::start(ServiceConfig {
            data_dir: data_dir.clone(),
            listen_port: args.port,
            discovery: if args.no_multicast {
                DiscoveryMode::Disabled
            } else {
                DiscoveryMode::Multicast
            },
            keystore: os_keystore(&data_dir),
            static_peers: args.peers.clone(),
            platform: PlatformHooks {
                thumbnailer: Some(Arc::new(SubprocessThumbnailer::new(cache_dir.clone()))),
                ..PlatformHooks::default()
            },
            localsend_port: LOCALSEND_PORT,
        })
        .await
        .context("启动服务失败（若数据目录标记使用系统密钥库而系统库不可用，请检查密钥库服务）")?;
        tracing::info!("lanmsgd 已启动，IPC 端点: {endpoint}");

        // IPC 接入循环。
        let svc = service.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(stream) => {
                        tokio::spawn(ipc_server::serve_connection(svc.clone(), stream));
                    }
                    Err(e) => {
                        tracing::warn!("IPC accept 失败: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });

        // 缩略图缓存清理：启动一次，之后每小时一次。
        let thumbs_dir = cache_dir.join("thumbs");
        tokio::spawn(async move {
            loop {
                let dir = thumbs_dir.clone();
                match tokio::task::spawn_blocking(move || {
                    enforce_thumb_cache_limit(&dir, THUMB_CACHE_LIMIT)
                })
                .await
                {
                    Ok(Ok(freed)) if freed > 0 => tracing::info!("缩略图缓存已清理 {freed} 字节"),
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => tracing::warn!("缩略图缓存清理失败: {e}"),
                    Err(e) => tracing::warn!("缩略图缓存清理任务异常: {e}"),
                }
                tokio::time::sleep(THUMB_CACHE_SWEEP_INTERVAL).await;
            }
        });

        Ok(service)
    })?;

    Ok(DaemonRuntime { service, rt })
}

impl DaemonRuntime {
    /// 阻塞当前线程直到收到 Ctrl-C，然后关闭服务。
    /// （`Request::Shutdown` 由 IPC 层直接关闭服务并退出进程。）
    pub fn block_until_shutdown(self) {
        let service = self.service.clone();
        self.rt.block_on(async move {
            if let Err(e) = tokio::signal::ctrl_c().await {
                tracing::warn!("监听 Ctrl-C 失败: {e}");
                // 无法监听信号时不退出，继续常驻服务。
                std::future::pending::<()>().await;
            }
            service.shutdown().await;
        });
        self.rt.shutdown_background();
    }

    #[allow(dead_code)] // 供 Task 9 的托盘模块使用，接入前主流程尚未调用。
    /// 把核心事件转发给回调（在运行时线程上调用 `f`，回调需自行跨线程投递到界面线程）。
    pub fn spawn_event_forwarder(&self, f: impl Fn(lanmsg_api::Event) + Send + 'static) {
        let mut events = self.service.subscribe();
        self.rt.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(e) => f(e),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    #[allow(dead_code)] // 供 Task 9 的托盘模块使用，接入前主流程尚未调用。
    /// 关闭服务并停止运行时（界面主动退出时使用）。
    pub fn shutdown(self) {
        let service = self.service.clone();
        self.rt.block_on(async move { service.shutdown().await });
        self.rt.shutdown_background();
    }
}

/// 是否应以无头模式运行。
/// `force`（`--headless`）为真则无头；Windows/macOS 恒有图形会话；
/// Linux 在 `DISPLAY` 与 `WAYLAND_DISPLAY` 都为空时无头。
/// 注意：只看环境变量，不保证显示服务真的可用；连不上时由 Task 9 的 `run_tray` 兜底降级。
pub fn is_headless(env: &dyn Fn(&str) -> Option<String>, force: bool) -> bool {
    if force {
        return true;
    }
    if !cfg!(target_os = "linux") {
        return false;
    }
    let unset = |k: &str| env(k).is_none_or(|v| v.is_empty());
    unset("DISPLAY") && unset("WAYLAND_DISPLAY")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_detection_without_display_env() {
        let none = |_: &str| None;
        let x11 = |k: &str| (k == "DISPLAY").then(|| ":0".to_string());
        #[cfg(target_os = "linux")]
        {
            let empty = |_: &str| Some(String::new());
            assert!(is_headless(&none, false));
            assert!(!is_headless(&x11, false));
            assert!(is_headless(&empty, false));
        }
        assert!(is_headless(&x11, true));
        assert!(is_headless(&none, true));
    }
}
