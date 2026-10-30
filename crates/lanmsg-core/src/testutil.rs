// P1 核心骨架：测试辅助（Task 10）。
// Node / pair_of_nodes / next_event，用于消息收发测试。
// 注释中文，标识符英文。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, watch, RwLock};

use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId, MsgId};
use lanmsg_proto::types::{DeviceType, MsgBody};

use crate::error::CoreError;
use crate::hlc::HlcClock;
use crate::identity::Identity;
use crate::keystore::FileKeyStore;
use crate::msg::Messaging;
use crate::net::{ConnManager, HelloInfo, PeerInfo};
use crate::store::Store;
use lanmsg_api::types::Event;

/// 测试节点：封装 Store、Identity、ConnManager、Messaging 与事件接收端。
pub struct Node {
    pub store: Arc<Store>,
    pub identity: Arc<RwLock<Identity>>,
    pub conns: ConnManager,
    pub msg: Messaging,
    pub events: broadcast::Receiver<Event>,
    pub user: lanmsg_proto::ids::UserId,
    pub device: DeviceId,
    _dir: tempfile::TempDir,
}

impl Node {
    /// 打开与指定用户的 DM 会话。
    pub fn open_dm(&self, peer_user: &lanmsg_proto::ids::UserId) -> Result<ConvId, CoreError> {
        self.msg.open_dm(peer_user)
    }

    /// 发送消息到指定会话。
    pub async fn send(&self, conv: &ConvId, body: MsgBody) -> Result<MsgId, CoreError> {
        self.msg.send(conv, body).await
    }

    /// 处理对端发来的帧。
    pub async fn on_frame(&self, from: DeviceId, frame: Frame) -> Result<(), CoreError> {
        self.msg.on_frame(from, frame).await
    }
}

/// 当前墙钟时间（毫秒）。
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

use std::time::SystemTime;

/// 创建单个测试节点：已 init_first_device、端口 0、内存 Store、路由任务已启动。
///
/// 路由任务：
/// - `Up(p)` → `upsert_device`/`upsert_user`
/// - `Frame(d, f)` → `msg.on_frame(d, f)`
pub async fn node() -> Node {
    let _dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());

    let ks = FileKeyStore::new(_dir.path().join("keys"));

    let identity = Arc::new(RwLock::new(Identity::load_or_create(&ks, &store).unwrap()));

    // init_first_device。
    {
        let mut id = identity.write().await;
        id.init_first_device(
            &ks,
            &store,
            "测试设备",
            DeviceType::Desktop,
            1_700_000_000_000,
        )
        .unwrap();
    }

    let user = {
        let id = identity.read().await;
        id.user_id().unwrap()
    };
    let device = {
        let id = identity.read().await;
        id.device_id()
    };

    let clock = Arc::new(Mutex::new(HlcClock::new()));

    // ConnManager：创建事件通道，发送端传给 bind，接收端留给路由任务。
    let (hello_tx, hello_rx) = watch::channel(HelloInfo {
        display_name: "测试".into(),
        cert: None,
        listen_port: 0,
    });
    let (conn_events_tx, conn_events_rx) = mpsc::channel(32);
    let filter = Arc::new(|_: &PeerInfo| true);

    let conns = ConnManager::bind(identity.clone(), 0, hello_rx, filter, conn_events_tx)
        .await
        .unwrap();

    // 更新 HelloInfo 中的 cert。
    {
        let id = identity.read().await;
        if let Some(cert) = &id.cert {
            let _ = hello_tx.send_replace(HelloInfo {
                display_name: cert.device_name.clone(),
                cert: Some(cert.clone()),
                listen_port: 0,
            });
        }
    }

    // Messaging。
    let (event_tx, event_rx) = broadcast::channel(128);
    let msg = Messaging::new(
        store.clone(),
        identity.clone(),
        clock.clone(),
        conns.clone(),
        event_tx,
    );

    // 路由任务。
    let msg_route = msg.clone();
    let store_route = store.clone();

    tokio::spawn(async move {
        let mut rx = conn_events_rx;
        while let Some(event) = rx.recv().await {
            match event {
                crate::net::ConnEvent::Up(peer_info) => {
                    if let Some(cert) = &peer_info.cert {
                        let _ = store_route.upsert_device(&crate::store::DeviceRow {
                            device: peer_info.device,
                            user: cert.user,
                            cert: cert.clone(),
                            name: peer_info.display_name.clone(),
                            device_type: cert.device_type,
                            last_addr: Some(peer_info.addr.to_string()),
                            last_path: Some(peer_info.path),
                            last_seen: Some(now_millis()),
                            revoked: false,
                        });
                        let _ = store_route.upsert_user(&crate::store::UserRow {
                            user: cert.user,
                            display_name: peer_info.display_name.clone(),
                            is_self: false,
                            updated_at: now_millis(),
                        });
                    }
                }
                crate::net::ConnEvent::Frame(from, frame) => {
                    let _ = msg_route.on_frame(from, frame).await;
                }
                crate::net::ConnEvent::Down(_) => {}
            }
        }
    });

    Node {
        store,
        identity,
        conns,
        msg,
        events: event_rx,
        user,
        device,
        _dir,
    }
}

/// 创建两个节点并建立连接：A.conns.connect(B 地址)，等待双方 connected。
/// 等待双方设备信息已入库（路由任务处理完 Up 事件）。
async fn wait_devices_synced(a: &Node, b: &Node) {
    let start = std::time::Instant::now();
    loop {
        let a_has_b = b.store.get_device(&a.device).ok().flatten().is_some();
        let b_has_a = a.store.get_device(&b.device).ok().flatten().is_some();
        if a_has_b && b_has_a {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("双方设备未在 5 秒内入库");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 创建两个节点并建立连接：A.conns.connect(B 地址)，等待双方 connected 且设备互相入库。
pub async fn pair_of_nodes() -> (Node, Node) {
    let a = node().await;
    let b = node().await;

    let b_device = b.device;
    let a_device = a.device;

    let _ = a.conns.connect(b.conns.local_addr(), false).await;

    // 等待双方 connected。
    let start = std::time::Instant::now();
    loop {
        if a.conns.is_connected(&b_device) && b.conns.is_connected(&a_device) {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("双方未在 5 秒内建立连接");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 等待 Up 事件被路由任务处理，双方设备均入库。
    wait_devices_synced(&a, &b).await;

    (a, b)
}

/// 从广播接收端取下一条匹配 `pred` 的事件，5 秒超时 panic。
pub async fn next_event(
    rx: &mut broadcast::Receiver<Event>,
    pred: impl Fn(&Event) -> bool,
) -> Event {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Ok(event)) => {
                if pred(&event) {
                    return event;
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) => panic!("事件通道已关闭"),
            Err(_) => panic!("5 秒内未收到匹配事件"),
        }
    }
}
