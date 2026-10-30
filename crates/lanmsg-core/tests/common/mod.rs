// P2 Task 4：Service 集成测试辅助。
// TestNode 封装 Service、临时目录与事件接收端；node() 在共享 MemoryBus 上启动。
// node_with() 接管已有目录（重启场景）或注入平台钩子。
// 注释中文，标识符英文。
// 部分辅助接口按计划为后续任务（pair/befriend 等）预留，此处允许暂未使用。
#![allow(dead_code)]

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;
use tokio::sync::broadcast;

use lanmsg_api::types::{ApiError, Event, InitMode, Request, Response};
use lanmsg_core::discovery::{DiscoveryMode, MemoryBus};
use lanmsg_core::keystore::FileKeyStore;
use lanmsg_core::platform::PlatformHooks;
use lanmsg_core::{Service, ServiceConfig};

/// 测试节点：Service + 数据目录 + 事件接收端 + user_id 缓存。
pub struct TestNode {
    pub svc: Service,
    pub dir: TempDir,
    pub events: broadcast::Receiver<Event>,
    /// init_first 填充的 user_id 缓存（同步访问用）。
    self_user: Mutex<Option<String>>,
}

/// 在共享 bus 上以临时目录启动测试节点（port 0、Memory 发现）。
pub async fn node(bus: &MemoryBus) -> TestNode {
    node_with(bus, tempfile::tempdir().unwrap(), PlatformHooks::default()).await
}

/// 以 Disabled 发现启动测试节点：不自动发现，仅手动互拨（排除 Memory 总线自动
/// 发现与手动拨号并存时的双连接裁决竞态）。
pub async fn node_offline() -> TestNode {
    let dir = tempfile::tempdir().unwrap();
    let svc = Service::start(ServiceConfig {
        data_dir: dir.path().to_path_buf(),
        listen_port: 0,
        discovery: DiscoveryMode::Disabled,
        keystore: Arc::new(FileKeyStore::new(dir.path().join("keys"))),
        static_peers: Vec::new(),
        platform: PlatformHooks::default(),
        localsend_port: 0,
    })
    .await
    .expect("Service 启动失败");
    let events = svc.subscribe();
    TestNode {
        svc,
        dir,
        events,
        self_user: Mutex::new(None),
    }
}

/// 指定目录与平台钩子启动测试节点：目录已含身份与数据时即“重启”。
pub async fn node_with(bus: &MemoryBus, dir: TempDir, hooks: PlatformHooks) -> TestNode {
    let svc = Service::start(ServiceConfig {
        data_dir: dir.path().to_path_buf(),
        listen_port: 0,
        discovery: DiscoveryMode::Memory(bus.clone()),
        keystore: Arc::new(FileKeyStore::new(dir.path().join("keys"))),
        static_peers: Vec::new(),
        platform: hooks,
        localsend_port: 0,
    })
    .await
    .expect("Service 启动失败");
    let events = svc.subscribe();
    TestNode {
        svc,
        dir,
        events,
        self_user: Mutex::new(None),
    }
}

/// 以 Disabled 发现重启已有目录的节点（身份与数据保留）。
pub async fn node_offline_with(dir: TempDir, hooks: PlatformHooks) -> TestNode {
    let svc = Service::start(ServiceConfig {
        data_dir: dir.path().to_path_buf(),
        listen_port: 0,
        discovery: DiscoveryMode::Disabled,
        keystore: Arc::new(FileKeyStore::new(dir.path().join("keys"))),
        static_peers: Vec::new(),
        platform: hooks,
        localsend_port: 0,
    })
    .await
    .expect("Service 启动失败");
    let events = svc.subscribe();
    TestNode {
        svc,
        dir,
        events,
        self_user: Mutex::new(None),
    }
}

impl TestNode {
    /// 发送请求，出错时 panic 并打印。
    pub async fn call(&self, r: Request) -> Response {
        match self.try_call(r.clone()).await {
            Ok(resp) => resp,
            Err(e) => panic!("请求失败: {r:?} → {e:?}"),
        }
    }

    /// 发送请求，保留错误。
    pub async fn try_call(&self, r: Request) -> Result<Response, ApiError> {
        self.svc.handle(r).await
    }

    /// 初始化首台设备（display_name = device_name = name），返回 user_id。
    pub async fn init_first(&self, name: &str) -> String {
        self.call(Request::Init {
            display_name: name.into(),
            device_name: name.into(),
            mode: InitMode::FirstDevice,
        })
        .await;
        let user = match self.call(Request::GetSelf).await {
            Response::SelfInfo { value } => value.user_id.expect("初始化后应有 user_id"),
            other => panic!("期望 SelfInfo，实际 {other:?}"),
        };
        *self.self_user.lock().unwrap() = Some(user.clone());
        user
    }

    /// 取缓存的 user_id（init_first 之后可用）。
    pub fn user_id(&self) -> String {
        self.self_user
            .lock()
            .unwrap()
            .clone()
            .expect("user_id 未缓存：先调用 init_first")
    }

    /// 取下一条匹配事件，超时 panic。
    pub async fn wait_event(&mut self, f: impl Fn(&Event) -> bool, secs: u64) -> Event {
        let deadline = Duration::from_secs(secs);
        loop {
            match tokio::time::timeout(deadline, self.events.recv()).await {
                Ok(Ok(event)) => {
                    if f(&event) {
                        return event;
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(broadcast::error::RecvError::Closed)) => panic!("事件通道已关闭"),
                Err(_) => panic!("{secs} 秒内未收到匹配事件"),
            }
        }
    }

    /// 在 millis 毫秒内出现匹配事件即 panic。
    pub async fn no_event(&mut self, f: impl Fn(&Event) -> bool, millis: u64) {
        let deadline = Duration::from_millis(millis);
        loop {
            match tokio::time::timeout(deadline, self.events.recv()).await {
                Err(_) => return, // 超时未出现，通过
                Ok(Ok(event)) => {
                    if f(&event) {
                        panic!("不应出现的事件出现了: {event:?}");
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(broadcast::error::RecvError::Closed)) => panic!("事件通道已关闭"),
            }
        }
    }
}

/// 轮询直到异步条件成立，50ms 间隔，超时 panic。
pub async fn wait_until<F, Fut>(f: F, secs: u64)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    let start = std::time::Instant::now();
    loop {
        if f().await {
            return;
        }
        if start.elapsed() > Duration::from_secs(secs) {
            panic!("{secs} 秒内条件未成立");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
