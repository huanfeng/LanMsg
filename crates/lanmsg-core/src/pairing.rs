// P2 Task 7：「我的设备」配对——扫码 HMAC 证明与 6 位数字 SAS 比对。
// PairWindow/pair_uri/parse_pair_uri/pair_mac/sas_code 与 Pairing 状态机：
// A（已初始化方）开窗口收 Begin/Proof/SasConfirm/Done；B（加入方）join 后
// 发 Begin（+Qr 的 Proof），收 Proof/Bundle 后采纳身份。所有失败路径在
// B 第 5 步之前不写任何 store/keystore。注释中文，标识符英文。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use blake3;
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use tokio::sync::{broadcast, watch, RwLock};

use lanmsg_api::types::{Event, PairSessionInfo, PairStateDto, PairTarget};
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId};
use lanmsg_proto::sign::UserKey;
use lanmsg_proto::types::{
    Announce, DeviceCert, DeviceType, MsgBody, PairBundle, PairMethodWire, PairMsg, SystemEvent,
};

use crate::error::CoreError;
use crate::identity::Identity;
use crate::keystore::KeyStore;
use crate::msg::Messaging;
use crate::net::{ConnManager, HelloInfo, PeerInfo};
use crate::settings::SETTING_DEVICE_NAME;
use crate::store::{ContactState, ConvKindRow, ConvRow, DeviceRow, Store, UserRow};

/// 配对 URI 前缀。
const URI_PREFIX: &str = "lanmsg://pair?";
/// Qr 证明 role：B 证明自己。
const ROLE_B: &[u8] = b"lanmsg-pair-b";
/// Qr 证明 role：A 证明自己。
const ROLE_A: &[u8] = b"lanmsg-pair-a";
/// SAS 数字派生前缀。
const SAS_PREFIX: &[u8] = b"lanmsg-sas";

type HmacSha256 = Hmac<Sha256>;

/// 配对帧类型名（日志用）。
fn pair_kind(msg: &PairMsg) -> &'static str {
    match msg {
        PairMsg::Begin { .. } => "Begin",
        PairMsg::Proof { .. } => "Proof",
        PairMsg::SasConfirm { .. } => "SasConfirm",
        PairMsg::Bundle(_) => "Bundle",
        PairMsg::Done => "Done",
        PairMsg::Abort { .. } => "Abort",
    }
}

// ===== 纯函数与数据结构 =====

/// 配对窗口：start 打开后在 ttl 内有效。
#[derive(Debug, Clone)]
pub struct PairWindow {
    pub session: String,
    pub method: PairMethodWire,
    pub secret: Option<[u8; 16]>,
    pub opened_at: Instant,
    pub ttl: Duration,
}

impl PairWindow {
    /// 窗口是否仍开放。
    pub fn is_open(&self, now: Instant) -> bool {
        now.duration_since(self.opened_at) < self.ttl
    }
}

/// 构造配对二维码 URI：
/// `lanmsg://pair?d=<hex64>&a=<ip:port>[,…]&p=<session>&s=<BASE32 大写>`。
pub fn pair_uri(
    device: &DeviceId,
    addrs: &[SocketAddr],
    session: &str,
    secret: &[u8; 16],
) -> String {
    let addrs = addrs
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{URI_PREFIX}d={}&a={addrs}&p={session}&s={}",
        device.to_hex(),
        BASE32_NOPAD.encode(secret)
    )
}

/// 解析后的配对 URI。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPairUri {
    pub device: DeviceId,
    pub addrs: Vec<SocketAddr>,
    pub session: String,
    pub secret: [u8; 16],
}

/// 解析配对二维码 URI。
pub fn parse_pair_uri(uri: &str) -> Result<ParsedPairUri, CoreError> {
    let query = uri
        .strip_prefix(URI_PREFIX)
        .ok_or_else(|| CoreError::InvalidArgument("非法配对 URI".into()))?;
    let mut d = None;
    let mut a = None;
    let mut p = None;
    let mut s = None;
    for item in query.split('&') {
        let (k, v) = item
            .split_once('=')
            .ok_or_else(|| CoreError::InvalidArgument("非法配对 URI".into()))?;
        match k {
            "d" => d = Some(v),
            "a" => a = Some(v),
            "p" => p = Some(v),
            "s" => s = Some(v),
            _ => {}
        }
    }
    let d = d.ok_or_else(|| CoreError::InvalidArgument("配对 URI 缺少 d 参数".into()))?;
    let a = a.ok_or_else(|| CoreError::InvalidArgument("配对 URI 缺少 a 参数".into()))?;
    let p = p.ok_or_else(|| CoreError::InvalidArgument("配对 URI 缺少 p 参数".into()))?;
    let s = s.ok_or_else(|| CoreError::InvalidArgument("配对 URI 缺少 s 参数".into()))?;

    let device = DeviceId::from_hex(d)
        .map_err(|_| CoreError::InvalidArgument("配对 URI 设备 ID 非法".into()))?;
    let mut addrs = Vec::new();
    for part in a.split(',') {
        let addr: SocketAddr = part
            .parse()
            .map_err(|_| CoreError::InvalidArgument(format!("配对 URI 地址非法: {part}")))?;
        addrs.push(addr);
    }
    if addrs.is_empty() {
        return Err(CoreError::InvalidArgument("配对 URI 无候选地址".into()));
    }
    if p.is_empty() || p.len() > 128 {
        return Err(CoreError::InvalidArgument("配对 URI 会话 ID 非法".into()));
    }
    let secret_bytes = BASE32_NOPAD
        .decode(s.as_bytes())
        .map_err(|_| CoreError::InvalidArgument("配对 URI 口令非法".into()))?;
    let secret: [u8; 16] = secret_bytes
        .try_into()
        .map_err(|_| CoreError::InvalidArgument("配对 URI 口令长度非法".into()))?;
    Ok(ParsedPairUri {
        device,
        addrs,
        session: p.to_string(),
        secret,
    })
}

