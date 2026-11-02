// P1 核心骨架：Service 组装（Task 11）。
// Service 组合身份、存储、发现、连接管理与消息模块；所有客户端能力经 handle() 暴露。
// 启动顺序：Store → Identity → ConnManager::bind → Discovery::spawn → 路由任务 → 静态对端拨号。
// 注释中文，标识符英文。

pub mod handlers;
pub mod ingest;
pub mod views;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::SystemTime;

use tokio::sync::{broadcast, mpsc, watch};

use lanmsg_api::types::Event;
use lanmsg_proto::consts::{PROTO_VERSION, RETRY_MAX, RETRY_MIN, SYNC_JOB_QUEUE};
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId, UserId};
use lanmsg_proto::sign::{sign_with_zeroed_sig, DeviceKey};
use lanmsg_proto::types::{Announce, Signature64, Status};

use crate::discovery::{DiscoveredTable, Discovery, DiscoveryEvent, DiscoveryMode};
use crate::error::CoreError;
use crate::hlc::HlcClock;
use crate::identity::Identity;
use crate::keystore::KeyStore;
use crate::msg::Messaging;
use crate::net::{ConnEvent, ConnManager, HelloInfo, PeerInfo};
use crate::pairing::{Pairing, PairingDeps, WindowSlot};
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
    pub(crate) identity: Arc<StdRwLock<Identity>>,
    pub(crate) keystore: Arc<dyn KeyStore>,
    pub(crate) conns: ConnManager,
    pub(crate) msg: Messaging,
    pub(crate) trust: crate::trust::Trust,
    pub(crate) events: broadcast::Sender<Event>,
    pub(crate) announce_tx: watch::Sender<Option<Announce>>,
    pub(crate) hello_tx: watch::Sender<HelloInfo>,
    pub(crate) local_addr: SocketAddr,
    pub(crate) state: Arc<StdRwLock<NodeState>>,
    pub(crate) discovery: Discovery,
    /// 近期发现的设备表（配对 Device 目标使用）。
    pub(crate) discovered: Arc<DiscoveredTable>,
    /// 「我的设备」配对。
    pub(crate) pairing: Pairing,
    /// 多设备同步、转交与离线补投。
    pub(crate) sync: crate::sync::SyncEngine,
    /// Blob 数据服务端与持有者帧处理（P2 Task 11）。
    pub(crate) blob: Arc<crate::blob::serve::BlobServer>,
    /// 传输引擎：分段并行、多源、断点续传（P2 Task 12）。
    pub(crate) transfer: Arc<crate::blob::transfer::TransferEngine>,
    /// 平台钩子：自动下载策略查询网络状态（P2 Task 13）。
    pub(crate) platform: PlatformHooks,
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

