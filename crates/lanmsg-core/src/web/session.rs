//! Web 临时会话：监听、握手、会话状态、访客会话消息与宿主侧推送。
//!
//! 协议细节（报文格式、seq 滑动窗口、请求认证头）见 P5 计划 Task 12/13/14 与修订附录。
//! 所有会话状态在一把 std Mutex 之内；磁盘读写与哈希在锁外的 spawn_blocking 中完成。

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use rand::RngCore;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, watch};
use zeroize::Zeroize;

use lanmsg_api::types::{Event, WebSessionInfo};
use lanmsg_proto::consts::{WEB_IDLE_TIMEOUT, WEB_PORT};
use lanmsg_proto::ids::{ConvId, DeviceId, MsgId};
use lanmsg_proto::types::{BlobMeta, Envelope, MsgBody, Signature64};

use crate::blob::register::register_local_file;
use crate::blob::transfer::{sanitize_component, unique_path};
use crate::error::CoreError;
use crate::hlc::HlcClock;
use crate::identity::Identity;
use crate::msg::Messaging;
use crate::net::path::{classify, local_ifaces, LocalIface};
use crate::net::ConnManager;
use crate::platform::ThumbnailProvider;
use crate::service::now_millis;
use crate::service::views::{conv_summary, message_view};
use crate::store::{BlobLocalKind, ConvKindRow, ConvRow, MsgStateRow, Store, StoredMessage};
use crate::web::crypto::{host_respond, CodeGuard, SessionKeys, WebMode, WebSecrets};
use crate::web::transfer::{
    chunk_count, frame_chunk, parse_chunk, read_chunk, write_chunk, UploadState, CHUNK_SIZE,
    MAX_FILE_SIZE,
};

/// 防重放滑动窗口大小。
const SEQ_WINDOW: u64 = 4096;
/// seq 上限（JS 安全整数）。
const SEQ_MAX: u64 = 1 << 53;
/// 未完成握手（尚未 confirm）的会话上限与有效期。
const MAX_PENDING: usize = 5;
const PENDING_TTL: Duration = Duration::from_secs(30);
/// confirm 之后等待宿主允许的最长时间。
const AWAITING_TTL: Duration = Duration::from_secs(120);
/// 被拒绝的会话保留多久（让 wait 能读到结果）。
const DENIED_TTL: Duration = Duration::from_secs(30);
/// 单 IP 每分钟 hello 上限。
const HELLO_PER_MINUTE: usize = 10;
/// 页面文字消息上限（字节）。
const MAX_TEXT_BYTES: usize = 64 * 1024;
/// 单会话同时存在的上传数上限。
const MAX_UPLOADS: usize = 256;
/// 访客上传的落盘子目录（英文，界面不显示）。
const UPLOAD_DIR_NAME: &str = "Web uploads";

/// Web 层错误，由 server.rs 映射为 HTTP 状态码。
#[derive(Debug)]
pub enum WebErr {
    /// 403：会话缺失、阶段不对、认证失败或重放。
    Forbidden,
    /// 403 `{"error":"bad_mac"}`。
    BadMac,
    NotFound,
    BadRequest,
    /// 429。
    TooMany,
    /// 409，body 为已封包的 `{"missing":[..]}`。
    Conflict(Vec<u8>),
    Internal,
}

impl From<CoreError> for WebErr {
    fn from(e: CoreError) -> Self {
        tracing::warn!(error = %e, "web request failed");
        WebErr::Internal
    }
}

/// 防重放滑动窗口：拒绝非法值、已见值与落后最大值一个窗口以上的值。
#[derive(Default)]
pub(crate) struct SeqWindow {
    max: u64,
    seen: HashSet<u64>,
}