/// 配对证明码：HMAC-SHA256(key = secret, msg = role || exporter || device)。
pub fn pair_mac(
    secret: &[u8; 16],
    role: &[u8],
    exporter: &[u8; 32],
    device: &DeviceId,
) -> [u8; 32] {
    let mac = mac_for(secret, role, exporter, device);
    mac.finalize().into_bytes().into()
}

/// 常量时间校验对方证明码。
fn mac_matches(
    secret: &[u8; 16],
    role: &[u8],
    exporter: &[u8; 32],
    device: &DeviceId,
    claimed: &[u8; 32],
) -> bool {
    let mac = mac_for(secret, role, exporter, device);
    mac.verify_slice(claimed).is_ok()
}

/// 构造按 pair_mac 定义填充过的 HMAC 实例。
fn mac_for(secret: &[u8; 16], role: &[u8], exporter: &[u8; 32], device: &DeviceId) -> HmacSha256 {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret).expect("HMAC 接受任意长度密钥");
    mac.update(role);
    mac.update(exporter);
    mac.update(&device.0);
    mac
}

/// SAS 数字：blake3(SAS_PREFIX || exporter) 前 4 字节 BE % 1_000_000，6 位。
pub fn sas_code(exporter: &[u8; 32]) -> String {
    let mut input = Vec::with_capacity(SAS_PREFIX.len() + exporter.len());
    input.extend_from_slice(SAS_PREFIX);
    input.extend_from_slice(exporter);
    let h = blake3::hash(&input);
    let be = u32::from_be_bytes([
        h.as_bytes()[0],
        h.as_bytes()[1],
        h.as_bytes()[2],
        h.as_bytes()[3],
    ]);
    format!("{:06}", be % 1_000_000)
}

/// 随机 N 字节。
fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand::thread_rng().fill_bytes(&mut out);
    out
}

/// 会话 ID：16 随机字节的 hex。
fn random_session() -> String {
    crate::identity::hex_encode(&random_bytes::<16>())
}

/// 配对窗口槽：accept_filter 闭包与 Pairing 共享（闭包先于 Pairing 构造）。
#[derive(Clone, Default)]
pub(crate) struct WindowSlot(Arc<StdMutex<Option<PairWindow>>>);

impl WindowSlot {
    fn set(&self, w: PairWindow) {
        *self.0.lock().unwrap() = Some(w);
    }

    /// 窗口当前是否开放（accept_filter 规则 1 使用）。
    pub(crate) fn open(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|w| w.is_open(Instant::now()))
    }

    fn get(&self) -> Option<PairWindow> {
        self.0.lock().unwrap().clone()
    }

    /// 窗口属于该会话时关闭它；返回是否确实关闭。
    fn clear_if_session(&self, session: &str) -> bool {
        let mut g = self.0.lock().unwrap();
        let hit = g.as_ref().is_some_and(|w| w.session == session);
        if hit {
            *g = None;
        }
        hit
    }
}

// ===== 会话状态 =====

/// A 侧（已初始化方）进行中的配对会话。
struct ASession {
    method: PairMethodWire,
    secret: Option<[u8; 16]>,
    exporter: [u8; 32],
    peer: DeviceId,
    peer_name: String,
    peer_type: DeviceType,
    /// 第 4 步签发的 B 证书（Done 后广播 CertUpdate 用）。
    new_cert: Option<DeviceCert>,
    local_confirmed: bool,
    peer_confirmed: bool,
}

/// B 侧（加入方）进行中的配对会话。
struct BSession {
    method: PairMethodWire,
    secret: Option<[u8; 16]>,
    a_device: DeviceId,
    exporter: [u8; 32],
    local_confirmed: bool,
    peer_confirmed: bool,
}

enum Role {
    A(ASession),
    B(BSession),
}

struct Active {
    session: String,
    role: Role,
}