/// 广播会话更新事件（unread 按已读水位重算）；会话不存在时跳过。
pub(crate) fn notify_conversation(inner: &Inner, conv: &ConvId) {
    let Ok(Some(row)) = inner.store.get_conversation(conv) else {
        return;
    };
    let peer_online = match row.kind {
        crate::store::ConvKindRow::Dm => row.peer_user.is_some_and(|pu| {
            inner
                .conns
                .peers()
                .iter()
                .any(|p| p.cert.as_ref().is_some_and(|c| c.user == pu))
        }),
        _ => false,
    };
    let self_user = inner.state.read().unwrap().self_user;
    let summary = views::conv_summary(inner.store.as_ref(), self_user.as_ref(), &row, peer_online);
    let _ = inner.events.send(Event::ConversationUpdated { summary });
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
            platform,
            localsend_port: _localsend_port,
        } = cfg;

        std::fs::create_dir_all(&data_dir)?;
        let store = Arc::new(Store::open(&data_dir.join("lanmsg.db"))?);
        // 每次启动记录数据目录（load_settings 的默认下载目录依赖它）。
        store.set_setting("data_dir", &data_dir.to_string_lossy())?;
        let identity = Arc::new(StdRwLock::new(Identity::load_or_create(
            keystore.as_ref(),
            &store,
        )?));
        let clock = Arc::new(std::sync::Mutex::new(HlcClock::new()));

        let (events_tx, _events_rx) = broadcast::channel(1024);
        let (hello_tx, hello_rx) = watch::channel(HelloInfo {
            display_name: String::new(),
            cert: None,
            listen_port: 0,
        });
        let (announce_tx, announce_rx) = watch::channel(None);

        let self_user = identity.read().unwrap().user_id();
        let state = Arc::new(StdRwLock::new(NodeState {
            status: Status::Online,
            self_user,
        }));

        // 配对窗口槽：accept_filter 与 Pairing 共享（闭包先于 Pairing 构造）。
        let window = WindowSlot::default();

        // accept_filter：Trust 的七条准入规则（status 取当时值；配对窗口经槽查询）。
        // 闭包直接捕获依赖而非 Trust 本身——ConnManager::bind 需要过滤闭包
        // 先于 Trust（依赖 bind 产出的 ConnManager）构造。
        let filter_store = store.clone();
        let filter_identity = identity.clone();
        let filter_state = state.clone();
        let filter_window = window.clone();
        let accept_filter = Arc::new(move |p: &PeerInfo| -> bool {
            let status = filter_state.read().unwrap().status.clone();
            crate::trust::accept_rules(
                &filter_store,
                &filter_identity,
                p,
                status,
                filter_window.open(),
            )
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
            platform.clone(),
        );

        // Blob 数据服务端：注册为数据连接处理器（须在连接涌入前完成注册）。
        let blob = crate::blob::serve::BlobServer::new(store.clone(), identity.clone());
        conns.set_data_handler(blob.clone().handler());

        // 传输引擎（P2 Task 12）：下载、取消与断点续传。
        let transfer = Arc::new(crate::blob::transfer::TransferEngine::new(
            store.clone(),
            conns.clone(),
            identity.clone(),
            events_tx.clone(),
            data_dir.clone(),
        ));

        // Trust：联系人确认、准入过滤、证书更新接收。
        let trust = crate::trust::Trust::new(
            store.clone(),
            identity.clone(),
            clock.clone(),
            conns.clone(),
            events_tx.clone(),
        );

        // 已初始化：立即填充 HelloInfo 与 announce（announce 依赖 bind 后才知道端口）。
        let display = store.get_setting("display_name")?.unwrap_or_default();
        if identity.read().unwrap().is_initialized() {
            let cert = identity.read().unwrap().cert.clone();
            hello_tx.send_replace(HelloInfo {
                display_name: display.clone(),
                cert,
                listen_port: 0,
            });
            if let Some(ann) =
                build_announce(&identity, &display, Status::Online, local_addr.port()).await
            {
                announce_tx.send_replace(Some(ann));
            }
        }

        let (disc_tx, disc_rx) = mpsc::channel::<DiscoveryEvent>(64);
        let self_device = identity.read().unwrap().device_id();
        let discovery = Discovery::spawn(discovery, self_device, announce_rx, disc_tx);

        // 「我的设备」配对（P2 Task 7）。
        let discovered = Arc::new(DiscoveredTable::new());
        let pairing = Pairing::new(PairingDeps {
            store: store.clone(),
            identity: identity.clone(),
            keystore: keystore.clone(),
            conns: conns.clone(),
            msg: msg.clone(),
            trust: trust.clone(),
            events: events_tx.clone(),
            hello_tx: hello_tx.clone(),
            announce_tx: announce_tx.clone(),
            state: state.clone(),
            local_addr,
            window,
        });

        // 多设备同步引擎（P2 Task 9）。
        let sync = crate::sync::SyncEngine::new(store.clone(), identity.clone(), conns.clone());

        let inner = Arc::new(Inner {
            store,
            identity,
            keystore,
            conns,
            msg,
            trust,
            events: events_tx,
            announce_tx,
            hello_tx,
            local_addr,
            state,
            discovery,
            discovered,
            pairing,
            sync,
            blob,
            transfer,
            platform,
        });
        let svc = Service {
            inner: inner.clone(),
        };

        // 注入消息入库回调：Weak 引用避免 Messaging → Inner 强引用环（Inner 的释放
        // 依赖用户侧 Service；升级失败说明服务已关闭，静默跳过即可）。
        let inner_weak = Arc::downgrade(&inner);
        inner.msg.set_ingest_callback(Arc::new(move |from, m| {
            if let Some(i) = inner_weak.upgrade() {
                tokio::spawn(async move {
                    ingest::after_ingest(&i, from, &m).await;
                });
            }
        }));

        // 注入门控回调：SyncPush 经 Trust::gate_inbound 判定陌生人消息去向。
        let gate_trust = inner.trust.clone();
        inner.msg.set_gate_callback(Arc::new(move |p, e, now| {
            gate_trust.gate_inbound(p, e, now)
        }));

        tokio::spawn(routing_task(inner.clone(), conn_rx, disc_rx));
        for addr in static_peers {
            tokio::spawn(static_peer_task(inner.clone(), addr));
        }

        // 恢复遗留的 preparing 行（重启前的文件发送重新走后台任务）。
        inner.msg.resume_preparing().await?;

        // 恢复遗留的活动传输（续传 .part 与已完成分段）。
        inner.transfer.clone().resume_all().await?;

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

    /// 打开到指定设备的数据连接（后续传输引擎与集成测试使用）。
    pub async fn open_data(
        &self,
        device: &lanmsg_proto::ids::DeviceId,
    ) -> Result<crate::net::DataStream, CoreError> {
        self.inner.conns.open_data(device).await
    }

    /// 测试辅助：向指定设备发送控制帧（BlobQuery 等测试用）。
    #[doc(hidden)]
    pub async fn send_frame_for_test(
        &self,
        device: &lanmsg_proto::ids::DeviceId,
        frame: Frame,
    ) -> Result<(), CoreError> {
        self.inner.conns.send(device, frame).await
    }

    /// 测试辅助：访问存储（集成测试断言 holders 等用）。
    #[doc(hidden)]
    pub fn store_for_test(&self) -> Arc<Store> {
        self.inner.store.clone()
    }

    /// 实际监听地址（测试用）。
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr
    }

    /// 关闭发现与所有连接；同时中止后台文件准备任务（preparing 行保留，
    /// 重启后续做）。
    pub async fn shutdown(&self) {
        self.inner.transfer.shutdown_all();
        self.inner.msg.abort_prepares();
        self.inner.discovery.shutdown();
        self.inner.conns.shutdown().await;
    }
}

