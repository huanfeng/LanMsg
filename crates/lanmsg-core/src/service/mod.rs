// P1 核心骨架：Service 组装（Task 11）。
// Service 组合身份、存储、发现、连接管理与消息模块；所有客户端能力经 handle() 暴露。
// 启动顺序：Store → Identity → ConnManager::bind → Discovery::spawn → 路由任务 → 静态对端拨号。
// 注释中文，标识符英文。

pub mod files;
pub mod handlers;
pub mod ingest;
pub(crate) mod manual;
pub(crate) mod redial;
pub mod views;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};
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
    /// LocalSend 开关：非 0 表示生产模式（端口取设置项 `localsend_port`，
    /// 且在组播发现开启时启用 LocalSend 发现）；0 表示测试模式（随机端口、无发现）。
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
    /// 文件中转站主机端（P5 Task 5）。
    pub(crate) station: Arc<crate::station::StationHost>,
    /// 网页传输宿主（P5 Task 14）。
    pub(crate) web: Arc<crate::web::WebHost>,
    /// 中转站同步的拉取端状态与请求等待表。
    pub(crate) station_client: crate::station::client::StationClient,
    /// 数据目录（中转站副本目录等）。
    pub(crate) data_dir: std::path::PathBuf,
    /// 混合逻辑时钟（LocalSend 桥为伪设备作者的消息取时间戳）。
    pub(crate) clock: Arc<StdMutex<HlcClock>>,
    /// LocalSend 兼容服务（P5 Task 11）。
    pub(crate) localsend: crate::localsend_bridge::LsSlot,
    /// 平台钩子：自动下载策略查询网络状态（P2 Task 13）。
    pub(crate) platform: PlatformHooks,
    /// 界面当前正在看的会话（SetUiFocus 设置，只存内存不落库）。
    pub(crate) ui_focus: StdMutex<Option<ConvId>>,
    /// 已登记的静态对端地址（启动配置 + AddManualPeer；只存内存，每个地址一个重连任务）。
    pub(crate) static_peers: StdMutex<HashSet<SocketAddr>>,
    /// 手动添加的对端后台任务（键为规范化的 host:port），可按键停止。
    pub(crate) manual_tasks: StdMutex<HashMap<String, tokio::task::AbortHandle>>,
    /// 最近一次发给各对端的地址簿指纹：内容未变则不重复推送。
    address_book_sent: StdMutex<HashMap<DeviceId, u64>>,
    /// 网段扫描进行中标志：同一时刻只允许一个扫描。
    pub(crate) scanning: std::sync::atomic::AtomicBool,
    /// 网段扫描探测的端口（默认服务端口；测试钩子可覆盖）。
    pub(crate) scan_port: std::sync::atomic::AtomicU16,
    /// 发现触发的在途拨号地址：同一地址拨号未结束时不重复发起。
    dialing: StdMutex<HashSet<SocketAddr>>,
    /// 受信对端的重连状态（任务登记、唤醒信号、尝试计数）。
    pub(crate) redial: redial::RedialState,
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
            localsend_port,
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
        let ls_multicast = matches!(discovery, DiscoveryMode::Multicast);
        let discovery = Discovery::spawn(discovery, self_device, announce_rx, disc_tx);
        // 用户配置的单播通告目标（解析失败的配置项在保存时已拒绝，这里兜底忽略）。
        match crate::settings::load_settings(&store) {
            Ok(st) => match crate::discovery::unicast::parse_unicast_targets(&st.unicast_targets) {
                Ok(targets) => discovery.set_unicast_targets(targets),
                Err(e) => tracing::warn!(error = %e, "ignoring invalid stored unicast targets"),
            },
            Err(e) => tracing::warn!(error = %e, "loading settings for unicast targets failed"),
        }

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

        // LocalSend 固定端口与发现仅在生产模式启用：端口非 0 且使用组播发现。
        let ls_production = localsend_port != 0 && ls_multicast;
        let station = Arc::new(crate::station::StationHost::new(
            store.clone(),
            identity.clone(),
            data_dir.clone(),
            platform.thumbnailer.clone(),
        ));

        let web = Arc::new(crate::web::WebHost::new(crate::web::WebDeps {
            store: store.clone(),
            identity: identity.clone(),
            conns: conns.clone(),
            events: events_tx.clone(),
            clock: clock.clone(),
            msg: msg.clone(),
            thumbnailer: platform.thumbnailer.clone(),
        }));

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
            station,
            web,
            station_client: crate::station::client::StationClient::new(),
            data_dir: data_dir.clone(),
            clock,
            localsend: crate::localsend_bridge::LsSlot::new(ls_production),
            platform,
            ui_focus: StdMutex::new(None),
            static_peers: StdMutex::new(HashSet::new()),
            manual_tasks: StdMutex::new(HashMap::new()),
            address_book_sent: StdMutex::new(HashMap::new()),
            scanning: std::sync::atomic::AtomicBool::new(false),
            scan_port: std::sync::atomic::AtomicU16::new(lanmsg_proto::consts::SERVICE_PORT),
            dialing: StdMutex::new(HashSet::new()),
            redial: redial::RedialState::new(),
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
        tokio::spawn(address_book_task(inner.clone()));
        // 订阅须在 spawn 之前完成，否则任务启动前的变化会丢失。
        let station_changes = inner.station.subscribe_changes();
        tokio::spawn(station_task(inner.clone(), station_changes));
        // 重启后：为已知地址的受信设备恢复重连任务。
        redial::resume_all(&inner);
        for addr in static_peers {
            register_static_peer(&inner, addr, false);
        }
        // 重启后恢复手动添加的对端（持久化在 manual_peers）。
        match inner.store.list_manual_peers() {
            Ok(list) => {
                // 主机名在任务内重新解析。
                for (key, _) in list {
                    manual::start(&inner, key, false);
                }
            }
            Err(e) => tracing::warn!("loading manual peers failed: {e}"),
        }

        // 旧库的文件 / 文字搜索索引回填：后台执行，不阻塞启动（完成后写标记，只做一次）。
        let backfill_store = inner.store.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = backfill_store.backfill_search_index() {
                tracing::warn!("search index backfill failed: {e}");
            }
        });

        // 恢复遗留的 preparing 行（重启前的文件发送重新走后台任务）。
        inner.msg.resume_preparing().await?;

        // 恢复遗留的活动传输（续传 .part 与已完成分段）。
        inner.transfer.clone().resume_all().await?;

        // 已初始化且开启 LocalSend 兼容时启动。配对加入、其他设备改昵称等路径不经过
        // 本机的请求入口，因此另起低频任务让桥跟上设置与身份的变化。
        inner.localsend.reconfigure(&inner).await;
        let weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(i) = weak.upgrade() else { break };
                i.localsend.reconfigure(&i).await;
            }
        });

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

    /// 测试辅助：不登记静态对端地直接拨号（避免静态对端重连掩盖受信重连）。
    #[doc(hidden)]
    pub async fn connect_for_test(&self, addr: SocketAddr) -> Result<DeviceId, CoreError> {
        self.inner.conns.connect(addr, false).await
    }

    /// 测试辅助：本端主动断开与某设备的连接。
    #[doc(hidden)]
    pub async fn disconnect_for_test(&self, device: &DeviceId) {
        self.inner.conns.disconnect(device).await
    }

    /// 测试辅助：受信重连累计发起的拨号次数。
    #[doc(hidden)]
    pub fn redial_attempts_for_test(&self) -> u64 {
        self.inner.redial.attempts()
    }

    /// 测试辅助：让所有地址都被重连任务视为不在本机网段（进程级）。
    #[doc(hidden)]
    #[cfg(feature = "test-hooks")]
    pub fn force_off_subnet_for_test(on: bool) {
        redial::force_off_subnet_for_tests(on);
    }

    /// 测试辅助：覆盖本端看到的某对端 dial_addr（模拟对端在 NAT 后，直拨不通）。
    #[doc(hidden)]
    #[cfg(feature = "test-hooks")]
    pub fn override_dial_addr_for_test(&self, device: DeviceId, addr: SocketAddr) {
        self.inner.conns.override_dial_addr_for_tests(device, addr);
    }

    /// 测试辅助：覆盖网段扫描探测的端口（默认服务端口）。
    #[doc(hidden)]
    #[cfg(feature = "test-hooks")]
    pub fn override_scan_port_for_test(&self, port: u16) {
        self.inner
            .scan_port
            .store(port, std::sync::atomic::Ordering::SeqCst);
    }

    /// 测试辅助：缩短网页传输会话的空闲超时。
    #[doc(hidden)]
    #[cfg(feature = "test-hooks")]
    pub fn set_web_idle_timeout_for_test(&self, d: std::time::Duration) {
        self.inner.web.set_idle_timeout(d);
    }

    /// 测试辅助：让网页传输把所有来源当作非局域网。
    #[doc(hidden)]
    #[cfg(feature = "test-hooks")]
    pub fn set_web_force_non_lan_for_test(&self, on: bool) {
        self.inner.web.set_force_non_lan(on);
    }

    /// 测试辅助：重连任务循环迭代总数（检测空转）。
    #[doc(hidden)]
    pub fn redial_loop_iterations_for_test(&self) -> u64 {
        self.inner.redial.loop_iterations()
    }

    /// 测试辅助：去抖后已触发的网络变化次数。
    #[doc(hidden)]
    pub fn redial_net_signals_for_test(&self) -> u64 {
        self.inner.redial.net_signals()
    }

    /// 测试辅助：要求对端为 `expect` 的拨号。
    #[doc(hidden)]
    pub async fn connect_expecting_for_test(
        &self,
        addr: SocketAddr,
        expect: DeviceId,
    ) -> Result<DeviceId, CoreError> {
        self.inner.conns.connect_expecting(addr, Some(expect)).await
    }

    /// 测试辅助：LocalSend 服务的实际监听端口（未运行为 None）。
    #[doc(hidden)]
    pub fn localsend_port_for_test(&self) -> Option<u16> {
        self.inner.localsend.get().map(|b| b.port())
    }

    /// 测试辅助：访问存储（集成测试断言 holders 等用）。
    #[doc(hidden)]
    pub fn store_for_test(&self) -> Arc<Store> {
        self.inner.store.clone()
    }

    /// 界面当前正在看的会话（SetUiFocus 设置的值），供 lanmsgd 判断是否弹通知。
    pub fn ui_focus(&self) -> Option<ConvId> {
        // 锁中毒时取内部值：该值只是 Copy 的 Option，不存在半更新状态。
        *self
            .inner
            .ui_focus
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// 配对模块与连接管理（仅供 crate 内单测注入帧）。
    #[cfg(test)]
    pub(crate) fn pairing_parts(&self) -> (Pairing, ConnManager) {
        (self.inner.pairing.clone(), self.inner.conns.clone())
    }

    /// 实际监听地址（测试用）。
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr
    }

    /// 关闭发现与所有连接；同时中止后台文件准备任务（preparing 行保留，
    /// 重启后续做）。
    pub async fn shutdown(&self) {
        self.inner.localsend.stop().await;
        self.inner.web.stop().await;
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
    let mut book_limiter = AddressBookLimiter::default();
    loop {
        tokio::select! {
            evt = conn_rx.recv() => match evt {
                Some(ConnEvent::Up(peer)) => {
                    let (tx, rx) = mpsc::channel(SYNC_JOB_QUEUE);
                    tokio::spawn(sync_worker(inner.clone(), rx));
                    sync_workers.insert(peer.device, tx.clone());
                    let up_peer = peer.clone();
                    on_up(&inner, peer, &tx).await;
                    // 自己的设备上线：通告本机托管的中转站（后台发送，不阻塞路由）。
                    let announce_inner = inner.clone();
                    tokio::spawn(async move {
                        crate::station::client::on_peer_up(&announce_inner, &up_peer).await;
                    });
                }
                Some(ConnEvent::Down(device)) => {
                    sync_workers.remove(&device);
                    book_limiter.forget(&device);
                    inner.address_book_sent.lock().unwrap().remove(&device);
                    inner.conns.forget_dial_back(&device);
                    inner.sync.on_peer_down(&device);
                    // 关系可能在连接期间才变成受信（刚加联系人），断开时补建重连任务。
                    redial::ensure_task(&inner, device);
                    redial::wake(&inner, &device);
                    let _ = inner
                        .events
                        .send(Event::PeerOffline { device: device.to_hex() });
                }
                Some(ConnEvent::Frame(d, f)) => match f {
                    // 信任帧分派给 Trust；其余交给消息模块。
                    Frame::CertUpdate(certs) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            if let Err(e) = inner.trust.on_cert_update(&pi, certs).await {
                                tracing::warn!(peer = %d, error = %e, "CertUpdate handling failed");
                            }
                        } else {
                            tracing::debug!(peer = %d, "connection no longer registered when frame arrived, dropped");
                        }
                    }
                    Frame::ContactRequest(req) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            if let Err(e) =
                                inner.trust.on_contact_request(&inner.msg, &pi, req).await
                            {
                                tracing::warn!(peer = %d, error = %e, "ContactRequest handling failed");
                            }
                        } else {
                            tracing::debug!(peer = %d, "connection no longer registered when frame arrived, dropped");
                        }
                    }
                    Frame::ContactResponse(resp) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            if let Err(e) =
                                inner.trust.on_contact_response(&inner.msg, &pi, resp).await
                            {
                                tracing::warn!(peer = %d, error = %e, "ContactResponse handling failed");
                            }
                        } else {
                            tracing::debug!(peer = %d, "connection no longer registered when frame arrived, dropped");
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
                                    tracing::warn!(peer = %d, error = %e, "Revocation handling failed");
                                }
                            }
                        } else {
                            tracing::debug!(peer = %d, "connection no longer registered when frame arrived, dropped");
                        }
                    }
                    // 配对帧分派给 Pairing（P2 Task 7）。
                    Frame::Pair(p) => {
                        if let Some(pi) = inner.conns.peer(&d) {
                            if let Err(e) = inner.pairing.on_pair_frame(&pi, p).await {
                                tracing::warn!(peer = %d, error = %e, "Pair frame handling failed");
                            }
                        } else {
                            tracing::debug!(peer = %d, "connection no longer registered when frame arrived, dropped");
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
                                    tracing::warn!(peer = %d, error = %e, "SyncOffer handling failed");
                                }
                            }
                        } else {
                            tracing::debug!(peer = %d, "connection no longer registered when frame arrived, dropped");
                        }
                    }
                    // 对端直拨不通，请求我们向它拨回一条数据连接（NAT 场景）。
                    Frame::DataDialBack { token } => {
                        let inner = inner.clone();
                        tokio::spawn(async move {
                            if let Err(e) = inner.conns.serve_dial_back(&d, token).await {
                                tracing::warn!(peer = %d, error = %e, "serving dial-back request failed");
                            }
                        });
                    }
                    // 地址簿：限速后放到阻塞线程合并（批量写库），再唤醒相关设备的重连任务。
                    Frame::AddressBook(book) => {
                        if !book_limiter.allow(d, std::time::Instant::now()) {
                            tracing::debug!(peer = %d, "AddressBook rate limited, dropped");
                        } else if let Some(pi) = inner.conns.peer(&d) {
                            let inner = inner.clone();
                            tokio::spawn(async move {
                                let self_user = inner.state.read().unwrap().self_user;
                                let store = inner.store.clone();
                                let merged = tokio::task::spawn_blocking(move || {
                                    crate::net::addrbook::merge_devices(
                                        &store,
                                        self_user,
                                        &pi,
                                        &book,
                                        now_millis(),
                                    )
                                })
                                .await;
                                match merged {
                                    Ok(Ok(devices)) => {
                                        for dev in devices {
                                            redial::addresses_changed(&inner, dev);
                                        }
                                    }
                                    Ok(Err(e)) => {
                                        tracing::warn!(peer = %d, error = %e, "AddressBook merge failed");
                                    }
                                    Err(e) => {
                                        tracing::warn!(peer = %d, error = %e, "AddressBook merge task failed");
                                    }
                                }
                            });
                        }
                    }
                    // Blob 持有者查询/应答分派给 BlobServer（P2 Task 11）。
                    Frame::BlobQuery(q) => inner.blob.handle_blob_query(&inner, d, q).await,
                    Frame::BlobHave(h) => inner.blob.handle_blob_have(d, h).await,
                    // 中转站同步帧：处理可能涉及写库与发送，放到后台任务。
                    Frame::Station(m) => {
                        tokio::spawn(crate::station::client::on_message(inner.clone(), d, m));
                    }
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
async fn on_up(inner: &Arc<Inner>, peer: PeerInfo, sync_tx: &mpsc::Sender<SyncJob>) {
    let Some(cert) = &peer.cert else {
        tracing::debug!(peer = %peer.device, "uninitialized peer connection (pairing only)");
        return;
    };
    // 兜底：准入检查之后到上线之前对端可能被吊销；查询出错一律按已吊销处理。
    if inner.store.is_revoked(&peer.device).unwrap_or(true) {
        tracing::info!(peer = %peer.device, "revoked device connected, disconnecting");
        inner.conns.disconnect(&peer.device).await;
        return;
    }
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
    if let Err(e) = crate::net::addrbook::on_peer_up_record(&inner.store, &peer, now) {
        tracing::warn!(peer = %peer.device, error = %e, "recording peer address failed");
    }
    redial::ensure_task(inner, peer.device);
    let self_user = inner.state.read().unwrap().self_user;
    push_address_books(inner, Some(peer.device));
    let _ = inner.events.send(Event::PeerOnline {
        peer: views::peer_view(&inner.store, &peer, self_user.as_ref()),
    });

    // 多设备同步：对端上线即互发证书与 SyncOffer（陌生人同样调用，以投递
    // 离线消息）；发送在该对端的后台任务中进行。
    let device = peer.device;
    try_enqueue(sync_tx, SyncJob::Up(peer.clone()), &device);
    // 补发 BlobHave：对方作者在我下载时可能不在线（回报丢失），重新连上后补报，
    // 作者据此判定“已送达”。排在 SyncOffer 之后，同一连接只发这一次。
    try_enqueue(sync_tx, SyncJob::HaveBlobs(peer), &device);
}