/// Pairing 依赖集合（Service 组装时一次注入）。
pub(crate) struct PairingDeps {
    pub(crate) store: Arc<Store>,
    pub(crate) identity: Arc<RwLock<Identity>>,
    pub(crate) keystore: Arc<dyn KeyStore>,
    pub(crate) conns: ConnManager,
    pub(crate) msg: Messaging,
    pub(crate) trust: crate::trust::Trust,
    pub(crate) events: broadcast::Sender<Event>,
    pub(crate) hello_tx: watch::Sender<HelloInfo>,
    pub(crate) announce_tx: watch::Sender<Option<Announce>>,
    pub(crate) state: Arc<StdRwLock<crate::service::NodeState>>,
    pub(crate) local_addr: SocketAddr,
    pub(crate) window: WindowSlot,
}

struct PairingInner {
    store: Arc<Store>,
    identity: Arc<RwLock<Identity>>,
    keystore: Arc<dyn KeyStore>,
    conns: ConnManager,
    msg: Messaging,
    trust: crate::trust::Trust,
    events: broadcast::Sender<Event>,
    hello_tx: watch::Sender<HelloInfo>,
    announce_tx: watch::Sender<Option<Announce>>,
    state: Arc<StdRwLock<crate::service::NodeState>>,
    local_addr: SocketAddr,
    window: WindowSlot,
    /// 进行中的配对会话（A 或 B 角色，同一时刻至多一个）。
    active: StdMutex<Option<Active>>,
}

/// 「我的设备」配对模块。
#[derive(Clone)]
pub struct Pairing {
    inner: Arc<PairingInner>,
}

impl Pairing {
    /// 构造 Pairing。
    pub(crate) fn new(deps: PairingDeps) -> Pairing {
        Pairing {
            inner: Arc::new(PairingInner {
                store: deps.store,
                identity: deps.identity,
                keystore: deps.keystore,
                conns: deps.conns,
                msg: deps.msg,
                trust: deps.trust,
                events: deps.events,
                hello_tx: deps.hello_tx,
                announce_tx: deps.announce_tx,
                state: deps.state,
                local_addr: deps.local_addr,
                window: deps.window,
                active: StdMutex::new(None),
            }),
        }
    }

