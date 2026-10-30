// P1 核心骨架：Service 组装（Task 11）。
// Service 组合身份、存储、发现、连接管理与消息模块；所有客户端能力经 handle() 暴露。
// 启动顺序：Store → Identity → ConnManager::bind → Discovery::spawn → 路由任务 → 静态对端拨号。
// 注释中文，标识符英文。

pub mod handlers;
pub mod views;

use std::net::SocketAddr;
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::SystemTime;

use tokio::sync::{broadcast, mpsc, watch, RwLock};

use lanmsg_api::types::Event;
use lanmsg_proto::consts::{PROTO_VERSION, RETRY_MAX, RETRY_MIN};
use lanmsg_proto::ids::UserId;
use lanmsg_proto::sign::{sign_with_zeroed_sig, DeviceKey};
use lanmsg_proto::types::{Announce, Signature64, Status};

use crate::discovery::{Discovery, DiscoveryEvent, DiscoveryMode};
use crate::error::CoreError;
use crate::hlc::HlcClock;
use crate::identity::Identity;
use crate::keystore::KeyStore;
use crate::msg::Messaging;
use crate::net::{ConnEvent, ConnManager, HelloInfo, PeerInfo};
use crate::platform::PlatformHooks;
use crate::store::{DeviceRow, Store, UserRow};

/// 服务配置（C6）。
pub struct ServiceConfig {
    /// 数据目录（C8）。
    pub data_dir: std::path::PathBuf,
    /// 监听端口，0 = 系统分配；被占用时回退 0。
    pub listen_port: u16,
    /// 发现模式。
    pub discovery: DiscoveryMode,
    /// 密钥库。
    pub keystore: Arc<dyn KeyStore>,
    /// 启动即尝试直连的地址（测试与手动添加共用）。
    pub static_peers: Vec<SocketAddr>,
    /// 平台钩子（P1 为空）。
    pub platform: PlatformHooks,
    /// LocalSend 监听端口（P1 仅定义，P5 使用）。
    #[allow(dead_code)]
    pub localsend_port: u16,
}

/// 节点可变状态：在线状态与自身用户（accept_filter 与路由同步读取）。
pub(crate) struct NodeState {
    pub(crate) status: Status,
    pub(crate) self_user: Option<UserId>,
}

/// Service 内部共享状态（C6）。
pub(crate) struct Inner {
    pub(crate) store: Arc<Store>,
    pub(crate) identity: Arc<RwLock<Identity>>,
    pub(crate) keystore: Arc<dyn KeyStore>,
    pub(crate) conns: ConnManager,
    pub(crate) msg: Messaging,
    pub(crate) events: broadcast::Sender<Event>,
    pub(crate) announce_tx: watch::Sender<Option<Announce>>,
    pub(crate) hello_tx: watch::Sender<HelloInfo>,
    pub(crate) local_addr: SocketAddr,
    pub(crate) state: Arc<StdRwLock<NodeState>>,
    pub(crate) discovery: Discovery,
}

/// 核心服务（C6）。内部状态由 Arc 持有，Clone 廉价。
#[derive(Clone)]
pub struct Service {
    inner: Arc<Inner>,
}

/// 当前墙钟时间（毫秒）。
pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Service {
    /// 启动服务：打开 Store → 加载身份 → 绑定连接 → 启动发现与路由。
    pub async fn start(cfg: ServiceConfig) -> Result<Service, CoreError> {
        let ServiceConfig {
            data_dir,
            listen_port,
            discovery,
            keystore,
            static_peers,
            platform: _platform,
            localsend_port: _localsend_port,
        } = cfg;

        std::fs::create_dir_all(&data_dir)?;
        let store = Arc::new(Store::open(&data_dir.join("lanmsg.db"))?);
        let identity = Arc::new(RwLock::new(Identity::load_or_create(
            keystore.as_ref(),
            &store,
        )?));
        let clock = Arc::new(std::sync::Mutex::new(HlcClock::new()));

        let (events_tx, _events_rx) = broadcast::channel(1024);
        let (hello_tx, hello_rx) = watch::channel(HelloInfo {
            display_name: String::new(),
            cert: None,
        });
        let (announce_tx, announce_rx) = watch::channel(None);

        let self_user = identity.read().await.user_id();
        let state = Arc::new(StdRwLock::new(NodeState {
            status: Status::Online,
            self_user,
        }));

        // accept_filter：cert 为 None 一律拒绝；Invisible 时只接受本人设备。
        let filter_state = state.clone();
        let accept_filter = Arc::new(move |p: &PeerInfo| -> bool {
            let Some(cert) = &p.cert else {
                return false;
            };
            let st = filter_state.read().unwrap();
            match st.status {
                Status::Invisible => st.self_user.is_some_and(|u| cert.user == u),
                _ => true,
            }
        });

        let (conn_tx, conn_rx) = mpsc::channel::<ConnEvent>(256);
        let conns = ConnManager::bind(
            identity.clone(),
            listen_port,
            hello_rx,
            accept_filter,
            conn_tx,
        )
        .await?;
        let local_addr = conns.local_addr();

        let msg = Messaging::new(
            store.clone(),
            identity.clone(),
            clock.clone(),
            conns.clone(),
            events_tx.clone(),
        );

        // 已初始化：立即填充 HelloInfo 与 announce（announce 依赖 bind 后才知道端口）。
        let display = store.get_setting("display_name")?.unwrap_or_default();
        if identity.read().await.is_initialized() {
            let cert = identity.read().await.cert.clone();
            hello_tx.send_replace(HelloInfo {
                display_name: display.clone(),
                cert,
            });
            if let Some(ann) =
                build_announce(&identity, &display, Status::Online, local_addr.port()).await
            {
                announce_tx.send_replace(Some(ann));
            }
        }

        let (disc_tx, disc_rx) = mpsc::channel::<DiscoveryEvent>(64);
        let self_device = identity.read().await.device_id();
        let discovery = Discovery::spawn(discovery, self_device, announce_rx, disc_tx);

        let inner = Arc::new(Inner {
            store,
            identity,
            keystore,
            conns,
            msg,
            events: events_tx,
            announce_tx,
            hello_tx,
            local_addr,
            state,
            discovery,
        });
        let svc = Service {
            inner: inner.clone(),
        };

        tokio::spawn(routing_task(inner.clone(), conn_rx, disc_rx));
        for addr in static_peers {
            tokio::spawn(static_peer_task(inner.clone(), addr));
        }

        Ok(svc)
    }

    /// 处理客户端请求（C6：所有客户端能力经此暴露）。
    pub async fn handle(
        &self,
        req: lanmsg_api::types::Request,
    ) -> Result<lanmsg_api::types::Response, lanmsg_api::types::ApiError> {
        handlers::handle(&self.inner, req).await
    }

    /// 订阅服务事件（容量 1024，落后时客户端应重新拉取）。
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    /// 实际监听地址（测试用）。
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr
    }

    /// 关闭发现与所有连接。
    pub async fn shutdown(&self) {
        self.inner.discovery.shutdown();
        self.inner.conns.shutdown().await;
    }
}