/// 同一对端的地址簿限速：滑动窗口内最多处理 [`BOOK_LIMIT_COUNT`] 个，其余丢弃。
#[derive(Default)]
struct AddressBookLimiter {
    seen: HashMap<DeviceId, std::collections::VecDeque<std::time::Instant>>,
}

impl AddressBookLimiter {
    fn allow(&mut self, device: DeviceId, now: std::time::Instant) -> bool {
        let q = self.seen.entry(device).or_default();
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) >= BOOK_LIMIT_WINDOW)
        {
            q.pop_front();
        }
        if q.len() >= BOOK_LIMIT_COUNT {
            return false;
        }
        q.push_back(now);
        true
    }

    fn forget(&mut self, device: &DeviceId) {
        self.seen.remove(device);
    }
}

/// 地址簿限速窗口与窗口内最大处理个数。
const BOOK_LIMIT_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);
const BOOK_LIMIT_COUNT: usize = 5;
/// 发送地址簿的超时：对端不读数据时不无限占用任务。
const BOOK_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 向所有在线的自己设备与已接受联系人发送各自可见的地址簿（空簿不发）。
/// 触发时机：对端上线、联系人变化后、定时兜底——新设备记录与新地址到达的先后顺序不固定。
fn push_address_books(inner: &Arc<Inner>, fresh: Option<DeviceId>) {
    let Some(self_user) = inner.state.read().unwrap().self_user else {
        return;
    };
    // 新连接上线：它没收过任何簿，清掉指纹强制重发。
    if let Some(d) = fresh {
        inner.address_book_sent.lock().unwrap().remove(&d);
    }
    for p in inner.conns.peers() {
        let Some(peer_user) = p.cert.as_ref().map(|c| c.user) else {
            continue;
        };
        let book = match crate::net::addrbook::build_for(&inner.store, &self_user, &peer_user) {
            Ok(b) if !b.entries.is_empty() => b,
            _ => continue,
        };
        // 内容未变则跳过。
        let fingerprint = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            lanmsg_proto::to_cbor(&book).hash(&mut h);
            h.finish()
        };
        {
            let mut sent = inner.address_book_sent.lock().unwrap();
            if sent.get(&p.device) == Some(&fingerprint) {
                continue;
            }
            sent.insert(p.device, fingerprint);
        }
        let inner = inner.clone();
        let device = p.device;
        // 发送可能等待写队列，放到后台任务里并限时，不阻塞调用方。
        tokio::spawn(async move {
            let sent = tokio::time::timeout(
                BOOK_SEND_TIMEOUT,
                inner.conns.send(&device, Frame::AddressBook(book)),
            )
            .await;
            if !matches!(sent, Ok(Ok(()))) {
                // 没发成：清掉指纹，下次推送重试。
                inner.address_book_sent.lock().unwrap().remove(&device);
            }
        });
    }
}