/// 构造已签名的自身通告；未初始化返回 None。
pub(crate) async fn build_announce(
    identity: &StdRwLock<Identity>,
    display_name: &str,
    status: Status,
    port: u16,
) -> Option<Announce> {
    let id = identity.read().unwrap();
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
    // 每个在线对端一个出站同步任务：Up 时创建、Down 时移除（丢弃发送端后任务
    // 处理完队列即退出；对已断开连接的 send 返回 PeerOffline 也会让其快速结束）。
    // 会长时间等待对端写队列的发送放在这里，避免慢对端阻塞整个事件循环；
    // 同一对端的发送按入队顺序串行，入站摄取仍在本循环内顺序处理。
    let mut sync_workers: HashMap<DeviceId, mpsc::Sender<SyncJob>> = HashMap::new();
    loop {
        tokio::select! {
            evt = conn_rx.recv() => match evt {
                Some(ConnEvent::Up(peer)) => {
                    let (tx, rx) = mpsc::channel(SYNC_JOB_QUEUE);
                    tokio::spawn(sync_worker(inner.sync.clone(), rx));
                    sync_workers.insert(peer.device, tx.clone());
                    on_up(&inner, peer, &tx).await;
                }
                Some(ConnEvent::Down(device)) => {
                    sync_workers.remove(&device);
                    inner.sync.on_peer_down(&device);
                    let _ = inner
                        .events
                        .send(Event::PeerOffline { device: device.to_hex() });
                }
                Some(ConnEvent::Frame(d, f)) => match f {
                    // 信任帧分派给 Trust；其余交给消息模块。
                    Frame::CertUpdate(certs) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            if let Err(e) = inner.trust.on_cert_update(&pi, certs).await {
                                tracing::warn!(peer = %d, error = %e, "CertUpdate 处理失败");
                            }
                        }
                    }
                    Frame::ContactRequest(req) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            if let Err(e) =
                                inner.trust.on_contact_request(&inner.msg, &pi, req).await
                            {
                                tracing::warn!(peer = %d, error = %e, "ContactRequest 处理失败");
                            }
                        }
                    }
                    Frame::ContactResponse(resp) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            if let Err(e) =
                                inner.trust.on_contact_response(&inner.msg, &pi, resp).await
                            {
                                tracing::warn!(peer = %d, error = %e, "ContactResponse 处理失败");
                            }
                        }
                    }
                    // 吊销帧分派给 Trust；本机被吊销时清空 HelloInfo 证书、
                    // 停止通告并清自身用户（之后按未初始化处理）。
                    Frame::Revocation(r) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            let was_self = inner.identity.read().unwrap().device_id() == r.device;
                            match inner.trust.on_revocation(&pi, r.clone()).await {
                                Ok(true) if was_self => {
                                    let mut hello = inner.hello_tx.borrow().clone();
                                    hello.cert = None;
                                    inner.hello_tx.send_replace(hello);
                                    let _ = inner.announce_tx.send(None);
                                    inner.state.write().unwrap().self_user = None;
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::warn!(peer = %d, error = %e, "Revocation 处理失败");
                                }
                            }
                        }
                    }
                    // 配对帧分派给 Pairing（P2 Task 7）。
                    Frame::Pair(p) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            if let Err(e) = inner.pairing.on_pair_frame(&pi, p).await {
                                tracing::warn!(peer = %d, error = %e, "Pair 帧处理失败");
                            }
                        }
                    }
                    // 同步 offer 分派给 SyncEngine（P2 Task 9）。
                    Frame::SyncOffer(offer) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            // 建会话/去重在循环内同步完成，推送交给该对端的后台任务。
                            let conv = offer.conv;
                            match inner.sync.prepare_offer(&pi, offer) {
                                Ok(Some(plan)) => {
                                    let was_reply = plan.is_reply();
                                    let enqueued = match sync_workers.get(&d) {
                                        Some(tx) => {
                                            try_enqueue(tx, SyncJob::Offer(pi, plan), &d)
                                        }
                                        None => false,
                                    };
                                    // 回发作业被丢弃：撤销登记，否则对端下次同会话 offer
                                    // 也不会再回发，同步停滞到重连。被丢的纯推送无登记，
                                    // 由对端下次 offer 或重连恢复，可接受。
                                    if !enqueued && was_reply {
                                        inner.sync.forget_replied(&d, &conv);
                                    }
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    tracing::warn!(peer = %d, error = %e, "SyncOffer 处理失败");
                                }
                            }
                        }
                    }
                    // Blob 持有者查询/应答分派给 BlobServer（P2 Task 11）。
                    Frame::BlobQuery(q) => inner.blob.handle_blob_query(&inner, d, q).await,
                    Frame::BlobHave(h) => inner.blob.handle_blob_have(d, h).await,
                    _ => {
                        let _ = inner.msg.on_frame(d, f).await;
                    }
                },
                None => break,
            },
            evt = disc_rx.recv(), if disc_open => match evt {
                // 发现事件先入设备表（配对 Device 目标使用；未初始化时也记录）。
                Some(de) => {
                    inner.discovered.record(&de);
                    on_discovery(&inner, de).await;
                }
                None => disc_open = false,
            },
        }
    }
}