/// 构造已签名的自身通告；未初始化返回 None。
pub(crate) async fn build_announce(
    identity: &RwLock<Identity>,
    display_name: &str,
    status: Status,
    port: u16,
) -> Option<Announce> {
    let id = identity.read().await;
    let cert = id.cert.clone()?;
    let mut ann = Announce {
        proto: PROTO_VERSION,
        cert,
        display_name: display_name.to_string(),
        avatar_hash: None,
        ports: vec![port],
        status,
        sig: Signature64([0u8; 64]),
    };
    let dev = DeviceKey::from_bytes(&id.device.to_bytes());
    sign_with_zeroed_sig(&mut ann, |b| dev.sign(b));
    Some(ann)
}

/// 路由任务：连接事件与发现事件分发。
async fn routing_task(
    inner: Arc<Inner>,
    mut conn_rx: mpsc::Receiver<ConnEvent>,
    mut disc_rx: mpsc::Receiver<DiscoveryEvent>,
) {
    let mut disc_open = true;
    loop {
        tokio::select! {
            evt = conn_rx.recv() => match evt {
                Some(ConnEvent::Up(peer)) => on_up(&inner, peer).await,
                Some(ConnEvent::Down(device)) => {
                    let _ = inner
                        .events
                        .send(Event::PeerOffline { device: device.to_hex() });
                }
                Some(ConnEvent::Frame(d, f)) => {
                    let _ = inner.msg.on_frame(d, f).await;
                }
                None => break,
            },
            evt = disc_rx.recv(), if disc_open => match evt {
                Some(de) => on_discovery(&inner, de).await,
                None => disc_open = false,
            },
        }
    }
}

/// 连接上线：设备与用户入库，发 PeerOnline。
async fn on_up(inner: &Inner, peer: PeerInfo) {
    let now = now_millis();
    if let Some(cert) = &peer.cert {
        let _ = inner.store.upsert_device(&DeviceRow {
            device: peer.device,
            user: cert.user,
            cert: cert.clone(),
            name: cert.device_name.clone(),
            device_type: cert.device_type,
            last_addr: Some(peer.dial_addr.to_string()),
            last_path: Some(peer.path.clone()),
            last_seen: Some(now),
            revoked: false,
        });
        let _ = inner.store.upsert_user(&UserRow {
            user: cert.user,
            display_name: peer.display_name.clone(),
            is_self: false,
            updated_at: now,
        });
    }
    let self_user = inner.state.read().unwrap().self_user;
    let _ = inner.events.send(Event::PeerOnline {
        peer: views::peer_view(&peer, self_user.as_ref()),
    });
}

/// 发现事件：未连接且自身设备 ID 较小时出站拨号；Invisible 不主动外联其他用户。
async fn on_discovery(inner: &Inner, de: DiscoveryEvent) {
    let peer_device = de.announce.cert.device;
    if inner.conns.is_connected(&peer_device) {
        return;
    }
    {
        let st = inner.state.read().unwrap();
        if matches!(st.status, Status::Invisible)
            && !st.self_user.is_some_and(|u| u == de.announce.cert.user)
        {
            return;
        }
    }
    let self_device = inner.identity.read().await.device_id();
    if self_device >= peer_device {
        return;
    }
    let port = de.announce.ports.first().copied().unwrap_or(0);
    if port == 0 {
        return;
    }
    let addr = SocketAddr::new(de.from.ip(), port);
    let _ = inner.conns.connect(addr, de.via_multicast).await;
}

/// 静态对端后台拨号：失败按 RETRY_MIN..RETRY_MAX 指数退避，连上后断开再重试。
async fn static_peer_task(inner: Arc<Inner>, addr: SocketAddr) {
    let mut delay = RETRY_MIN;
    loop {
        match inner.conns.connect(addr, false).await {
            Ok(_) => {
                delay = RETRY_MIN;
                tokio::time::sleep(RETRY_MIN).await;
            }
            Err(_) => {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(RETRY_MAX);
            }
        }
    }
}