/// 地址簿兜底推送任务：联系人变化后（稍等对端同步完成）与定时各推一次。
async fn address_book_task(inner: Arc<Inner>) {
    let mut events = inner.events.subscribe();
    let mut tick = tokio::time::interval(ADDRESS_BOOK_INTERVAL);
    tick.tick().await;
    loop {
        tokio::select! {
            _ = tick.tick() => push_address_books(&inner, None),
            ev = events.recv() => match ev {
                Ok(Event::ContactsChanged) => {
                    let inner = inner.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(ADDRESS_BOOK_DELAY).await;
                        push_address_books(&inner, None);
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                _ => {}
            },
            _ = inner.conns.wait_shutdown() => break,
        }
    }
}

/// 中转站后台任务：每 60 秒巡检过期与引用文件可用性，并把条目变化转成界面事件。
async fn station_task(
    inner: Arc<Inner>,
    mut changes: tokio::sync::broadcast::Receiver<(
        ConvId,
        Vec<lanmsg_proto::station::StationItem>,
    )>,
) {
    let mut tick = tokio::time::interval(STATION_SWEEP_INTERVAL);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let host = inner.station.clone();
                match tokio::task::spawn_blocking(move || host.sweep(now_millis())).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => tracing::warn!(error = %e, "station sweep failed"),
                    Err(e) => tracing::warn!(error = %e, "station sweep task failed"),
                }
                crate::station::client::retry_pulls(&inner);
            }
            ch = changes.recv() => match ch {
                Ok((station, items)) => {
                    let _ = inner.events.send(Event::StationUpdated { station: station.to_hex() });
                    // 推给在线的自己设备（后台发送，慢对端不阻塞本任务）。
                    let push_inner = inner.clone();
                    tokio::spawn(async move {
                        crate::station::client::push_delta(&push_inner, station, items).await;
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                // 落后时丢了若干通知：界面刷新一次即可，无需逐条补发。
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            },
            _ = inner.conns.wait_shutdown() => break,
        }
    }
}