impl SeqWindow {
    pub(crate) fn accept(&mut self, s: u64) -> bool {
        if s == 0 || s > SEQ_MAX {
            return false;
        }
        if s + SEQ_WINDOW <= self.max || self.seen.contains(&s) {
            return false;
        }
        self.seen.insert(s);
        self.max = self.max.max(s);
        if self.seen.len() as u64 > SEQ_WINDOW * 2 {
            let max = self.max;
            self.seen.retain(|v| *v + SEQ_WINDOW > max);
        }
        true
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// hello 之后、confirm 之前。
    Pending,
    /// confirm 通过，等宿主点允许。
    Awaiting,
    Allowed,
    Denied,
}

/// `wait` 长轮询看到的状态。
#[derive(Clone, Debug)]
pub(crate) enum WaitStatus {
    Pending,
    Allowed([u8; 32]),
    Denied,
}

/// 发往 WebSocket 连接的指令。
pub(crate) enum WsOut {
    Frame(Vec<u8>),
    Close,
}

#[derive(Clone)]
struct FileEntry {
    path: PathBuf,
    name: String,
    size: u64,
    mime: String,
}

#[derive(Clone)]
enum Content {
    Text(String),
    File { fid: String, entry: FileEntry },
}

/// 会话消息日志条目（WebSocket 重连时整体重放）。
#[derive(Clone)]
struct LogEntry {
    id: String,
    from_page: bool,
    ts: i64,
    content: Content,
}

struct Session {
    phase: Phase,
    mode: WebMode,
    keys: Arc<SessionKeys>,
    created: Instant,
    last_activity: Instant,
    ip: IpAddr,
    user_agent: String,
    host_seq: u64,
    page_window: SeqWindow,
    status_tx: watch::Sender<WaitStatus>,
    conv: Option<ConvId>,
    pseudo: Option<DeviceId>,
    uploads: HashMap<String, UploadState>,
    client_keys: HashMap<String, String>,
    files: HashMap<String, FileEntry>,
    log: Vec<LogEntry>,
    ws: Option<(u64, mpsc::UnboundedSender<WsOut>)>,
    next_conn: u64,
}

struct Run {
    secrets: WebSecrets,
    guard: CodeGuard,
    port: u16,
    shutdown: watch::Sender<bool>,
    sessions: HashMap<String, Session>,
    hello_log: HashMap<IpAddr, VecDeque<Instant>>,
    /// 最近一次存在会话的时刻：长时间无会话则自动停止。
    idle_since: Instant,
}

/// WebHost 依赖的服务内部组件。
pub struct WebDeps {
    pub store: Arc<Store>,
    pub identity: Arc<RwLock<Identity>>,
    pub conns: ConnManager,
    pub events: broadcast::Sender<Event>,
    pub clock: Arc<Mutex<HlcClock>>,
    pub msg: Messaging,
    pub thumbnailer: Option<Arc<dyn ThumbnailProvider>>,
}

/// 共享状态（HTTP 处理函数持有 `Arc<Shared>`）。
pub(crate) struct Shared {
    deps: WebDeps,
    idle_timeout_ms: AtomicU64,
    state: Mutex<Option<Run>>,
}

/// 网页传输宿主：对外只暴露启动 / 停止 / 应答 / 向访客推送。
pub struct WebHost {
    shared: Arc<Shared>,
}

/// hello 应答所需字段。
pub(crate) struct HelloOut {
    pub session: String,
    pub host_pk: [u8; 32],
    pub host_nonce: [u8; 16],
    pub host_name: String,
    pub scrypt_salt: [u8; 16],
}

/// 访客伪设备：blake3("lanmsg-guest" || conv)。
pub(crate) fn guest_device(conv: &ConvId) -> DeviceId {
    let mut h = blake3::Hasher::new();
    h.update(b"lanmsg-guest");
    h.update(&conv.0);
    DeviceId(*h.finalize().as_bytes())
}

/// 来源地址是否属于局域网（回环视为局域网）。
pub(crate) fn is_lan(ip: IpAddr, ifaces: &[LocalIface]) -> bool {
    classify(ip, ifaces, false) == lanmsg_proto::types::PathClass::Lan
}

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut b);
    HEXLOWER.encode(&b)
}

fn b64u_array<const N: usize>(s: &str) -> Option<[u8; N]> {
    BASE64URL_NOPAD.decode(s.as_bytes()).ok()?.try_into().ok()
}

/// 分配宿主到页面方向的下一个 seq。
fn alloc_seq(sess: &mut Session) -> Result<u64, WebErr> {
    sess.host_seq += 1;
    if sess.host_seq > SEQ_MAX {
        return Err(WebErr::Forbidden);
    }
    Ok(sess.host_seq)
}

/// 给 JSON 对象加 `seq` 并封包（宿主 -> 页面）。
fn seal_value(sess: &mut Session, mut v: Value) -> Result<Vec<u8>, WebErr> {
    let seq = alloc_seq(sess)?;
    v["seq"] = json!(seq);
    let bytes = serde_json::to_vec(&v).map_err(|_| WebErr::Internal)?;
    Ok(sess.keys.seal_to_page(&bytes))
}

fn entry_json(e: &LogEntry) -> Value {
    let from = if e.from_page { "page" } else { "host" };
    match &e.content {
        Content::Text(t) => {
            json!({"t":"msg","id":e.id,"from":from,"kind":"text","text":t,"ts":e.ts})
        }
        Content::File { fid, entry } => json!({
            "t":"msg","id":e.id,"from":from,"kind":"file","ts":e.ts,
            "file":{"fid":fid,"name":entry.name,"size":entry.size,"mime":entry.mime,
                    "chunks":chunk_count(entry.size)}
        }),
    }
}

/// 记录并（若 WebSocket 在线）推送一条消息。
fn push_entry(sess: &mut Session, entry: LogEntry) {
    let v = entry_json(&entry);
    sess.log.push(entry);
    send_ws_value(sess, v);
}

fn send_ws_value(sess: &mut Session, v: Value) {
    if sess.ws.is_none() {
        return;
    }
    if let Ok(frame) = seal_value(sess, v) {
        if let Some((_, tx)) = &sess.ws {
            let _ = tx.send(WsOut::Frame(frame));
        }
    }
}