/// 连接上线：设备与用户入库，发 PeerOnline；未初始化对端（cert == None，
/// 配对流程专用）只记日志，不写库、不发 PeerOnline。
async fn on_up(inner: &Inner, peer: PeerInfo, sync_tx: &mpsc::Sender<SyncJob>) {
    let Some(cert) = &peer.cert else {
        tracing::debug!(peer = %peer.device, "未初始化对端连接（配对流程专用）");
        return;
    };
    let now = now_millis();
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
    let self_user = inner.state.read().unwrap().self_user;
    let _ = inner.events.send(Event::PeerOnline {
        peer: views::peer_view(&inner.store, &peer, self_user.as_ref()),
    });

    // 多设备同步：对端上线即互发证书与 SyncOffer（陌生人同样调用，以投递
    // 离线消息）；发送在该对端的后台任务中进行。
    let device = peer.device;
    try_enqueue(sync_tx, SyncJob::Up(peer), &device);
}

/// 非阻塞入队：队列满（对端不读我方数据）则丢弃并告警。丢弃安全：
/// SyncOffer 可由对端或下次上线重发。返回是否入队成功。
fn try_enqueue<T>(tx: &mpsc::Sender<T>, job: T, peer: &DeviceId) -> bool {
    match tx.try_send(job) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(peer = %peer, error = %e, "同步作业队列已满或已关闭，丢弃作业");
            false
        }
    }
}