/// 中转站巡检间隔。
const STATION_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// 地址簿定时推送间隔。
const ADDRESS_BOOK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// 联系人变化后延迟多久再推，留给对端同步联系人与设备记录的时间。
const ADDRESS_BOOK_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// 非阻塞入队：队列满（对端不读我方数据）则丢弃并告警。丢弃安全：
/// SyncOffer 可由对端或下次上线重发。返回是否入队成功。
fn try_enqueue<T>(tx: &mpsc::Sender<T>, job: T, peer: &DeviceId) -> bool {
    match tx.try_send(job) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(peer = %peer, error = %e, "sync job queue full or closed, job dropped");
            false
        }
    }
}

/// 交给对端后台同步任务的作业。
enum SyncJob {
    Up(PeerInfo),
    /// 向对端补发“本机已持有、且由其用户发出的消息引用”的 BlobHave。
    HaveBlobs(PeerInfo),
    Offer(PeerInfo, crate::sync::OfferPlan),
}

/// 单个对端的出站同步任务：按入队顺序串行执行，发送端被丢弃后退出。
async fn sync_worker(inner: Arc<Inner>, mut rx: mpsc::Receiver<SyncJob>) {
    let sync = inner.sync.clone();
    while let Some(job) = rx.recv().await {
        match job {
            SyncJob::HaveBlobs(peer) => push_blob_have(&inner, &peer).await,
            SyncJob::Up(peer) => {
                if let Err(e) = sync.on_peer_up(&peer).await {
                    tracing::warn!(peer = %peer.device, error = %e, "SyncOffer generation failed");
                }
            }
            SyncJob::Offer(peer, plan) => {
                if let Err(e) = sync.run_offer(&peer, plan).await {
                    tracing::warn!(peer = %peer.device, error = %e, "SyncOffer handling failed");
                }
            }
        }
    }
}