/// 候选访问地址：普通网卡的私有 IPv4 优先，其次隧道网卡，其余非回环 IPv4 殿后。
fn candidate_ips() -> Vec<Ipv4Addr> {
    let mut v: Vec<(bool, bool, Ipv4Addr)> = local_ifaces()
        .into_iter()
        .filter_map(|i| match i.addr {
            IpAddr::V4(a) if !a.is_loopback() => Some((i.tunnel.is_some(), !a.is_private(), a)),
            _ => None,
        })
        .collect();
    v.sort_by_key(|(t, p, _)| (*p, *t));
    let mut out: Vec<Ipv4Addr> = Vec::new();
    for (_, _, a) in v {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    if out.is_empty() {
        out.push(Ipv4Addr::LOCALHOST);
    }
    out
}

fn build_info(secrets: &WebSecrets, port: u16) -> WebSessionInfo {
    let key = BASE64URL_NOPAD.encode(&secrets.url_key);
    let urls: Vec<String> = candidate_ips()
        .into_iter()
        .map(|ip| format!("http://{ip}:{port}/#k={key}"))
        .collect();
    let url = urls.first().cloned().unwrap_or_default();
    WebSessionInfo {
        qr_uri: url.clone(),
        url,
        code: secrets.code.clone(),
        urls: Some(urls),
    }
}

impl WebHost {
    pub fn new(deps: WebDeps) -> Self {
        WebHost {
            shared: Arc::new(Shared {
                deps,
                idle_timeout_ms: AtomicU64::new(WEB_IDLE_TIMEOUT.as_millis() as u64),
                state: Mutex::new(None),
            }),
        }
    }

    /// 测试钩子：缩短会话空闲超时。
    #[doc(hidden)]
    pub fn set_idle_timeout(&self, d: Duration) {
        self.shared
            .idle_timeout_ms
            .store(d.as_millis() as u64, Ordering::SeqCst);
    }

    /// 开启网页传输：监听 `WEB_PORT`（被占用则系统分配），已开启则返回当前信息。
    pub async fn start(&self) -> Result<WebSessionInfo, CoreError> {
        {
            let g = self.shared.lock();
            if let Some(run) = g.as_ref() {
                return Ok(build_info(&run.secrets, run.port));
            }
        }
        let listener = match tokio::net::TcpListener::bind(("0.0.0.0", WEB_PORT)).await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind(("0.0.0.0", 0)).await?,
        };
        let port = listener.local_addr()?.port();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let secrets = WebSecrets::generate();
        let info = build_info(&secrets, port);
        {
            let mut g = self.shared.lock();
            if g.is_some() {
                // 并发的另一次 start 已先完成。
                return Ok(info_of(&g));
            }
            *g = Some(Run {
                secrets,
                guard: CodeGuard::default(),
                port,
                shutdown,
                sessions: HashMap::new(),
                hello_log: HashMap::new(),
                idle_since: Instant::now(),
            });
        }
        // 上次异常退出遗留的未结束访客记录。
        self.shared.close_stale_guests();
        let app = crate::web::server::router(self.shared.clone());
        tokio::spawn(crate::web::server::serve(listener, app, shutdown_rx));
        tokio::spawn(self.shared.clone().sweeper());
        Ok(info)
    }

    /// 结束全部会话并停止监听；密钥与配对码作废。
    pub async fn stop(&self) {
        self.shared.stop().await;
    }

    /// 允许 / 拒绝一个等待中的浏览器。
    pub fn respond(&self, session: &str, allow: bool) -> Result<(), CoreError> {
        self.shared.respond(session, allow)
    }

    /// 该会话是否为访客（网页）会话。
    pub fn is_guest_conv(&self, conv: &ConvId) -> bool {
        matches!(
            self.shared.deps.store.get_conversation(conv),
            Ok(Some(ConvRow {
                kind: ConvKindRow::Guest,
                ..
            }))
        )
    }

    /// 宿主向访客会话发文字：生成本机签名消息，推送给页面，成功后置 Delivered。
    pub async fn send_text(&self, conv: &ConvId, text: &str) -> Result<MsgId, CoreError> {
        let sid = self.shared.active_session_of(conv)?;
        let id = self
            .shared
            .deps
            .msg
            .send(
                conv,
                MsgBody::Text {
                    text: text.to_string(),
                },
            )
            .await?;
        self.shared.push_host(
            &sid,
            vec![LogEntryNew {
                id: id.to_string(),
                content: ContentNew::Text(text.to_string()),
            }],
        )?;
        self.shared.mark_delivered(&id);
        Ok(id)
    }

    /// 宿主向访客会话发文件：登记为本机持有的 Blob，推送文件消息，页面按分块下载。
    pub async fn send_files(&self, conv: &ConvId, paths: &[PathBuf]) -> Result<MsgId, CoreError> {
        let sid = self.shared.active_session_of(conv)?;
        let d = &self.shared.deps;
        let self_device = d
            .identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .device_id();
        let mut items: Vec<BlobMeta> = Vec::new();
        for p in paths {
            let meta = register_local_file(
                d.store.clone(),
                self_device,
                d.thumbnailer.clone(),
                p.clone(),
                BlobLocalKind::Source,
                now_millis(),
            )
            .await?;
            items.push(meta);
        }
        if items.is_empty() {
            return Err(CoreError::InvalidArgument("no files".into()));
        }
        let id = d
            .msg
            .send(
                conv,
                MsgBody::Files {
                    items: items.clone(),
                },
            )
            .await?;
        let multi = items.len() > 1;
        let news = items
            .iter()
            .zip(paths)
            .enumerate()
            .map(|(i, (m, p))| LogEntryNew {
                id: if multi {
                    format!("{id}#{i}")
                } else {
                    id.to_string()
                },
                content: ContentNew::File(FileEntry {
                    path: p.clone(),
                    name: m.name.clone(),
                    size: m.size,
                    mime: m.mime.clone(),
                }),
            })
            .collect();
        self.shared.push_host(&sid, news)?;
        self.shared.mark_delivered(&id);
        Ok(id)
    }
}

fn info_of(g: &Option<Run>) -> WebSessionInfo {
    match g {
        Some(run) => build_info(&run.secrets, run.port),
        None => WebSessionInfo {
            url: String::new(),
            code: String::new(),
            qr_uri: String::new(),
            urls: None,
        },
    }
}

/// 待推送的宿主消息（尚未分配 fid）。
struct LogEntryNew {
    id: String,
    content: ContentNew,
}

enum ContentNew {
    Text(String),
    File(FileEntry),
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Run>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn idle_timeout(&self) -> Duration {
        Duration::from_millis(self.idle_timeout_ms.load(Ordering::SeqCst))
    }

    fn emit(&self, e: Event) {
        let _ = self.deps.events.send(e);
    }

    fn close_stale_guests(&self) {
        if let Ok(conn) = self.deps.store.conn() {
            let _ = conn.execute(
                "UPDATE web_guests SET ended_at = started_at WHERE ended_at IS NULL",
                [],
            );
        }
    }

    fn host_name(&self) -> String {
        let s = &self.deps.store;
        s.get_setting("device_name")
            .ok()
            .flatten()
            .filter(|n| !n.is_empty())
            .or_else(|| s.get_setting("display_name").ok().flatten())
            .unwrap_or_default()
    }

