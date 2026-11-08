// P5 LocalSend 桥：把 lanmsg-localsend 的收发事件映射到会话 / 消息 / 请求。
// LocalSend 对端以指纹（小写）标识，伪设备 ID 只用作消息作者；这类会话不验签、不同步。
// 注释中文，标识符英文。

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use lanmsg_api::types::{
    Event, PathClassDto, PeerView, RequestAction, RequestKind, RequestPreviewView, RequestView,
};
use lanmsg_localsend::{
    LocalSend, LsConfig, LsDecision, LsError, LsEvent, LsFileMeta, LsPeer, LOCALSEND_PORT,
};
use lanmsg_proto::ids::{ConvId, DeviceId, MsgId};
use lanmsg_proto::types::{BlobMeta, Envelope, MsgBody, PathClass, Signature64, SystemEvent};

use crate::error::CoreError;
use crate::hlc::HlcClock;
use crate::identity::Identity;
use crate::net::path::{cached_ifaces, classify};
use crate::platform::ThumbnailProvider;
use crate::service::now_millis;
use crate::service::views::{conv_summary, message_view};
use crate::store::{
    pseudo_device_id, BlobLocalKind, ConvKindRow, ConvRow, LsPeerRow, MsgStateRow, Store,
    StoredMessage,
};

/// 对端超过该时长没有再被发现，视为离线。
const PEER_OFFLINE_AFTER: Duration = Duration::from_secs(90);
/// 离线检查间隔。
const OFFLINE_SWEEP: Duration = Duration::from_secs(15);
/// 同一对端的数据库刷新间隔（发现通告很频繁，不逐条落库）。
const PEER_DB_REFRESH: Duration = Duration::from_secs(20);
/// 内存中记录的对端数量上限（防止局域网内恶意通告撑爆内存与数据库）。
const MAX_TRACKED_PEERS: usize = 512;
/// 别名最大字符数。
const ALIAS_MAX_CHARS: usize = 64;
/// 数据库中保留的 LocalSend 对端上限（含已信任与已屏蔽）。
const MAX_STORED_PEERS: u64 = 512;
/// 同时跟踪的接收会话上限与过期时间。
const MAX_SESSIONS: usize = 64;
const SESSION_TTL: Duration = Duration::from_secs(3600);
/// 每个指纹最多记录的来源 IP 数。
const MAX_IPS_PER_PEER: usize = 4;
/// 文字请求预览的最大字符数。
const TEXT_PREVIEW_CHARS: usize = 100;
/// 等待文件消息准备完成（出现在 messages 表）的最长时间。
const PREPARE_WAIT: Duration = Duration::from_secs(600);

/// 运行配置（改变任意一项都需要重启服务）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LsKey {
    pub alias: String,
    pub download_dir: PathBuf,
    pub http_port: u16,
    pub enable_discovery: bool,
}

/// 发送失败的分类（界面据此显示不同提示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LsSendFailure {
    /// 对端当前不在线（未被发现）。
    Offline,
    /// 对端开启了 PIN。
    Pin,
    /// 对端正忙：已有传输会话（409）或待审批已满（429）。
    Busy,
    /// 对端拒绝（403）。
    Declined,
    /// 其他错误（英文排障信息）。
    Other(String),
}

fn classify_failure(e: &LsError) -> LsSendFailure {
    match e {
        LsError::Status(401) => LsSendFailure::Pin,
        LsError::Status(409) | LsError::Status(429) => LsSendFailure::Busy,
        LsError::Status(403) => LsSendFailure::Declined,
        other => LsSendFailure::Other(other.to_string()),
    }
}

/// 桥所需的外部依赖。
pub(crate) struct LsDeps {
    pub store: Arc<Store>,
    pub identity: Arc<StdRwLock<Identity>>,
    pub clock: Arc<StdMutex<HlcClock>>,
    pub events: broadcast::Sender<Event>,
    pub thumbnailer: Option<Arc<dyn ThumbnailProvider>>,
}

/// 等待用户决定的请求。
struct PendingReq {
    reply: oneshot::Sender<LsDecision>,
    fingerprint: String,
    /// 请求方的发现信息（批准后才落库）。
    peer: LsPeer,
    text: Option<String>,
    view: RequestView,
}

/// 已发现对端的内存状态。
struct SeenPeer {
    peer: LsPeer,
    /// 各来源 IP 最近一次被发现的时间（同一指纹可能出现在多个 IP）。
    by_ip: HashMap<IpAddr, (LsPeer, Instant)>,
    alias: String,
    last_event: Instant,
    last_db: Instant,
    online: bool,
}

struct BridgeInner {
    ls: LocalSend,
    own_fingerprint: String,
    deps: LsDeps,
    pending: StdMutex<HashMap<String, PendingReq>>,
    seen: StdMutex<HashMap<String, SeenPeer>>,
}