/// 交给对端后台同步任务的作业。
enum SyncJob {
    Up(PeerInfo),
    Offer(PeerInfo, crate::sync::OfferPlan),
}

/// 单个对端的出站同步任务：按入队顺序串行执行，发送端被丢弃后退出。
async fn sync_worker(sync: crate::sync::SyncEngine, mut rx: mpsc::Receiver<SyncJob>) {
    while let Some(job) = rx.recv().await {
        match job {
            SyncJob::Up(peer) => {
                if let Err(e) = sync.on_peer_up(&peer).await {
                    tracing::warn!(peer = %peer.device, error = %e, "SyncOffer 生成失败");
                }
            }
            SyncJob::Offer(peer, plan) => {
                if let Err(e) = sync.run_offer(&peer, plan).await {
                    tracing::warn!(peer = %peer.device, error = %e, "SyncOffer 处理失败");
                }
            }
        }
    }
}

/// 发现事件：未连接且自身设备 ID 较小时出站拨号；Invisible 不主动外联其他用户。
async fn on_discovery(inner: &Inner, de: DiscoveryEvent) {
    let peer_device = de.announce.cert.device;
    if inner.conns.is_connected(&peer_device) {
        return;
    }
    {
        let st = inner.state.read().unwrap();
        // 未初始化的 Service 不因发现而自动拨号（配对经 PairJoin 显式连接）。
        if st.self_user.is_none() {
            return;
        }
        if matches!(st.status, Status::Invisible)
            && !st.self_user.is_some_and(|u| u == de.announce.cert.user)
        {
            return;
        }
    }
    let self_device = inner.identity.read().unwrap().device_id();
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

#[cfg(test)]
mod queue_tests {
    use super::*;

    /// 对端不读数据（接收端不消费）时，洪泛入队最多接受 SYNC_JOB_QUEUE 个，
    /// 其余被丢弃，队列长度有上限。选确定性单测而非网络洪泛：入队决策是
    /// 唯一的放大点，直接断言它比依赖时序的集成测试稳定。
    #[test]
    fn flood_enqueue_is_bounded() {
        let (tx, mut rx) = mpsc::channel::<u32>(SYNC_JOB_QUEUE);
        let peer = DeviceId([7u8; 32]);
        let accepted = (0..(SYNC_JOB_QUEUE * 4) as u32)
            .filter(|i| try_enqueue(&tx, *i, &peer))
            .count();
        assert_eq!(accepted, SYNC_JOB_QUEUE);
        let mut queued = 0;
        while rx.try_recv().is_ok() {
            queued += 1;
        }
        assert_eq!(queued, SYNC_JOB_QUEUE);
    }
}