    // ---------- 握手 ----------

    pub(crate) fn hello(
        &self,
        ip: IpAddr,
        user_agent: &str,
        mode: &str,
        page_pk: &str,
        page_nonce: &str,
    ) -> Result<HelloOut, WebErr> {
        let mode = match mode {
            "key" => WebMode::Key,
            "code" => WebMode::Code,
            _ => return Err(WebErr::BadRequest),
        };
        let pk: [u8; 32] = b64u_array(page_pk).ok_or(WebErr::BadRequest)?;
        let nonce: [u8; 16] = b64u_array(page_nonce).ok_or(WebErr::BadRequest)?;
        let host_name = self.host_name();
        let mut g = self.lock();
        let run = g.as_mut().ok_or(WebErr::NotFound)?;

        // 单 IP 每分钟 hello 限速。
        let now = Instant::now();
        let hits = run.hello_log.entry(ip).or_default();
        while hits
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            hits.pop_front();
        }
        if hits.len() >= HELLO_PER_MINUTE {
            return Err(WebErr::TooMany);
        }
        hits.push_back(now);

        // 未完成握手的会话最多 MAX_PENDING 个：超出时淘汰最旧的。
        let mut pending: Vec<(Instant, String)> = run
            .sessions
            .iter()
            .filter(|(_, s)| s.phase == Phase::Pending)
            .map(|(k, s)| (s.created, k.clone()))
            .collect();
        pending.sort();
        while pending.len() >= MAX_PENDING {
            let (_, k) = pending.remove(0);
            run.sessions.remove(&k);
        }