/// LocalSend 桥；`stop` 后不可再用。
pub struct LocalSendBridge {
    inner: Arc<BridgeInner>,
    tasks: StdMutex<Vec<JoinHandle<()>>>,
}

fn lock<T>(m: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    // 持锁段内不会 panic；即便中毒，数据仍可用
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 指纹规范化：小写；必须是 16–128 位十六进制，否则视为非法。
fn normalize_fingerprint(raw: &str) -> Option<String> {
    let fp = raw.to_ascii_lowercase();
    let ok = (16..=128).contains(&fp.len()) && fp.bytes().all(|b| b.is_ascii_hexdigit());
    ok.then_some(fp)
}

/// 别名清理：去控制字符、截断、空则用指纹前 8 位。
fn clean_alias(raw: &str, fingerprint: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| !c.is_control() && !is_format_char(*c))
        .take(ALIAS_MAX_CHARS)
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        fingerprint.chars().take(8).collect()
    } else {
        s
    }
}

/// Unicode 格式字符（Cf 类中常被用来伪装文字的部分：零宽、双向控制、BOM 等）。
fn is_format_char(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}'
        | '\u{E0000}'..='\u{E007F}')
}

fn is_lan(ip: IpAddr) -> bool {
    classify(ip, &cached_ifaces(), false) == PathClass::Lan
}

/// 来源判定结果。
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Reject,
    Auto,
    Ask,
}

impl LocalSendBridge {
    /// 启动 LocalSend 服务与事件循环。
    pub(crate) async fn start(deps: LsDeps, cfg: LsConfig) -> Result<LocalSendBridge, CoreError> {
        let (tx, rx) = mpsc::channel(64);
        let ls = LocalSend::start(cfg, tx)
            .await
            .map_err(|e| CoreError::Tls(e.to_string()))?;
        let own_fingerprint = ls.info().fingerprint.to_ascii_lowercase();
        let inner = Arc::new(BridgeInner {
            ls,
            own_fingerprint,
            deps,
            pending: StdMutex::new(HashMap::new()),
            seen: StdMutex::new(HashMap::new()),
        });
        let (fin_tx, fin_rx) = mpsc::unbounded_channel();
        let tasks = vec![
            tokio::spawn(event_loop(inner.clone(), rx, fin_tx)),
            tokio::spawn(finisher_loop(inner.clone(), fin_rx)),
            tokio::spawn(sweep_loop(inner.clone())),
        ];
        Ok(LocalSendBridge {
            inner,
            tasks: StdMutex::new(tasks),
        })
    }

    /// 停止服务：中止进行中的会话（清理 `.part`）并结束后台任务。
    pub async fn stop(&self) {
        self.inner.ls.shutdown().await;
        for t in lock(&self.tasks).drain(..) {
            t.abort();
        }
        lock(&self.inner.pending).clear();
    }

    /// TCP 端口是否绑定成功（能否接收）。
    pub fn listening(&self) -> bool {
        self.inner.ls.listening()
    }

    /// 待用户处理的请求（已超时的不再返回）。
    pub fn pending_requests(&self) -> Vec<RequestView> {
        let mut g = lock(&self.inner.pending);
        g.retain(|_, p| !p.reply.is_closed());
        let mut v: Vec<RequestView> = g.values().map(|p| p.view.clone()).collect();
        v.sort_by_key(|r| r.created_at);
        v
    }

    /// 处理 `ls-` 前缀的请求。请求已超时或不存在 → `NotFound`。
    pub async fn respond(&self, request: &str, action: RequestAction) -> Result<(), CoreError> {
        let p = lock(&self.inner.pending)
            .remove(request)
            .ok_or_else(|| CoreError::NotFound(format!("request not found: {request}")))?;
        if p.reply.is_closed() {
            return Err(CoreError::NotFound(format!("request expired: {request}")));
        }
        let store = &self.inner.deps.store;
        let accept = matches!(
            action,
            RequestAction::AcceptOnce | RequestAction::AddContact
        );
        // 只有用户做出接受 / 信任 / 屏蔽的决定后才落库对端与会话（拒绝不留痕）
        let persisted = match action {
            RequestAction::AcceptOnce | RequestAction::Block => self
                .inner
                .ensure_peer(&p.peer, &p.fingerprint, false)
                .map(|_| ()),
            RequestAction::AddContact => {
                // 信任以本次请求的来源 IP 为准：之后只有同一 IP 的请求才会自动接受
                self.inner
                    .ensure_peer(&p.peer, &p.fingerprint, true)
                    .map(|_| ())
            }
            RequestAction::Reject => Ok(()),
        };
        if let Err(e) = persisted {
            let _ = p.reply.send(LsDecision::Reject);
            return Err(e);
        }
        match action {
            RequestAction::AddContact => {
                store.set_ls_trust(&p.fingerprint, true, false)?;
                let _ = self.inner.deps.events.send(Event::ContactsChanged);
            }
            RequestAction::Block => {
                store.set_ls_trust(&p.fingerprint, false, true)?;
                let _ = self.inner.deps.events.send(Event::ContactsChanged);
            }
            RequestAction::AcceptOnce | RequestAction::Reject => {}
        }
        if accept {
            if let Some(text) = p.text {
                // 先入库再应答，入库失败时让发送方得到拒绝
                if let Err(e) = self
                    .inner
                    .insert_remote(&p.fingerprint, MsgBody::Text { text })
                {
                    let _ = p.reply.send(LsDecision::Reject);
                    return Err(e);
                }
            }
        }
        let decision = if accept {
            LsDecision::Accept
        } else {
            LsDecision::Reject
        };
        p.reply
            .send(decision)
            .map_err(|_| CoreError::NotFound(format!("request expired: {request}")))
    }