    /// 第 1 步：A 开配对窗口。已有窗口 → 覆盖；Qr 生成口令并返回二维码 URI。
    pub async fn start(
        &self,
        method: PairMethodWire,
        ttl: Duration,
    ) -> Result<PairSessionInfo, CoreError> {
        let device = {
            let id = self.inner.identity.read().await;
            if !id.is_initialized() {
                return Err(CoreError::NotInitialized);
            }
            id.device_id()
        };
        let session = random_session();
        let secret = matches!(method, PairMethodWire::Qr).then(random_bytes::<16>);
        let opened_at = Instant::now();
        self.inner.window.set(PairWindow {
            session: session.clone(),
            method,
            secret,
            opened_at,
            ttl,
        });
        let addrs = self.local_addrs();
        let qr_uri = secret.map(|s| pair_uri(&device, &addrs, &session, &s));
        self.emit_status(&session, PairStateDto::WaitingPeer);

        // 到期任务：窗口仍是本会话且已过期 → 关闭并发 Failed{"超时"}。
        let slot = self.inner.window.clone();
        let events = self.inner.events.clone();
        let exp_session = session.clone();
        let deadline = tokio::time::Instant::from_std(opened_at + ttl);
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            if slot.clear_if_session(&exp_session) {
                let _ = events.send(Event::PairStatus {
                    session: exp_session,
                    state: PairStateDto::Failed {
                        reason: "超时".into(),
                    },
                });
            }
        });
        Ok(PairSessionInfo { session, qr_uri })
    }

    /// 第 2 步：B 加入。QrUri 解析后逐地址连接；Device 目标从发现表取地址。
    pub async fn join(
        &self,
        target: PairTarget,
        discovered: &crate::discovery::DiscoveredTable,
    ) -> Result<PairSessionInfo, CoreError> {
        if self.inner.identity.read().await.is_initialized() {
            return Err(CoreError::AlreadyInitialized);
        }
        if self
            .inner
            .store
            .get_setting(crate::settings::SETTING_JOIN_PENDING)?
            .as_deref()
            != Some("1")
        {
            return Err(CoreError::InvalidArgument("请先以加入模式初始化".into()));
        }
        let (addrs, session, secret, method, target_device) = match target {
            PairTarget::QrUri { uri } => {
                let p = parse_pair_uri(&uri)?;
                let d = p.device;
                (p.addrs, p.session, Some(p.secret), PairMethodWire::Qr, d)
            }
            PairTarget::Device { device } => {
                let d = DeviceId::from_hex(&device)
                    .map_err(|_| CoreError::InvalidArgument(format!("非法设备 ID: {device}")))?;
                let (ann, addr) = discovered
                    .get(&d)
                    .ok_or_else(|| CoreError::NotFound(format!("未发现该设备: {device}")))?;
                let port = ann.ports.first().copied().unwrap_or(addr.port());
                (
                    vec![SocketAddr::new(addr.ip(), port)],
                    random_session(),
                    None,
                    PairMethodWire::Sas,
                    d,
                )
            }
        };

        // 逐地址连接，直到对端设备与目标一致。
        let mut a_peer: Option<PeerInfo> = None;
        for addr in &addrs {
            match self.inner.conns.connect(*addr, false).await {
                Ok(dev) if dev == target_device => {
                    a_peer = self.inner.conns.peer(&dev);
                    break;
                }
                Ok(dev) => {
                    // 连到了其他设备（地址复用），断开继续尝试。
                    self.inner.conns.disconnect(&dev).await;
                }
                Err(_) => continue,
            }
        }
        let a = a_peer.ok_or(CoreError::PeerOffline)?;
        tracing::debug!(peer = %a.device, addr = %a.dial_addr, "join：已连接对端");

        let device_name = self
            .inner
            .store
            .get_setting(SETTING_DEVICE_NAME)?
            .unwrap_or_default();
        let device_type = crate::service::handlers::default_device_type();
        // 先登记会话再发帧，避免对端响应先于本地状态就绪。
        {
            let mut g = self.inner.active.lock().unwrap();
            *g = Some(Active {
                session: session.clone(),
                role: Role::B(BSession {
                    method,
                    secret,
                    a_device: a.device,
                    exporter: a.tls_exporter,
                    local_confirmed: false,
                    peer_confirmed: false,
                }),
            });
        }
        self.inner
            .conns
            .send(
                &a.device,
                Frame::Pair(PairMsg::Begin {
                    session: Some(session.clone()),
                    method,
                    device_name,
                    device_type,
                }),
            )
            .await?;
        tracing::debug!(peer = %a.device, method = ?method, "join：Begin 已发送");
        match secret {
            Some(s) => {
                // Qr：Begin 后立即发证明。
                let self_device = self.self_device().await;
                let mac = pair_mac(&s, ROLE_B, &a.tls_exporter, &self_device);
                self.inner
                    .conns
                    .send(&a.device, Frame::Pair(PairMsg::Proof { mac }))
                    .await?;
            }
            None => {
                // Sas：显示本端 SAS。
                self.emit_status(
                    &session,
                    PairStateDto::ShowSas {
                        code: sas_code(&a.tls_exporter),
                    },
                );
            }
        }
        Ok(PairSessionInfo {
            session,
            qr_uri: None,
        })
    }

    /// 第 3 步（Sas）：本端用户确认数字是否一致。
    pub async fn confirm(&self, session: &str, matches: bool) -> Result<(), CoreError> {
        let (peer, is_sas) = {
            let mut g = self.inner.active.lock().unwrap();
            match g.as_mut() {
                Some(a) if a.session == session => match &mut a.role {
                    Role::A(s) => {
                        s.local_confirmed = matches;
                        (s.peer, matches!(s.method, PairMethodWire::Sas))
                    }
                    Role::B(s) => {
                        s.local_confirmed = matches;
                        (s.a_device, matches!(s.method, PairMethodWire::Sas))
                    }
                },
                _ => return Err(CoreError::NotFound("配对会话不存在".into())),
            }
        };
        if !is_sas {
            return Err(CoreError::InvalidArgument("当前配对方式无数字确认".into()));
        }
        self.inner
            .conns
            .send(&peer, Frame::Pair(PairMsg::SasConfirm { ok: matches }))
            .await?;
        if matches {
            self.advance_sas(session).await?;
        } else {
            self.fail_session(session, &peer, "数字不一致").await;
        }
        Ok(())
    }

    /// 取消配对：会话进行中 → 通知对端并失败；仅窗口开放 → 关闭窗口。
    pub async fn cancel(&self, session: &str) -> Result<(), CoreError> {
        let peer = {
            let g = self.inner.active.lock().unwrap();
            match g.as_ref() {
                Some(a) if a.session == session => match &a.role {
                    Role::A(s) => Some(s.peer),
                    Role::B(s) => Some(s.a_device),
                },
                _ => None,
            }
        };
        if let Some(peer) = peer {
            let _ = self
                .inner
                .conns
                .send(
                    &peer,
                    Frame::Pair(PairMsg::Abort {
                        reason: "已取消".into(),
                    }),
                )
                .await;
            self.fail_session(session, &peer, "已取消").await;
            return Ok(());
        }
        if self.inner.window.clear_if_session(session) {
            self.emit_status(
                session,
                PairStateDto::Failed {
                    reason: "已取消".into(),
                },
            );
            return Ok(());
        }
        Err(CoreError::NotFound("配对会话不存在".into()))
    }

    /// 配对帧入口（routing_task 分派）。
    pub async fn on_pair_frame(&self, from: &PeerInfo, msg: PairMsg) -> Result<(), CoreError> {
        tracing::debug!(peer = %from.device, kind = %pair_kind(&msg), "收到配对帧");
        match msg {
            PairMsg::Begin {
                session,
                method,
                device_name,
                device_type,
            } => {
                self.on_begin(from, session, method, device_name, device_type)
                    .await
            }
            PairMsg::Proof { mac } => self.on_proof(from, mac).await,
            PairMsg::SasConfirm { ok } => self.on_sas_confirm(from, ok).await,
            PairMsg::Bundle(bundle) => self.on_bundle(from, *bundle).await,
            PairMsg::Done => self.on_done(from).await,
            PairMsg::Abort { reason } => self.on_abort(from, reason).await,
        }
    }

    // ---- 入站帧处理 ----

    /// 第 3 步：A 收 Begin。无窗口或方法不符 → Abort{"未开启配对"}。
    async fn on_begin(
        &self,
        from: &PeerInfo,
        begin_session: Option<String>,
        method: PairMethodWire,
        device_name: String,
        device_type: DeviceType,
    ) -> Result<(), CoreError> {
        let window = self
            .inner
            .window
            .get()
            .filter(|w| w.is_open(Instant::now()));
        let Some(w) = window.filter(|w| w.method == method) else {
            tracing::debug!(peer = %from.device, "Begin 被拒：窗口未开或方法不符");
            let _ = self
                .inner
                .conns
                .send(
                    &from.device,
                    Frame::Pair(PairMsg::Abort {
                        reason: "未开启配对".into(),
                    }),
                )
                .await;
            return Ok(());
        };
        let session = begin_session.unwrap_or_else(random_session);
        {
            let mut g = self.inner.active.lock().unwrap();
            // 并发配对会话：新 Begin 覆盖旧会话。
            *g = Some(Active {
                session: session.clone(),
                role: Role::A(ASession {
                    method,
                    secret: w.secret,
                    exporter: from.tls_exporter,
                    peer: from.device,
                    peer_name: device_name,
                    peer_type: device_type,
                    new_cert: None,
                    local_confirmed: false,
                    peer_confirmed: false,
                }),
            });
        }
        if matches!(method, PairMethodWire::Sas) {
            self.emit_status(
                &session,
                PairStateDto::ShowSas {
                    code: sas_code(&from.tls_exporter),
                },
            );
        }
        Ok(())
    }

    /// Qr 证明：A 校验 B 的证明（通过后回证明并送 Bundle）；B 校验 A 的证明。
    async fn on_proof(&self, from: &PeerInfo, claimed: [u8; 32]) -> Result<(), CoreError> {
        let Some(session) = self.session_for_peer(&from.device) else {
            return Ok(());
        };
        let info = {
            let g = self.inner.active.lock().unwrap();
            match g
                .as_ref()
                .and_then(|a| (a.session == session).then_some(&a.role))
            {
                Some(Role::A(s)) if matches!(s.method, PairMethodWire::Qr) => {
                    Some((true, s.secret, s.exporter))
                }
                Some(Role::B(s)) if matches!(s.method, PairMethodWire::Qr) => {
                    Some((false, s.secret, s.exporter))
                }
                _ => None,
            }
        };
        let Some((is_a, secret, exporter)) = info else {
            return Ok(());
        };
        let Some(secret) = secret else {
            return Ok(()); // Qr 会话必有口令，异常状态直接忽略
        };
        if is_a {
            if !mac_matches(&secret, ROLE_B, &exporter, &from.device, &claimed) {
                let _ = self
                    .inner
                    .conns
                    .send(
                        &from.device,
                        Frame::Pair(PairMsg::Abort {
                            reason: "口令错误".into(),
                        }),
                    )
                    .await;
                self.fail_session(&session, &from.device, "口令错误").await;
                return Ok(());
            }
            // 回自己的证明并进入第 4 步。
            let self_device = self.self_device().await;
            let mac = pair_mac(&secret, ROLE_A, &exporter, &self_device);
            self.inner
                .conns
                .send(&from.device, Frame::Pair(PairMsg::Proof { mac }))
                .await?;
            self.a_send_bundle(&session).await?;
        } else {
            let Some(a_device) = self.b_a_device(&session) else {
                return Ok(());
            };
            if !mac_matches(&secret, ROLE_A, &exporter, &a_device, &claimed) {
                let _ = self
                    .inner
                    .conns
                    .send(
                        &from.device,
                        Frame::Pair(PairMsg::Abort {
                            reason: "口令错误".into(),
                        }),
                    )
                    .await;
                self.fail_session(&session, &from.device, "口令错误").await;
            }
        }
        Ok(())
    }

    /// Sas 对端确认到达：false → 双方失败；true 且本端已确认 → 推进。
    async fn on_sas_confirm(&self, from: &PeerInfo, ok: bool) -> Result<(), CoreError> {
        let Some(session) = self.session_for_peer(&from.device) else {
            return Ok(());
        };
        self.set_peer_confirmed(&session, ok);
        if !ok {
            self.fail_session(&session, &from.device, "数字不一致")
                .await;
            return Ok(());
        }
        self.advance_sas(&session).await
    }

    /// 第 5 步：B 收 Bundle。校验通过后采纳身份并落库；失败保持未初始化。
    async fn on_bundle(&self, from: &PeerInfo, bundle: PairBundle) -> Result<(), CoreError> {
        let Some(session) = self.session_for_peer(&from.device) else {
            return Ok(());
        };
        let Some(a_device) = self.b_a_device(&session) else {
            return Ok(());
        };
        let self_device = self.self_device().await;

        // 校验：设备指向自己、证书验签、用户密钥与证书一致。
        let reject = |reason: &str| reason.to_string();
        let reason = if bundle.device_cert.device != self_device {
            Some(reject("证书设备不符"))
        } else if bundle.device_cert.verify().is_err() {
            Some(reject("证书验签失败"))
        } else if UserKey::from_bytes(&bundle.user_secret).user_id() != bundle.device_cert.user {
            Some(reject("用户身份不符"))
        } else {
            None
        };
        if let Some(reason) = reason {
            let _ = self
                .inner
                .conns
                .send(
                    &a_device,
                    Frame::Pair(PairMsg::Abort {
                        reason: reason.clone(),
                    }),
                )
                .await;
            self.fail_session(&session, &a_device, &reason).await;
            return Ok(());
        }

        let cert = bundle.device_cert.clone();
        {
            let mut id = self.inner.identity.write().await;
            id.adopt_user(
                self.inner.keystore.as_ref(),
                &self.inner.store,
                bundle.user_secret,
                cert.clone(),
            )?;
        }
        let user_id = cert.user;
        let now = crate::service::now_millis();
        self.inner.store.upsert_user(&UserRow {
            user: user_id,
            display_name: bundle.display_name.clone(),
            is_self: true,
            updated_at: now,
        })?;
        let device_name = self
            .inner
            .store
            .get_setting(SETTING_DEVICE_NAME)?
            .unwrap_or_default();
        self.inner.store.upsert_device(&DeviceRow {
            device: self_device,
            user: user_id,
            cert: cert.clone(),
            name: device_name,
            device_type: crate::service::handlers::default_device_type(),
            last_addr: None,
            last_path: None,
            last_seen: None,
            revoked: false,
        })?;
        for dc in &bundle.my_devices {
            if dc.device == self_device {
                continue;
            }
            self.inner.store.upsert_device(&DeviceRow {
                device: dc.device,
                user: dc.user,
                cert: dc.clone(),
                name: dc.device_name.clone(),
                device_type: dc.device_type,
                last_addr: None,
                last_path: None,
                last_seen: None,
                revoked: false,
            })?;
        }
        for r in &bundle.revocations {
            self.inner.store.insert_revocation(r)?;
        }
        for rec in &bundle.contacts {
            self.inner.trust.apply_contact_record(rec)?;
        }
        self.inner.store.ensure_conversation(&ConvRow {
            conv: ConvId::self_devices(&user_id),
            kind: ConvKindRow::SelfDevices,
            peer_user: None,
            title: None,
            created_at: now,
        })?;
        // 清除待接入标记。
        self.inner
            .store
            .set_setting(crate::settings::SETTING_JOIN_PENDING, "")?;

        // 发 Done、事件，更新 HelloInfo/announce/self_user。
        self.inner
            .conns
            .send(&a_device, Frame::Pair(PairMsg::Done))
            .await?;
        self.emit_status(&session, PairStateDto::Done);
        let _ = self.inner.events.send(Event::DevicesChanged);
        let status = self.inner.state.read().unwrap().status.clone();
        let mut hello = self.inner.hello_tx.borrow().clone();
        hello.display_name = bundle.display_name.clone();
        hello.cert = Some(cert.clone());
        self.inner.hello_tx.send_replace(hello);
        if let Some(ann) = crate::service::build_announce(
            &self.inner.identity,
            &bundle.display_name,
            status,
            self.inner.local_addr.port(),
        )
        .await
        {
            self.inner.announce_tx.send_replace(Some(ann));
        }
        self.inner.state.write().unwrap().self_user = Some(user_id);

        // 断开配对连接，以常规身份重连 A（之后走正常同步）。
        // 稍候再断：Done 帧经 write 通道异步冲刷，且对端需时间处理 Done 并
        // 清理旧连接登记；立即断开+重连会与对端的重复连接裁决竞态丢帧。
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        self.inner.conns.disconnect(&a_device).await;
        if let Err(e) = self.inner.conns.connect(from.dial_addr, false).await {
            tracing::warn!(peer = %a_device, error = %e, "配对完成后的重连失败");
        }
        let mut g = self.inner.active.lock().unwrap();
        if g.as_ref().is_some_and(|a| a.session == session) {
            *g = None;
        }
        Ok(())
    }

    /// 第 6 步：A 收 Done。发事件、SelfDevices 可见系统消息、广播 CertUpdate。
    async fn on_done(&self, from: &PeerInfo) -> Result<(), CoreError> {
        let info = {
            let g = self.inner.active.lock().unwrap();
            match g.as_ref() {
                Some(a) => match &a.role {
                    Role::A(s) if s.peer == from.device => Some((
                        a.session.clone(),
                        s.new_cert.clone(),
                        s.peer_name.clone(),
                        s.peer,
                    )),
                    _ => None,
                },
                None => None,
            }
        };
        let Some((session, new_cert, peer_name, peer_device)) = info else {
            return Ok(());
        };
        self.emit_status(&session, PairStateDto::Done);
        let _ = self.inner.events.send(Event::DevicesChanged);

        // SelfDevices 会话发可见系统消息。
        let self_user = {
            let id = self.inner.identity.read().await;
            id.user_id()
        };
        if let Some(u) = self_user {
            let conv = ConvId::self_devices(&u);
            if let Err(e) = self
                .inner
                .msg
                .send(
                    &conv,
                    MsgBody::System {
                        event: SystemEvent::DeviceAdded {
                            device: peer_device,
                            name: peer_name,
                        },
                    },
                )
                .await
            {
                tracing::warn!(error = %e, "DeviceAdded 系统消息发送失败");
            }
        }

        // 向所有已连接自己设备与 Accepted 联系人发 CertUpdate。
        if let Some(cert) = new_cert {
            for p in self.inner.conns.peers() {
                let Some(c) = &p.cert else { continue };
                let mine = self_user.is_some_and(|u| u == c.user);
                let contact = self.inner.trust.is_contact(&c.user);
                if mine || contact {
                    let _ = self
                        .inner
                        .conns
                        .send(&p.device, Frame::CertUpdate(vec![cert.clone()]))
                        .await;
                }
            }
        }

        // 关闭窗口、清除会话。
        self.inner.window.clear_if_session(&session);
        let mut g = self.inner.active.lock().unwrap();
        if g.as_ref().is_some_and(|a| a.session == session) {
            *g = None;
        }
        Ok(())
    }

    /// 第 7 步：对端中止。
    async fn on_abort(&self, from: &PeerInfo, reason: String) -> Result<(), CoreError> {
        let Some(session) = self.session_for_peer(&from.device) else {
            return Ok(());
        };
        self.fail_session(&session, &from.device, &reason).await;
        Ok(())
    }

    // ---- 内部辅助 ----

    /// Sas 双端确认后的推进：双方发 Verifying；A 继续第 4 步送 Bundle。
    async fn advance_sas(&self, session: &str) -> Result<(), CoreError> {
        let both = {
            let g = self.inner.active.lock().unwrap();
            match g.as_ref() {
                Some(a) if a.session == session => match &a.role {
                    Role::A(s) => s.local_confirmed && s.peer_confirmed,
                    Role::B(s) => s.local_confirmed && s.peer_confirmed,
                },
                _ => false,
            }
        };
        if !both {
            return Ok(());
        }
        self.emit_status(session, PairStateDto::Verifying);
        let is_a = {
            let g = self.inner.active.lock().unwrap();
            matches!(g.as_ref().map(|a| &a.role), Some(Role::A(_)))
        };
        if is_a {
            self.a_send_bundle(session).await?;
        }
        Ok(())
    }

    /// 第 4 步：A 签发 B 的证书、组配对包并发送。
    async fn a_send_bundle(&self, session: &str) -> Result<(), CoreError> {
        let (peer, peer_name, peer_type) = {
            let g = self.inner.active.lock().unwrap();
            match g.as_ref() {
                Some(a) => match &a.role {
                    Role::A(s) if a.session == session => {
                        (s.peer, s.peer_name.clone(), s.peer_type)
                    }
                    _ => return Ok(()),
                },
                None => return Ok(()),
            }
        };
        let (user_secret, user) = {
            let id = self.inner.identity.read().await;
            let key = id.user.as_ref().ok_or(CoreError::NotInitialized)?;
            (key.to_bytes(), key.user_id())
        };
        let now = crate::service::now_millis();
        let new_cert = DeviceCert::issue(
            &UserKey::from_bytes(&user_secret),
            peer,
            peer_name.clone(),
            peer_type,
            now,
        );
        let mut my_devices = vec![new_cert.clone()];
        for row in self.inner.store.devices_of_user(&user)? {
            if !row.revoked && row.device != new_cert.device {
                my_devices.push(row.cert);
            }
        }
        let revocations = self.inner.store.list_revocations()?;
        let mut contacts = Vec::new();
        for row in self.inner.store.list_contacts(None)? {
            if matches!(row.state, ContactState::Accepted | ContactState::Blocked) {
                if let Some(rec) = self.inner.trust.contact_record(&row.user)? {
                    contacts.push(rec);
                }
            }
        }
        let display_name = self
            .inner
            .store
            .get_setting("display_name")?
            .unwrap_or_default();
        let bundle = PairBundle {
            user_secret,
            display_name,
            device_cert: new_cert.clone(),
            my_devices,
            revocations,
            contacts,
        };
        self.inner
            .conns
            .send(&peer, Frame::Pair(PairMsg::Bundle(Box::new(bundle))))
            .await?;
        // 记录新证书供 Done 后广播。
        let mut g = self.inner.active.lock().unwrap();
        if let Some(a) = g.as_mut() {
            if a.session == session {
                if let Role::A(s) = &mut a.role {
                    s.new_cert = Some(new_cert);
                }
            }
        }
        Ok(())
    }

    /// 会话失败：清除状态、发 Failed 事件并断开对端。
    async fn fail_session(&self, session: &str, device: &DeviceId, reason: &str) {
        {
            let mut g = self.inner.active.lock().unwrap();
            if g.as_ref().is_some_and(|a| a.session == session) {
                *g = None;
            }
        }
        self.emit_status(
            session,
            PairStateDto::Failed {
                reason: reason.to_string(),
            },
        );
        // 缓冲后再断开：此前发出的帧经 write 通道异步冲刷，且立即断开会因
        // 读半部带未读数据关闭 socket（RST）而丢失在途帧。
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        self.inner.conns.disconnect(device).await;
    }

    /// 找到与该对端设备关联的进行中会话。
    fn session_for_peer(&self, device: &DeviceId) -> Option<String> {
        let g = self.inner.active.lock().unwrap();
        g.as_ref()
            .filter(|a| match &a.role {
                Role::A(s) => s.peer == *device,
                Role::B(s) => s.a_device == *device,
            })
            .map(|a| a.session.clone())
    }

    /// B 会话的 A 设备 ID（非 B 会话返回 None）。
    fn b_a_device(&self, session: &str) -> Option<DeviceId> {
        let g = self.inner.active.lock().unwrap();
        match g.as_ref() {
            Some(a) if a.session == session => match &a.role {
                Role::B(s) => Some(s.a_device),
                _ => None,
            },
            _ => None,
        }
    }

    /// 记录对端确认状态。
    fn set_peer_confirmed(&self, session: &str, ok: bool) {
        let mut g = self.inner.active.lock().unwrap();
        if let Some(a) = g.as_mut() {
            if a.session == session {
                match &mut a.role {
                    Role::A(s) => s.peer_confirmed = ok,
                    Role::B(s) => s.peer_confirmed = ok,
                }
            }
        }
    }

    /// 本机设备 ID。
    async fn self_device(&self) -> DeviceId {
        self.inner.identity.read().await.device_id()
    }

    /// 发配对状态事件。
    fn emit_status(&self, session: &str, state: PairStateDto) {
        let _ = self.inner.events.send(Event::PairStatus {
            session: session.to_string(),
            state,
        });
    }

    /// 本机候选地址：各非回环网卡地址 + 监听端口；无则用监听地址本身。
    fn local_addrs(&self) -> Vec<SocketAddr> {
        let port = self.inner.local_addr.port();
        let mut out = Vec::new();
        if let Ok(ifaces) = if_addrs::get_if_addrs() {
            for i in ifaces {
                if i.is_loopback() {
                    continue;
                }
                out.push(SocketAddr::new(i.ip(), port));
            }
        }
        if out.is_empty() {
            out.push(self.inner.local_addr);
        }
        out
    }
}