        let mut auth = run.secrets.auth_key(mode);
        let hs = host_respond(mode, &auth, &pk, &nonce);
        auth.zeroize();
        let hs = hs.map_err(|_| WebErr::BadRequest)?;
        let out = HelloOut {
            session: hs.session.clone(),
            host_pk: hs.host_pk,
            host_nonce: hs.host_nonce,
            host_name,
            scrypt_salt: run.secrets.scrypt_salt,
        };
        let (status_tx, _) = watch::channel(WaitStatus::Pending);
        run.sessions.insert(
            hs.session.clone(),
            Session {
                phase: Phase::Pending,
                mode,
                keys: Arc::new(hs.into_keys()),
                created: now,
                last_activity: now,
                ip,
                user_agent: user_agent.chars().take(256).collect(),
                host_seq: 0,
                page_window: SeqWindow::default(),
                status_tx,
                conv: None,
                pseudo: None,
                uploads: HashMap::new(),
                client_keys: HashMap::new(),
                files: HashMap::new(),
                log: Vec::new(),
                ws: None,
                next_conn: 0,
            },
        );
        Ok(out)
    }

    pub(crate) fn confirm(&self, sid: &str, mac_b64: &str) -> Result<(), WebErr> {
        let mac = BASE64URL_NOPAD
            .decode(mac_b64.as_bytes())
            .map_err(|_| WebErr::BadRequest)?;
        let mut g = self.lock();
        let run = g.as_mut().ok_or(WebErr::NotFound)?;
        let sess = run.sessions.get_mut(sid).ok_or(WebErr::NotFound)?;
        if sess.phase != Phase::Pending || sess.created.elapsed() > PENDING_TTL {
            run.sessions.remove(sid);
            return Err(WebErr::NotFound);
        }
        if sess.keys.verify_page_mac(&mac) {
            let mode = sess.mode;
            sess.phase = Phase::Awaiting;
            sess.last_activity = Instant::now();
            let ev = Event::WebRequest {
                request: sid.to_string(),
                user_agent: sess.user_agent.clone(),
                ip: sess.ip.to_string(),
            };
            // 只在页面 MAC 校验通过后清零失败计数（denied、超时不清零）。
            if mode == WebMode::Code {
                run.guard.record_success();
            }
            self.emit(ev);
            return Ok(());
        }
        let mode = sess.mode;
        run.sessions.remove(sid);
        if mode == WebMode::Code && run.guard.record_failure() {
            run.secrets.rotate_code();
            let info = build_info(&run.secrets, run.port);
            self.emit(Event::WebSessionChanged { info: Some(info) });
        }
        Err(WebErr::BadMac)
    }

    /// 取得长轮询用的状态订阅。
    pub(crate) fn wait_rx(&self, sid: &str) -> Result<watch::Receiver<WaitStatus>, WebErr> {
        let g = self.lock();
        let run = g.as_ref().ok_or(WebErr::NotFound)?;
        let sess = run.sessions.get(sid).ok_or(WebErr::NotFound)?;
        if sess.phase == Phase::Pending {
            return Err(WebErr::Forbidden);
        }
        Ok(sess.status_tx.subscribe())
    }

    fn respond(&self, sid: &str, allow: bool) -> Result<(), CoreError> {
        let mut g = self.lock();
        let run = g
            .as_mut()
            .ok_or_else(|| CoreError::NotFound("web transfer is not running".into()))?;
        let sess = run
            .sessions
            .get_mut(sid)
            .filter(|s| s.phase == Phase::Awaiting)
            .ok_or_else(|| CoreError::NotFound(format!("web request not found: {sid}")))?;
        if !allow {
            sess.phase = Phase::Denied;
            sess.last_activity = Instant::now();
            sess.status_tx.send_replace(WaitStatus::Denied);
            return Ok(());
        }
        let conv = ConvId::random();
        let pseudo = guest_device(&conv);
        let now = now_millis();
        // 访客会话标题留空，界面按会话类型本地化。
        let row = ConvRow {
            conv,
            kind: ConvKindRow::Guest,
            peer_user: None,
            title: None,
            created_at: now,
        };
        self.deps.store.ensure_conversation(&row)?;
        {
            let conn = self
                .deps
                .store
                .conn()
                .map_err(|e| CoreError::Proto(e.to_string()))?;
            conn.execute(
                "INSERT INTO web_guests(conv_id, pseudo_device, user_agent, ip, started_at)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    &conv.0[..],
                    &pseudo.0[..],
                    sess.user_agent,
                    sess.ip.to_string(),
                    now
                ],
            )
            .map_err(crate::store::StoreError::from)?;
        }
        sess.conv = Some(conv);
        sess.pseudo = Some(pseudo);
        sess.phase = Phase::Allowed;
        sess.last_activity = Instant::now();
        let mac = sess.keys.host_mac();
        sess.status_tx.send_replace(WaitStatus::Allowed(mac));
        drop(g);
        self.emit_conv_updated(&row);
        Ok(())
    }

    fn self_user(&self) -> Option<lanmsg_proto::ids::UserId> {
        self.deps
            .identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .user_id()
    }

    fn emit_conv_updated(&self, row: &ConvRow) {
        let self_user = self.self_user();
        let summary = conv_summary(self.deps.store.as_ref(), self_user.as_ref(), row, false);
        self.emit(Event::ConversationUpdated { summary });
    }

    // ---------- 已允许会话的通用入口 ----------

    /// 取已允许会话的密钥并刷新活动时间。
    pub(crate) fn authed(&self, sid: &str) -> Result<Arc<SessionKeys>, WebErr> {
        let mut g = self.lock();
        let sess = allowed_mut(&mut g, sid)?;
        sess.last_activity = Instant::now();
        Ok(sess.keys.clone())
    }

    /// 解密页面发来的 JSON 并检查 seq。
    pub(crate) fn open_json(&self, sid: &str, ct: &[u8]) -> Result<Value, WebErr> {
        let keys = self.authed(sid)?;
        let pt = keys.open_from_page(ct).map_err(|_| WebErr::BadRequest)?;
        let v: Value = serde_json::from_slice(&pt).map_err(|_| WebErr::BadRequest)?;
        let seq = v
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or(WebErr::BadRequest)?;
        let mut g = self.lock();
        let sess = allowed_mut(&mut g, sid)?;
        if !sess.page_window.accept(seq) {
            return Err(WebErr::Forbidden);
        }
        Ok(v)
    }

    /// 校验 `X-LanMsg-Req` 认证头：op、fid、idx 与路径一致且 seq 未重放。
    pub(crate) fn check_req(
        &self,
        sid: &str,
        header: &str,
        op: &str,
        fid: &str,
        idx: Option<u32>,
    ) -> Result<(), WebErr> {
        let ct = BASE64URL_NOPAD
            .decode(header.as_bytes())
            .map_err(|_| WebErr::Forbidden)?;
        let v = self.open_json(sid, &ct).map_err(|e| match e {
            WebErr::BadRequest => WebErr::Forbidden,
            other => other,
        })?;
        let ok = v.get("op").and_then(Value::as_str) == Some(op)
            && v.get("fid").and_then(Value::as_str) == Some(fid)
            && match idx {
                Some(i) => v.get("idx").and_then(Value::as_u64) == Some(i as u64),
                None => true,
            };
        if ok {
            Ok(())
        } else {
            Err(WebErr::Forbidden)
        }
    }

    // ---------- 上传 ----------

    pub(crate) async fn upload_init(&self, sid: &str, body: &[u8]) -> Result<Vec<u8>, WebErr> {
        let v = self.open_json(sid, body)?;
        let client_key = v.get("client_key").and_then(Value::as_str).unwrap_or("");
        let name = v.get("name").and_then(Value::as_str).unwrap_or("");
        let size = v
            .get("size")
            .and_then(Value::as_u64)
            .ok_or(WebErr::BadRequest)?;
        let mime = v
            .get("mime")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream");
        if client_key.is_empty() || client_key.len() > 64 || name.is_empty() || size > MAX_FILE_SIZE
        {
            return Err(WebErr::BadRequest);
        }
        // 同一 client_key 重连后续传：直接返回已有进度。
        {
            let mut g = self.lock();
            let sess = allowed_mut(&mut g, sid)?;
            if let Some(fid) = sess.client_keys.get(client_key).cloned() {
                if let Some(u) = sess.uploads.get(&fid) {
                    if u.size != size {
                        return Err(WebErr::BadRequest);
                    }
                    let resp = json!({"fid":u.fid,"chunk_size":u.chunk_size,
                        "have":u.have_list(),"client_key":client_key});
                    return seal_value(sess, resp);
                }
            }
            if sess.uploads.len() >= MAX_UPLOADS {
                return Err(WebErr::TooMany);
            }
        }
        let settings = crate::settings::load_settings(&self.deps.store)?;
        let dir = PathBuf::from(settings.download_dir).join(UPLOAD_DIR_NAME);
        let fid = random_hex(16);
        let st = UploadState::new(
            fid.clone(),
            client_key.to_string(),
            name.to_string(),
            mime.to_string(),
            size,
            dir.clone(),
        );
        let part = st.path_part.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&dir)?;
            std::fs::File::create(&part).map(|_| ())
        })
        .await
        .map_err(|_| WebErr::Internal)?
        .map_err(|e| {
            tracing::warn!(error = %e, "creating upload part file failed");
            WebErr::Internal
        })?;
        let mut g = self.lock();
        let sess = allowed_mut(&mut g, sid)?;
        let resp = json!({"fid":fid,"chunk_size":st.chunk_size,"have":st.have_list(),
            "client_key":client_key});
        sess.client_keys.insert(client_key.to_string(), fid.clone());
        sess.uploads.insert(fid, st);
        seal_value(sess, resp)
    }

    pub(crate) async fn put_chunk(
        &self,
        sid: &str,
        fid: &str,
        idx: u32,
        body: &[u8],
    ) -> Result<(), WebErr> {
        let keys = self.authed(sid)?;
        let pt = keys.open_from_page(body).map_err(|_| WebErr::BadRequest)?;
        let p = parse_chunk(&pt).ok_or(WebErr::BadRequest)?;
        // 明文里的 fid / chunk_idx 必须与 URL 一致，防止分块被换位或跨文件替换。
        if p.fid != fid || p.idx != idx {
            return Err(WebErr::BadRequest);
        }
        let path = {
            let mut g = self.lock();
            let sess = allowed_mut(&mut g, sid)?;
            if !sess.page_window.accept(p.seq) {
                return Err(WebErr::Forbidden);
            }
            let u = sess.uploads.get(fid).ok_or(WebErr::NotFound)?;
            if u.finished.is_some() || idx as usize >= u.have.len() {
                return Err(WebErr::BadRequest);
            }
            if p.data.len() != crate::web::transfer::chunk_len(u.size, idx) {
                return Err(WebErr::BadRequest);
            }
            u.path_part.clone()
        };
        let offset = idx as u64 * CHUNK_SIZE as u64;
        let data = p.data.to_vec();
        tokio::task::spawn_blocking(move || write_chunk(&path, offset, &data))
            .await
            .map_err(|_| WebErr::Internal)?
            .map_err(|e| {
                tracing::warn!(error = %e, "writing upload chunk failed");
                WebErr::Internal
            })?;
        let mut g = self.lock();
        let sess = allowed_mut(&mut g, sid)?;
        if let Some(u) = sess.uploads.get_mut(fid) {
            if let Some(b) = u.have.get_mut(idx as usize) {
                *b = true;
            }
        }
        Ok(())
    }

    pub(crate) async fn finish(&self, sid: &str, fid: &str) -> Result<Vec<u8>, WebErr> {
        // 取快照：缺块则 409；已完成则幂等返回。
        let (name, mime, size, part, dir, conv, pseudo) = {
            let mut g = self.lock();
            let sess = allowed_mut(&mut g, sid)?;
            let u = sess.uploads.get(fid).ok_or(WebErr::NotFound)?;
            if let Some(msg) = u.finished.clone() {
                let resp = json!({"msg":msg,"fid":fid});
                return seal_value(sess, resp);
            }
            let missing = u.missing_list();
            if !missing.is_empty() {
                let resp = json!({"missing":missing,"fid":fid});
                let body = seal_value(sess, resp)?;
                return Err(WebErr::Conflict(body));
            }
            (
                u.name.clone(),
                u.mime.clone(),
                u.size,
                u.path_part.clone(),
                u.dir.clone(),
                sess.conv.ok_or(WebErr::Internal)?,
                sess.pseudo.ok_or(WebErr::Internal)?,
            )
        };
        let dest = unique_path(&dir.join(sanitize_component(&name)));
        let (p2, d2) = (part.clone(), dest.clone());
        tokio::task::spawn_blocking(move || {
            if std::fs::metadata(&p2)?.len() != size {
                return Err(std::io::Error::other("part size mismatch"));
            }
            std::fs::rename(&p2, &d2)
        })
        .await
        .map_err(|_| WebErr::Internal)?
        .map_err(|e| {
            tracing::warn!(error = %e, "finalizing upload failed");
            WebErr::Internal
        })?;
        let d = &self.deps;
        let self_device = d
            .identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .device_id();
        let meta = register_local_file(
            d.store.clone(),
            self_device,
            d.thumbnailer.clone(),
            dest.clone(),
            BlobLocalKind::Downloaded,
            now_millis(),
        )
        .await?;
        let id = self.insert_guest_message(&conv, &pseudo, MsgBody::Files { items: vec![meta] })?;
        let entry = FileEntry {
            path: dest,
            name,
            size,
            mime,
        };
        let msg_id = id.to_string();
        let mut g = self.lock();
        let sess = allowed_mut(&mut g, sid)?;
        if let Some(u) = sess.uploads.get_mut(fid) {
            u.finished = Some(msg_id.clone());
        }
        sess.files.insert(fid.to_string(), entry.clone());
        push_entry(
            sess,
            LogEntry {
                id: msg_id.clone(),
                from_page: true,
                ts: now_millis(),
                content: Content::File {
                    fid: fid.to_string(),
                    entry,
                },
            },
        );
        seal_value(sess, json!({"msg":msg_id,"fid":fid}))
    }

    // ---------- 下载 ----------

    pub(crate) async fn download(&self, sid: &str, fid: &str, idx: u32) -> Result<Vec<u8>, WebErr> {
        let (entry, seq) = {
            let mut g = self.lock();
            let sess = allowed_mut(&mut g, sid)?;
            let e = sess.files.get(fid).cloned().ok_or(WebErr::NotFound)?;
            if idx >= chunk_count(e.size) {
                return Err(WebErr::NotFound);
            }
            // 重试下载必须重新封包并换新 seq，不缓存密文。
            (e, alloc_seq(sess)?)
        };
        let (path, size) = (entry.path.clone(), entry.size);
        let data = tokio::task::spawn_blocking(move || read_chunk(&path, size, idx))
            .await
            .map_err(|_| WebErr::Internal)?
            .map_err(|e| {
                tracing::warn!(error = %e, "reading download chunk failed");
                WebErr::Internal
            })?;
        let keys = self.authed(sid)?;
        Ok(keys.seal_to_page(&frame_chunk(seq, fid, idx, &data)))
    }

    // ---------- WebSocket ----------

    /// 接入 WebSocket：先发 welcome，再重放全部历史消息。新连接取代旧连接。
    pub(crate) fn ws_attach(
        &self,
        sid: &str,
    ) -> Result<(u64, mpsc::UnboundedReceiver<WsOut>, watch::Receiver<bool>), WebErr> {
        let host_name = self.host_name();
        let mut g = self.lock();
        let run = g.as_mut().ok_or(WebErr::NotFound)?;
        let shutdown = run.shutdown.subscribe();
        let sess = allowed_mut(&mut g, sid)?;
        sess.last_activity = Instant::now();
        if let Some((_, old)) = sess.ws.take() {
            let _ = old.send(WsOut::Close);
        }
        sess.next_conn += 1;
        let conn = sess.next_conn;
        let (tx, rx) = mpsc::unbounded_channel();
        sess.ws = Some((conn, tx));
        send_ws_value(sess, json!({"t":"welcome","host_name":host_name}));
        let log = sess.log.clone();
        for e in &log {
            send_ws_value(sess, entry_json(e));
        }
        Ok((conn, rx, shutdown))
    }

    pub(crate) fn ws_detach(&self, sid: &str, conn: u64) {
        let mut g = self.lock();
        if let Some(sess) = g.as_mut().and_then(|r| r.sessions.get_mut(sid)) {
            if sess.ws.as_ref().is_some_and(|(c, _)| *c == conn) {
                sess.ws = None;
            }
        }
    }

    /// 处理页面经 WebSocket 发来的一帧；认证或重放失败返回错误（调用方关闭连接）。
    pub(crate) fn ws_incoming(&self, sid: &str, frame: &[u8]) -> Result<(), WebErr> {
        let v = self.open_json(sid, frame)?;
        match v.get("t").and_then(Value::as_str) {
            Some("text") => {
                let text = v.get("text").and_then(Value::as_str).unwrap_or("");
                let cid = v.get("cid").and_then(Value::as_str).unwrap_or("");
                if text.is_empty() || text.len() > MAX_TEXT_BYTES {
                    return Ok(());
                }
                self.page_text(sid, cid, text)
            }
            // ping 只刷新活动时间（open_json 已刷新）。
            _ => Ok(()),
        }
    }

    fn page_text(&self, sid: &str, cid: &str, text: &str) -> Result<(), WebErr> {
        let (conv, pseudo) = {
            let mut g = self.lock();
            let sess = allowed_mut(&mut g, sid)?;
            (
                sess.conv.ok_or(WebErr::Internal)?,
                sess.pseudo.ok_or(WebErr::Internal)?,
            )
        };
        let id = self.insert_guest_message(
            &conv,
            &pseudo,
            MsgBody::Text {
                text: text.to_string(),
            },
        )?;
        let mut g = self.lock();
        let sess = allowed_mut(&mut g, sid)?;
        let id = id.to_string();
        push_entry(
            sess,
            LogEntry {
                id: id.clone(),
                from_page: true,
                ts: now_millis(),
                content: Content::Text(text.to_string()),
            },
        );
        send_ws_value(sess, json!({"t":"ack","cid":cid,"id":id}));
        Ok(())
    }

    // ---------- 访客会话消息 ----------

    /// 以访客伪设备为作者插入本地消息（签名全零、Delivered），并发事件。
    fn insert_guest_message(
        &self,
        conv: &ConvId,
        pseudo: &DeviceId,
        body: MsgBody,
    ) -> Result<MsgId, CoreError> {
        let d = &self.deps;
        let (seq, conv_seq) = d.store.next_seqs(pseudo, conv)?;
        let hlc = d
            .clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .now(now_millis());
        let env = Envelope {
            conv: *conv,
            id: MsgId {
                author: *pseudo,
                seq,
            },
            conv_seq,
            hlc,
            body,
            enc: None,
            sig: Signature64([0u8; 64]),
        };
        let received_at = now_millis();
        d.store
            .insert_envelope(&env, MsgStateRow::Delivered, received_at)?;
        let self_device = d
            .identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .device_id();
        let stored = StoredMessage {
            env: env.clone(),
            state: MsgStateRow::Delivered,
            received_at,
        };
        let msg = message_view(d.store.as_ref(), &self_device, &d.conns.peers(), &stored);
        self.emit(Event::MessageAdded { msg });
        if let Some(row) = d.store.get_conversation(conv)? {
            self.emit_conv_updated(&row);
        }
        Ok(env.id)
    }

    fn mark_delivered(&self, id: &MsgId) {
        let d = &self.deps;
        if d.store
            .set_message_state(id, MsgStateRow::Delivered)
            .is_err()
        {
            return;
        }
        if let Ok(Some(stored)) = d.store.get_message(id) {
            let self_device = d
                .identity
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .device_id();
            let msg = message_view(d.store.as_ref(), &self_device, &d.conns.peers(), &stored);
            self.emit(Event::MessageUpdated { msg });
        }
    }

    /// 访客会话当前对应的活动 Web 会话；已结束则 PeerOffline。
    fn active_session_of(&self, conv: &ConvId) -> Result<String, CoreError> {
        let g = self.lock();
        g.as_ref()
            .and_then(|r| {
                r.sessions
                    .iter()
                    .find(|(_, s)| s.phase == Phase::Allowed && s.conv.as_ref() == Some(conv))
                    .map(|(k, _)| k.clone())
            })
            .ok_or(CoreError::PeerOffline)
    }

    fn push_host(&self, sid: &str, news: Vec<LogEntryNew>) -> Result<(), CoreError> {
        let mut g = self.lock();
        let sess = allowed_mut(&mut g, sid).map_err(|_| CoreError::PeerOffline)?;
        for n in news {
            let content = match n.content {
                ContentNew::Text(t) => Content::Text(t),
                ContentNew::File(entry) => {
                    let fid = random_hex(16);
                    sess.files.insert(fid.clone(), entry.clone());
                    Content::File { fid, entry }
                }
            };
            push_entry(
                sess,
                LogEntry {
                    id: n.id,
                    from_page: false,
                    ts: now_millis(),
                    content,
                },
            );
        }
        Ok(())
    }

    // ---------- 结束与清理 ----------

    /// 结束一批会话：通知页面、删除未完成的 `.part`、记录结束时间。
    async fn end_sessions(&self, ids: Vec<String>) {
        let mut parts: Vec<PathBuf> = Vec::new();
        let mut convs: Vec<ConvId> = Vec::new();
        {
            let mut g = self.lock();
            let Some(run) = g.as_mut() else { return };
            for id in ids {
                let Some(mut sess) = run.sessions.remove(&id) else {
                    continue;
                };
                if sess.ws.is_some() {
                    send_ws_value(&mut sess, json!({"t":"bye","reason":"ended"}));
                    if let Some((_, tx)) = &sess.ws {
                        let _ = tx.send(WsOut::Close);
                    }
                }
                for u in sess.uploads.values() {
                    if u.finished.is_none() {
                        parts.push(u.path_part.clone());
                    }
                }
                if let Some(c) = sess.conv {
                    convs.push(c);
                }
            }
        }
        let now = now_millis();
        if let Ok(conn) = self.deps.store.conn() {
            for c in &convs {
                let _ = conn.execute(
                    "UPDATE web_guests SET ended_at = ?1 WHERE conv_id = ?2",
                    rusqlite::params![now, &c.0[..]],
                );
            }
        }
        let _ = tokio::task::spawn_blocking(move || {
            for p in parts {
                let _ = std::fs::remove_file(p);
            }
        })
        .await;
    }

    async fn stop(&self) {
        let ids: Vec<String> = {
            let g = self.lock();
            match g.as_ref() {
                Some(run) => run.sessions.keys().cloned().collect(),
                None => return,
            }
        };
        self.end_sessions(ids).await;
        let run = self.lock().take();
        if let Some(run) = run {
            run.shutdown.send_replace(true);
            self.emit(Event::WebSessionChanged { info: None });
        }
    }

    /// 周期清理：握手过期、会话空闲超时、长期无会话则停止监听。
    async fn sweeper(self: Arc<Self>) {
        loop {
            let tick =
                (self.idle_timeout() / 4).clamp(Duration::from_millis(50), Duration::from_secs(1));
            tokio::time::sleep(tick).await;
            let idle = self.idle_timeout();
            let mut expired: Vec<String> = Vec::new();
            let mut stop_all = false;
            {
                let mut g = self.lock();
                let Some(run) = g.as_mut() else { return };
                let now = Instant::now();
                run.sessions.retain(|id, s| {
                    let drop_it = match s.phase {
                        Phase::Pending => s.created.elapsed() > PENDING_TTL,
                        Phase::Awaiting => s.created.elapsed() > AWAITING_TTL,
                        Phase::Denied => s.last_activity.elapsed() > DENIED_TTL,
                        Phase::Allowed => false,
                    };
                    if !drop_it
                        && s.phase == Phase::Allowed
                        && (s.last_activity.elapsed() >= idle || s.host_seq >= SEQ_MAX)
                    {
                        expired.push(id.clone());
                    }
                    !drop_it
                });
                if run.sessions.is_empty() {
                    if now.duration_since(run.idle_since) >= idle {
                        stop_all = true;
                    }
                } else {
                    run.idle_since = now;
                }
            }
            if !expired.is_empty() {
                self.end_sessions(expired).await;
            }
            if stop_all {
                self.stop().await;
                return;
            }
        }
    }
}