/// 每个 BlobHave 帧最多携带的哈希数（每个约 40 字节，远小于帧上限）。
const BLOB_HAVE_BATCH: usize = 256;

/// 向对端补发 BlobHave：本机已持有、且被该对端用户的设备发出的消息引用的哈希，分批发送。
/// 对端只接受受信设备的声明，对陌生人发了也会被忽略，所以只对已有证书的对端发。
async fn push_blob_have(inner: &Arc<Inner>, peer: &PeerInfo) {
    let Some(cert) = &peer.cert else {
        return;
    };
    let hashes = match inner.store.held_blobs_authored_by_user(&cert.user) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(peer = %peer.device, error = %e, "failed to list held blobs for BlobHave push");
            return;
        }
    };
    for chunk in hashes.chunks(BLOB_HAVE_BATCH) {
        let frame = Frame::BlobHave(lanmsg_proto::types::BlobHave {
            hashes: chunk.to_vec(),
        });
        if inner.conns.send(&peer.device, frame).await.is_err() {
            return;
        }
    }
}

/// 发现事件：未连接且自身设备 ID 较小时出站拨号；Invisible 不主动外联其他用户。
async fn on_discovery(inner: &Arc<Inner>, de: DiscoveryEvent) {
    let peer_device = de.announce.cert.device;
    // 连接表里有但已长时间没收包的连接可能已死（对端无 FIN 消失后重启），仍要拨号，
    // 由连接管理器的裁决用新连接替换。
    if inner.conns.is_connected_alive(&peer_device) {
        return;
    }
    // 受信设备又出现了：若其重连任务正在退避，让它马上重试。
    redial::kick_device(inner, &peer_device);
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
    // 拨号放后台：对端沉默时最长等 DIAL_TIMEOUT，不能冻结路由循环；同地址在途则不重复发起。
    if !inner.dialing.lock().unwrap().insert(addr) {
        return;
    }
    let guard = DialingGuard {
        inner: inner.clone(),
        addr,
    };
    let inner = inner.clone();
    let via_multicast = de.via_multicast;
    tokio::spawn(async move {
        // guard 随任务结束（含被取消/panic）Drop，必定从在途表移除。
        let _guard = guard;
        // 关停时打断拨号，不让后台任务拖住退出。
        tokio::select! {
            _ = inner.conns.connect(addr, via_multicast) => {}
            _ = inner.conns.wait_shutdown() => {}
        }
    });
}