// ===== 单测 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_uri_roundtrip_with_ipv6() {
        let device = DeviceId([0xab; 32]);
        let addrs = [
            "[fe80::1]:53320".parse::<SocketAddr>().unwrap(),
            "192.168.1.2:53320".parse::<SocketAddr>().unwrap(),
        ];
        let secret = [7u8; 16];
        let uri = pair_uri(&device, &addrs, "aabbccddeeff0011", &secret);
        let parsed = parse_pair_uri(&uri).unwrap();
        assert_eq!(parsed.device, device);
        assert_eq!(parsed.addrs, addrs);
        assert_eq!(parsed.session, "aabbccddeeff0011");
        assert_eq!(parsed.secret, secret);
    }

    #[test]
    fn sas_code_is_six_digits_and_deterministic() {
        let e = [7u8; 32];
        assert_eq!(sas_code(&e), sas_code(&e));
        assert_eq!(sas_code(&e).len(), 6);
    }

    #[test]
    fn pair_mac_differs_by_role() {
        let secret = [1u8; 16];
        let exporter = [2u8; 32];
        let device = DeviceId([3u8; 32]);
        let a = pair_mac(&secret, ROLE_A, &exporter, &device);
        let b = pair_mac(&secret, ROLE_B, &exporter, &device);
        assert_ne!(a, b, "role 不同 MAC 应不同");
        let other = pair_mac(&[9u8; 16], ROLE_A, &exporter, &device);
        assert_ne!(a, other, "secret 不同 MAC 应不同");
    }

    #[test]
    fn pair_window_expires() {
        let w = PairWindow {
            session: "s".into(),
            method: PairMethodWire::Qr,
            secret: None,
            opened_at: Instant::now(),
            ttl: Duration::from_millis(1),
        };
        assert!(w.is_open(Instant::now()));
        std::thread::sleep(Duration::from_millis(5));
        assert!(!w.is_open(Instant::now()));
    }
}