/// 取处于 Allowed 阶段的会话，否则 403。
fn allowed_mut<'a>(g: &'a mut Option<Run>, sid: &str) -> Result<&'a mut Session, WebErr> {
    let run = g.as_mut().ok_or(WebErr::Forbidden)?;
    match run.sessions.get_mut(sid) {
        Some(s) if s.phase == Phase::Allowed => Ok(s),
        _ => Err(WebErr::Forbidden),
    }
}

/// 来源地址的局域网判定（取本机网卡实时列表）。
pub(crate) fn peer_is_lan(addr: SocketAddr) -> bool {
    is_lan(addr.ip(), &local_ifaces())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::path::LocalIface;

    #[test]
    fn seq_window_rejects_replay_and_stale() {
        let mut w = SeqWindow::default();
        assert!(!w.accept(0));
        assert!(w.accept(1));
        assert!(!w.accept(1));
        assert!(w.accept(5000));
        // 5000 - 4096 = 904：不高于 904 的值过旧。
        assert!(!w.accept(904));
        assert!(w.accept(905));
        assert!(!w.accept(SEQ_MAX + 1));
        assert!(w.accept(SEQ_MAX));
    }

    #[test]
    fn non_lan_source_is_rejected() {
        let ifaces = vec![LocalIface {
            name: "eth0".into(),
            addr: "192.168.1.5".parse().unwrap(),
            prefix: 24,
            tunnel: None,
        }];
        assert!(is_lan("192.168.1.9".parse().unwrap(), &ifaces));
        assert!(is_lan("127.0.0.1".parse().unwrap(), &ifaces));
        assert!(!is_lan("8.8.8.8".parse().unwrap(), &ifaces));
    }

    #[test]
    fn guest_device_is_stable_per_conv() {
        let a = ConvId([1; 32]);
        assert_eq!(guest_device(&a), guest_device(&a));
        assert_ne!(guest_device(&a), guest_device(&ConvId([2; 32])));
    }
}