    /// 实际监听端口（测试里端口为 0 时取系统分配的端口）。
    pub fn port(&self) -> u16 {
        self.inner.ls.info().port
    }

    /// 本机 LocalSend 证书指纹（小写）。
    pub fn fingerprint(&self) -> String {
        self.inner.own_fingerprint.clone()
    }

    /// 发给尚无会话的在线对端前，按需建立会话（用户主动发起才落库）。
    /// 返回该会话现在是否存在。
    pub fn ensure_conv(&self, conv: &ConvId) -> Result<bool, CoreError> {
        let found = {
            let g = lock(&self.inner.seen);
            g.iter()
                .find(|(fp, s)| s.online && ConvId::localsend(fp) == *conv)
                .map(|(fp, s)| (fp.clone(), s.peer.clone()))
        };
        match found {
            Some((fp, peer)) => {
                self.inner.ensure_peer(&peer, &fp, false)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// 对端当前是否在线（最近被发现过）。
    pub fn peer_online(&self, conv: &ConvId) -> bool {
        self.inner.peer_for_conv(conv).is_some()
    }

    /// 发送文字到 LocalSend 会话。
    pub async fn send_text(&self, conv: &ConvId, text: &str) -> Result<(), LsSendFailure> {
        let peer = self
            .inner
            .peer_for_conv(conv)
            .ok_or(LsSendFailure::Offline)?;
        self.inner
            .ls
            .send_text(&peer, text)
            .await
            .map_err(|e| classify_failure(&e))
    }

    /// 发送文件到 LocalSend 会话。
    pub async fn send_paths(&self, conv: &ConvId, paths: &[PathBuf]) -> Result<(), LsSendFailure> {
        let peer = self
            .inner
            .peer_for_conv(conv)
            .ok_or(LsSendFailure::Offline)?;
        // 进度由发送端 try_send 上报，接收端关闭即忽略
        let (ptx, _prx) = mpsc::channel(1);
        self.inner
            .ls
            .send_files(&peer, paths, ptx)
            .await
            .map_err(|e| classify_failure(&e))
    }

    /// 后台投递文字：完成后更新本机已发消息的状态（成功 Delivered，失败 Failed）。
    pub async fn deliver_text(&self, conv: ConvId, id: MsgId, text: String) {
        let result = self.send_text(&conv, &text).await;
        self.inner.finish_send(&conv, &id, result);
    }

    /// 后台投递文件：消息行由 `Messaging` 在后台哈希后才入库，先等它出现再更新状态。
    pub async fn deliver_files(&self, conv: ConvId, id: MsgId, paths: Vec<PathBuf>) {
        let result = self.send_paths(&conv, &paths).await;
        let deadline = Instant::now() + PREPARE_WAIT;
        while Instant::now() < deadline {
            if matches!(self.inner.deps.store.get_message(&id), Ok(Some(_))) {
                self.inner.finish_send(&conv, &id, result);
                return;
            }
            let still_preparing = self
                .inner
                .deps
                .store
                .list_preparing()
                .map(|v| v.iter().any(|r| r.author == id.author && r.seq == id.seq))
                .unwrap_or(false);
            if !still_preparing {
                // 准备失败，Messaging 已把消息标为失败
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

impl BridgeInner {
    fn self_device(&self) -> DeviceId {
        self.deps
            .identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .device_id()
    }

    /// 对端当前可达的 LocalSend 地址。
    /// 已信任的对端只发往记录的地址（新通告的其他 IP 不会改变发送目标）；
    /// 其余对端取最近一次被发现的记录。
    fn peer_for_conv(&self, conv: &ConvId) -> Option<LsPeer> {
        let (fp, latest, by_ip) = {
            let g = lock(&self.seen);
            let (fp, s) = g
                .iter()
                .find(|(fp, s)| s.online && ConvId::localsend(fp) == *conv)?;
            (fp.clone(), s.peer.clone(), s.by_ip.clone())
        };
        let row = self.deps.store.get_ls_peer(&fp).ok().flatten();
        match row {
            Some(r) if r.trusted => {
                let addr: IpAddr = r.last_addr.as_deref()?.parse().ok()?;
                by_ip
                    .get(&addr)
                    .filter(|(_, at)| at.elapsed() < PEER_OFFLINE_AFTER)
                    .map(|(p, _)| p.clone())
            }
            _ => Some(latest),
        }
    }

    /// 来源判定：非局域网或指纹非法 → 拒绝；已屏蔽 → 拒绝；
    /// 已信任且来源 IP 与记录一致 → 自动接受；其余询问用户。
    fn judge(&self, peer: &LsPeer) -> (Verdict, Option<String>) {
        let Some(fp) = normalize_fingerprint(&peer.info.fingerprint) else {
            return (Verdict::Reject, None);
        };
        if fp == self.own_fingerprint || !is_lan(peer.ip) {
            return (Verdict::Reject, Some(fp));
        }
        let row = self.deps.store.get_ls_peer(&fp).ok().flatten();
        match row {
            Some(r) if r.blocked => (Verdict::Reject, Some(fp)),
            Some(r)
                if r.trusted
                    && r.last_addr
                        .as_deref()
                        .and_then(|a| a.parse::<IpAddr>().ok())
                        == Some(peer.ip) =>
            {
                (Verdict::Auto, Some(fp))
            }
            _ => (Verdict::Ask, Some(fp)),
        }
    }

    /// 由发现信息构造对端行（不落库）。已信任对端的别名、型号与地址只在用户确认
    /// （`confirm`）时更新，避免被伪造通告改写。
    fn row_from_peer(
        &self,
        peer: &LsPeer,
        fp: &str,
        existing: Option<&LsPeerRow>,
        confirm: bool,
    ) -> LsPeerRow {
        let keep = existing.filter(|r| r.trusted && !confirm);
        LsPeerRow {
            fingerprint: fp.to_string(),
            pseudo_device: pseudo_device_id(fp),
            alias: keep
                .map(|r| r.alias.clone())
                .unwrap_or_else(|| clean_alias(&peer.info.alias, fp)),
            device_model: match keep {
                Some(r) => r.device_model.clone(),
                None => peer.info.device_model.clone(),
            },
            device_type: match keep {
                Some(r) => r.device_type.clone(),
                None => peer.info.device_type.clone(),
            },
            last_addr: match keep {
                Some(r) => r.last_addr.clone(),
                None => Some(peer.ip.to_string()),
            },
            protocol: match peer.info.protocol {
                lanmsg_localsend::LsProtocol::Http => "http".into(),
                lanmsg_localsend::LsProtocol::Https => "https".into(),
            },
            port: peer.info.port,
            last_seen: now_millis(),
            trusted: existing.is_some_and(|r| r.trusted),
            blocked: existing.is_some_and(|r| r.blocked),
        }
    }

    /// 落库一个对端并确保会话存在（只在用户主动操作或对端已存在时调用）。
    /// 新对端超过库级上限时拒绝。
    fn ensure_peer(&self, peer: &LsPeer, fp: &str, confirm: bool) -> Result<LsPeerRow, CoreError> {
        let store = &self.deps.store;
        let existing = store.get_ls_peer(fp)?;
        if existing.is_none() && store.count_ls_peers()? >= MAX_STORED_PEERS {
            return Err(CoreError::Rejected("too many localsend peers".into()));
        }
        let row = self.row_from_peer(peer, fp, existing.as_ref(), confirm);
        store.upsert_ls_peer(&row)?;
        let conv = ConvId::localsend(fp);
        store.ensure_conversation(&ConvRow {
            conv,
            kind: ConvKindRow::LocalSend,
            peer_user: None,
            title: Some(row.alias.clone()),
            created_at: row.last_seen,
        })?;
        if existing.as_ref().is_some_and(|r| r.alias != row.alias) {
            store.set_conversation_title(&conv, &row.alias)?;
        }
        Ok(row)
    }

    fn emit_conv_updated(&self, conv: &ConvId) {
        let store = &self.deps.store;
        let Ok(Some(row)) = store.get_conversation(conv) else {
            return;
        };
        let self_user = self
            .deps
            .identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .user_id();
        let summary = conv_summary(store.as_ref(), self_user.as_ref(), &row, false);
        let _ = self
            .deps
            .events
            .send(Event::ConversationUpdated { summary });
    }

    fn peer_view(&self, row: &LsPeerRow, addr: String) -> PeerView {
        PeerView {
            device_id: row.pseudo_device.to_hex(),
            device_short: row.pseudo_device.short(),
            user_id: String::new(),
            display_name: row.alias.clone(),
            device_name: row.device_model.clone().unwrap_or_default(),
            device_type: match row.device_type.as_deref() {
                Some("mobile") => "Phone".into(),
                _ => "Desktop".into(),
            },
            addr,
            path: PathClassDto::Lan,
            is_contact: row.trusted,
            is_mine: false,
        }
    }

    /// 处理发现到的对端：发现阶段只放内存；库里已有的对端（用户接受过 / 主动发过）
    /// 才刷新数据库。
    fn on_peer_seen(&self, peer: LsPeer) {
        let Some(fp) = normalize_fingerprint(&peer.info.fingerprint) else {
            return;
        };
        if fp == self.own_fingerprint || !is_lan(peer.ip) {
            return;
        }
        let now = Instant::now();
        let (write, announce) = {
            let mut g = lock(&self.seen);
            if !g.contains_key(&fp) && g.len() >= MAX_TRACKED_PEERS {
                return;
            }
            let alias = clean_alias(&peer.info.alias, &fp);
            let entry = g.entry(fp.clone()).or_insert_with(|| SeenPeer {
                peer: peer.clone(),
                by_ip: HashMap::new(),
                alias: alias.clone(),
                last_event: now,
                last_db: now - PEER_DB_REFRESH * 2,
                online: false,
            });
            let changed = entry.peer.ip != peer.ip || entry.alias != alias;
            let announce = !entry.online;
            entry.peer = peer.clone();
            if !entry.by_ip.contains_key(&peer.ip) && entry.by_ip.len() >= MAX_IPS_PER_PEER {
                if let Some(oldest) = entry
                    .by_ip
                    .iter()
                    .min_by_key(|(_, (_, at))| *at)
                    .map(|(ip, _)| *ip)
                {
                    entry.by_ip.remove(&oldest);
                }
            }
            entry.by_ip.insert(peer.ip, (peer.clone(), now));
            entry.alias = alias;
            entry.last_event = now;
            entry.online = true;
            let write = changed || announce || entry.last_db.elapsed() >= PEER_DB_REFRESH;
            if write {
                entry.last_db = now;
            }
            (write, announce)
        };
        if !write {
            return;
        }
        let existing = match self.deps.store.get_ls_peer(&fp) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read localsend peer");
                return;
            }
        };
        let row = self.row_from_peer(&peer, &fp, existing.as_ref(), false);
        if existing.is_some() {
            if let Err(e) = self.ensure_peer(&peer, &fp, false) {
                tracing::warn!(error = %e, "failed to refresh localsend peer");
            }
        }
        if announce {
            let addr = std::net::SocketAddr::new(peer.ip, peer.info.port).to_string();
            let _ = self.deps.events.send(Event::PeerOnline {
                peer: self.peer_view(&row, addr),
            });
            self.emit_conv_updated(&ConvId::localsend(&fp));
        }
    }

    /// 超过 `PEER_OFFLINE_AFTER` 没有再出现的对端置为离线。
    fn sweep_offline(&self) {
        let gone: Vec<String> = {
            let mut g = lock(&self.seen);
            g.iter_mut()
                .filter(|(_, s)| s.online && s.last_event.elapsed() >= PEER_OFFLINE_AFTER)
                .map(|(fp, s)| {
                    s.online = false;
                    fp.clone()
                })
                .collect()
        };
        for fp in gone {
            let _ = self.deps.events.send(Event::PeerOffline {
                device: pseudo_device_id(&fp).to_hex(),
            });
            self.emit_conv_updated(&ConvId::localsend(&fp));
        }
    }

    /// 处理接收请求（文字或文件）。
    fn on_request(
        &self,
        peer: LsPeer,
        files: Option<Vec<LsFileMeta>>,
        text: Option<String>,
        reply: oneshot::Sender<LsDecision>,
    ) {
        let (verdict, fp) = self.judge(&peer);
        let Some(fp) = fp.filter(|_| verdict != Verdict::Reject) else {
            let _ = reply.send(LsDecision::Reject);
            return;
        };
        // 未经批准的请求不落库；已信任对端（Auto）的行与会话本来就在库里
        if verdict == Verdict::Auto {
            if let Err(e) = self.ensure_peer(&peer, &fp, false) {
                tracing::warn!(error = %e, "failed to refresh localsend peer");
                let _ = reply.send(LsDecision::Reject);
                return;
            }
            if let Some(t) = text {
                if let Err(e) = self.insert_remote(&fp, MsgBody::Text { text: t }) {
                    tracing::warn!(error = %e, "failed to store localsend text");
                    let _ = reply.send(LsDecision::Reject);
                    return;
                }
            }
            let _ = reply.send(LsDecision::Accept);
            return;
        }
        let alias = clean_alias(&peer.info.alias, &fp);
        let addr_changed = self
            .deps
            .store
            .get_ls_peer(&fp)
            .ok()
            .flatten()
            .is_some_and(|r| r.trusted);
        let (kind, preview) = match (&text, &files) {
            (Some(t), _) => (
                RequestKind::Message,
                RequestPreviewView::Text {
                    text: t.chars().take(TEXT_PREVIEW_CHARS).collect(),
                },
            ),
            (None, Some(f)) => (
                RequestKind::File,
                RequestPreviewView::Files {
                    first_name: f.first().map(|m| m.file_name.clone()).unwrap_or_default(),
                    count: f.len() as u32,
                    total_size: f.iter().map(|m| m.size).sum(),
                },
            ),
            (None, None) => {
                let _ = reply.send(LsDecision::Reject);
                return;
            }
        };
        let id = format!("ls-{}", uuid_simple());
        let view = RequestView {
            request: id.clone(),
            kind,
            from_user: String::new(),
            from_name: alias,
            from_short: fp.chars().take(8).collect(),
            preview,
            created_at: now_millis(),
            source_addr: Some(peer.ip.to_string()),
            addr_changed,
        };
        {
            let mut g = lock(&self.pending);
            g.retain(|_, p| !p.reply.is_closed());
            g.insert(
                id,
                PendingReq {
                    reply,
                    fingerprint: fp,
                    peer,
                    text,
                    view: view.clone(),
                },
            );
        }
        let _ = self
            .deps
            .events
            .send(Event::IncomingRequest { request: view });
    }

    /// 在 LocalSend 会话中插入一条对端发来的消息（伪设备作者、签名全零、已送达）。
    fn insert_remote(&self, fp: &str, body: MsgBody) -> Result<(), CoreError> {
        let author = pseudo_device_id(fp);
        self.insert_as(&ConvId::localsend(fp), author, body, true)
    }

    fn insert_as(
        &self,
        conv: &ConvId,
        author: DeviceId,
        body: MsgBody,
        emit_unread: bool,
    ) -> Result<(), CoreError> {
        let store = &self.deps.store;
        let (seq, conv_seq) = store.next_seqs(&author, conv)?;
        let hlc = self
            .deps
            .clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .now(now_millis());
        let env = Envelope {
            conv: *conv,
            id: MsgId { author, seq },
            conv_seq,
            hlc,
            body,
            enc: None,
            sig: Signature64([0u8; 64]),
        };
        let received_at = now_millis();
        let inserted = store.insert_envelope(&env, MsgStateRow::Delivered, received_at)?;
        if inserted && emit_unread {
            let stored = StoredMessage {
                env,
                state: MsgStateRow::Delivered,
                received_at,
            };
            let view = message_view(store.as_ref(), &self.self_device(), &[], &stored);
            let _ = self.deps.events.send(Event::MessageAdded { msg: view });
            self.emit_conv_updated(conv);
        }
        Ok(())
    }

    /// 更新本机已发消息的状态并通知界面；失败时按原因追加一条本地系统提示。
    fn finish_send(&self, conv: &ConvId, id: &MsgId, result: Result<(), LsSendFailure>) {
        let store = &self.deps.store;
        let state = if result.is_ok() {
            MsgStateRow::Delivered
        } else {
            MsgStateRow::Failed
        };
        if let Err(e) = store.set_message_state(id, state) {
            tracing::warn!(error = %e, "failed to update localsend message state");
        }
        if let Ok(Some(m)) = store.get_message(id) {
            let view = message_view(store.as_ref(), &self.self_device(), &[], &m);
            let _ = self.deps.events.send(Event::MessageUpdated { msg: view });
        }
        let Err(failure) = result else {
            return;
        };
        tracing::info!(?failure, "localsend send failed");
        let Some(row) = self.peer_row_for_conv(conv) else {
            return;
        };
        let event = match failure {
            LsSendFailure::Pin => SystemEvent::LocalSendPin {
                name: row.alias.clone(),
            },
            LsSendFailure::Busy => SystemEvent::LocalSendBusy {
                name: row.alias.clone(),
            },
            _ => return,
        };
        if let Err(e) = self.insert_as(conv, row.pseudo_device, MsgBody::System { event }, true) {
            tracing::warn!(error = %e, "failed to store localsend notice");
        }
    }

    fn peer_row_for_conv(&self, conv: &ConvId) -> Option<LsPeerRow> {
        let fp = lock(&self.seen)
            .keys()
            .find(|fp| ConvId::localsend(fp) == *conv)
            .cloned()?;
        self.deps.store.get_ls_peer(&fp).ok().flatten()
    }

    /// 一个接收会话结束：把已收到的文件合并为一条文件消息。
    fn flush_session(&self, fp: &str, items: Vec<BlobMeta>) {
        if items.is_empty() {
            return;
        }
        if let Err(e) = self.insert_remote(fp, MsgBody::Files { items }) {
            tracing::warn!(error = %e, "failed to store received files");
        }
    }
}

fn uuid_simple() -> String {
    // 请求编号只需在进程内唯一且不可猜：用随机 16 字节的 hex
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 事件循环：快速事件就地处理，文件登记与会话收尾转交 `finisher_loop`（保持先后顺序）。
async fn event_loop(
    inner: Arc<BridgeInner>,
    mut rx: mpsc::Receiver<LsEvent>,
    fin_tx: mpsc::UnboundedSender<LsEvent>,
) {
    while let Some(ev) = rx.recv().await {
        match ev {
            LsEvent::PeerSeen(p) => inner.on_peer_seen(p),
            LsEvent::IncomingFiles {
                peer, files, reply, ..
            } => inner.on_request(peer, Some(files), None, reply),
            LsEvent::TextReceived { peer, text, reply } => {
                inner.on_request(peer, None, Some(text), reply)
            }
            LsEvent::Progress { .. } => {}
            other @ (LsEvent::FileReceived { .. } | LsEvent::SessionFinished { .. }) => {
                let _ = fin_tx.send(other);
            }
        }
    }
}

/// 登记收到的文件并在会话结束时生成文件消息；哈希与缩略图较慢，因此独立成任务。
async fn finisher_loop(inner: Arc<BridgeInner>, mut rx: mpsc::UnboundedReceiver<LsEvent>) {
    // 会话 ID -> (指纹, 已登记文件)
    let mut sessions: HashMap<String, (String, Vec<BlobMeta>, Instant)> = HashMap::new();
    while let Some(ev) = rx.recv().await {
        // 收尾事件丢失时的兜底：过期或超限的会话把已收到的文件先落成消息
        let stale: Vec<String> = sessions
            .iter()
            .filter(|(_, (_, _, at))| at.elapsed() >= SESSION_TTL)
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some((fp, items, _)) = sessions.remove(&id) {
                inner.flush_session(&fp, items);
            }
        }
        match ev {
            LsEvent::FileReceived {
                session,
                peer,
                path,
                ..
            } => {
                let Some(fp) = normalize_fingerprint(&peer.info.fingerprint) else {
                    continue;
                };
                let meta = crate::blob::register::register_local_file(
                    inner.deps.store.clone(),
                    inner.self_device(),
                    inner.deps.thumbnailer.clone(),
                    path,
                    BlobLocalKind::Downloaded,
                    now_millis(),
                    false,
                )
                .await;
                if !sessions.contains_key(&session) && sessions.len() >= MAX_SESSIONS {
                    let oldest = sessions
                        .iter()
                        .min_by_key(|(_, (_, _, at))| *at)
                        .map(|(id, _)| id.clone());
                    if let Some((fp, items, _)) = oldest.and_then(|id| sessions.remove(&id)) {
                        inner.flush_session(&fp, items);
                    }
                }
                match meta {
                    Ok(m) => sessions
                        .entry(session)
                        .or_insert((fp, Vec::new(), Instant::now()))
                        .1
                        .push(m),
                    Err(e) => tracing::warn!(error = %e, "failed to register received file"),
                }
            }
            LsEvent::SessionFinished { session, .. } => {
                if let Some((fp, items, _)) = sessions.remove(&session) {
                    inner.flush_session(&fp, items);
                }
            }
            _ => {}
        }
    }
}

async fn sweep_loop(inner: Arc<BridgeInner>) {
    let mut tick = tokio::time::interval(OFFLINE_SWEEP);
    loop {
        tick.tick().await;
        inner.sweep_offline();
    }
}

/// 服务侧持有的桥槽位：当前运行的桥与上次启动所用配置。
pub(crate) struct LsSlot {
    /// 生产模式：固定端口并启用组播发现；否则（测试等）随机端口、无发现。
    production: bool,
    current: StdMutex<Option<Arc<LocalSendBridge>>>,
    /// 串行化重配置，并记录当前运行所用配置。
    key: tokio::sync::Mutex<Option<LsKey>>,
    /// 服务已关闭：之后的重配置（含后台轮询）不得再拉起桥。
    closed: std::sync::atomic::AtomicBool,
}

impl LsSlot {
    pub(crate) fn new(production: bool) -> Self {
        LsSlot {
            production,
            current: StdMutex::new(None),
            key: tokio::sync::Mutex::new(None),
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 当前运行的桥。
    pub(crate) fn get(&self) -> Option<Arc<LocalSendBridge>> {
        lock(&self.current).clone()
    }

    /// 停止当前的桥（服务关闭时调用）。
    pub(crate) async fn stop(&self) {
        let mut key = self.key.lock().await;
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        let old = lock(&self.current).take();
        if let Some(b) = old {
            b.stop().await;
        }
        *key = None;
    }

    /// 按当前设置启动、重启或停止桥：已初始化且开关开启才运行，配置变化则重启。
    pub(crate) async fn reconfigure(&self, inner: &crate::service::Inner) {
        let mut key = self.key.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let desired = self.desired_key(inner);
        if *key == desired && (desired.is_none() || self.get().is_some()) {
            return;
        }
        let old = lock(&self.current).take();
        if let Some(b) = old {
            b.stop().await;
        }
        *key = None;
        let Some(want) = desired else {
            return;
        };
        let cfg = LsConfig {
            alias: want.alias.clone(),
            device_model: "LanMsg".into(),
            device_type: if cfg!(any(target_os = "android", target_os = "ios")) {
                "mobile".into()
            } else {
                "desktop".into()
            },
            download_dir: want.download_dir.clone(),
            cert_dir: inner.data_dir.join("keys"),
            http_port: want.http_port,
            mcast_port: if want.http_port == 0 {
                LOCALSEND_PORT
            } else {
                want.http_port
            },
            enable_discovery: want.enable_discovery,
        };
        let deps = LsDeps {
            store: inner.store.clone(),
            identity: inner.identity.clone(),
            clock: inner.clock.clone(),
            events: inner.events.clone(),
            thumbnailer: inner.platform.thumbnailer.clone(),
        };
        match LocalSendBridge::start(deps, cfg).await {
            Ok(b) => {
                *lock(&self.current) = Some(Arc::new(b));
                *key = Some(want);
            }
            Err(e) => tracing::warn!(error = %e, "failed to start localsend"),
        }
    }

    fn desired_key(&self, inner: &crate::service::Inner) -> Option<LsKey> {
        if !inner
            .identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_initialized()
        {
            return None;
        }
        let settings = crate::settings::load_settings(&inner.store).ok()?;
        if !settings.localsend_enabled {
            return None;
        }
        let display = inner
            .store
            .get_setting("display_name")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| settings.device_name.clone());
        let alias = if display.trim().is_empty() {
            "LanMsg".to_string()
        } else {
            display
        };
        Some(if self.production {
            LsKey {
                alias,
                download_dir: PathBuf::from(&settings.download_dir).join("LocalSend"),
                http_port: settings.localsend_port,
                enable_discovery: true,
            }
        } else {
            LsKey {
                alias,
                download_dir: inner.data_dir.join("localsend"),
                http_port: 0,
                enable_discovery: false,
            }
        })
    }
}

/// 该会话是否为 LocalSend 会话。
pub(crate) fn is_localsend_conv(store: &Store, conv: &ConvId) -> bool {
    matches!(
        store.get_conversation(conv),
        Ok(Some(ConvRow {
            kind: ConvKindRow::LocalSend,
            ..
        }))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::FileKeyStore;

    #[test]
    fn fingerprints_are_lowercased_and_validated() {
        let fp = "AB".repeat(32);
        assert_eq!(normalize_fingerprint(&fp), Some("ab".repeat(32)));
        assert_eq!(normalize_fingerprint("short"), None);
        assert_eq!(normalize_fingerprint(&"zz".repeat(32)), None);
        assert_eq!(normalize_fingerprint(&"a".repeat(129)), None);
    }

    #[test]
    fn alias_is_cleaned_and_falls_back_to_fingerprint_prefix() {
        assert_eq!(clean_alias("  Pho\nne ", "abcdef0123456789"), "Phone");
        assert_eq!(clean_alias("", "abcdef0123456789"), "abcdef01");
        assert_eq!(clean_alias(&"x".repeat(200), "ff").chars().count(), 64);
    }

    #[test]
    fn alias_drops_bidi_and_zero_width_characters() {
        assert_eq!(clean_alias("a\u{202E}b\u{200B}c\u{FEFF}", "ff"), "abc");
        assert_eq!(
            clean_alias("\u{2066}\u{2069}", "abcdef0123456789"),
            "abcdef01"
        );
    }

    #[test]
    fn send_failures_are_classified_by_status() {
        assert_eq!(classify_failure(&LsError::Status(401)), LsSendFailure::Pin);
        assert_eq!(classify_failure(&LsError::Status(409)), LsSendFailure::Busy);
        assert_eq!(classify_failure(&LsError::Status(429)), LsSendFailure::Busy);
        assert_eq!(
            classify_failure(&LsError::Status(403)),
            LsSendFailure::Declined
        );
        assert!(matches!(
            classify_failure(&LsError::Status(500)),
            LsSendFailure::Other(_)
        ));
    }

    /// 端口被占用：不监听（不能接收），但桥仍可启动（仍可发送）。
    #[tokio::test]
    async fn port_busy_bridge_is_not_listening_but_starts() {
        let busy = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = busy.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let ks = FileKeyStore::new(dir.path().join("keys"));
        let identity = Identity::load_or_create(&ks, &store).unwrap();
        let (events, _rx) = broadcast::channel(16);
        let deps = LsDeps {
            store,
            identity: Arc::new(StdRwLock::new(identity)),
            clock: Arc::new(StdMutex::new(HlcClock::new())),
            events,
            thumbnailer: None,
        };
        let bridge = LocalSendBridge::start(
            deps,
            LsConfig {
                alias: "t".into(),
                device_model: "LanMsg".into(),
                device_type: "desktop".into(),
                download_dir: dir.path().join("dl"),
                cert_dir: dir.path().join("cert"),
                http_port: port,
                mcast_port: 53317,
                enable_discovery: false,
            },
        )
        .await
        .unwrap();
        assert!(!bridge.listening());
        bridge.stop().await;
    }
}