/// 发现触发拨号的在途登记：Drop 时移除，保证任何退出路径都不泄漏地址。
pub(crate) struct DialingGuard {
    pub(crate) inner: Arc<Inner>,
    pub(crate) addr: SocketAddr,
}

impl Drop for DialingGuard {
    fn drop(&mut self) {
        self.inner
            .dialing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.addr);
    }
}

/// 登记静态对端并启动其后台重连任务；同一地址只启动一次。
///
/// `connected`：调用方刚连上该地址，任务先等一个周期再拨，避免与刚建立的连接并发重复拨号。
pub(crate) fn register_static_peer(inner: &Arc<Inner>, addr: SocketAddr, connected: bool) {
    if inner.static_peers.lock().unwrap().insert(addr) {
        tokio::spawn(static_peer_task(inner.clone(), addr, connected));
    }
}

/// 静态对端后台拨号：失败按 RETRY_MIN..RETRY_MAX 指数退避，连上后断开再重试。
/// 退避睡眠与进行中的拨号都会被关停信号打断。
async fn static_peer_task(inner: Arc<Inner>, addr: SocketAddr, connected: bool) {
    let mut delay = RETRY_MIN;
    if connected && sleep_or_shutdown(&inner, RETRY_MIN).await {
        return;
    }
    while !inner.conns.is_shut_down() {
        let result = tokio::select! {
            r = inner.conns.connect(addr, false) => r,
            _ = inner.conns.wait_shutdown() => return,
        };
        match result {
            Ok(_) => {
                delay = RETRY_MIN;
                if sleep_or_shutdown(&inner, RETRY_MIN).await {
                    return;
                }
            }
            Err(_) => {
                if sleep_or_shutdown(&inner, delay).await {
                    return;
                }
                delay = (delay * 2).min(RETRY_MAX);
            }
        }
    }
}

/// 睡眠 `d`；期间收到关停信号则提前返回 true。
pub(crate) async fn sleep_or_shutdown(inner: &Inner, d: std::time::Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(d) => false,
        _ = inner.conns.wait_shutdown() => true,
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    #[test]
    fn address_book_limiter_allows_burst_then_drops() {
        let mut l = AddressBookLimiter::default();
        let d = DeviceId([1u8; 32]);
        let t0 = std::time::Instant::now();
        for _ in 0..BOOK_LIMIT_COUNT {
            assert!(l.allow(d, t0));
        }
        assert!(!l.allow(d, t0));
        // 其他对端不受影响；窗口过后恢复。
        assert!(l.allow(DeviceId([2u8; 32]), t0));
        assert!(l.allow(d, t0 + BOOK_LIMIT_WINDOW));
    }

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
