//! LocalSend 接收服务（HTTPS）与对外门面 `LocalSend`。

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::cert;
use crate::discovery::{self, LsDiscovery, LsPeerEvent};
use crate::types::{
    LsFileMeta, LsInfo, LsPeer, LsProtocol, PrepareUploadRequest, PrepareUploadResponse,
};
use crate::LsError;

/// prepare-upload 请求体上限。
const PREPARE_BODY_LIMIT: usize = 1024 * 1024;
/// 等待用户决定的最长时间。
const DECISION_TIMEOUT: Duration = Duration::from_secs(60);
/// 会话无活动超过该时长后，允许被新的 prepare-upload 取代。
const SESSION_IDLE: Duration = Duration::from_secs(300);
/// 已发现对端的保留数量上限与存活时间。
const PEERS_MAX: usize = 256;
const PEER_TTL: Duration = Duration::from_secs(30 * 60);
/// 落盘文件名的 UTF-8 字节上限。
const FILE_NAME_MAX_BYTES: usize = 200;

/// 启动配置。
pub struct LsConfig {
    pub alias: String,
    pub device_model: String,
    pub device_type: String,
    pub download_dir: PathBuf,
    pub cert_dir: PathBuf,
    /// HTTP(S) 监听端口，生产为 `LOCALSEND_PORT`，测试可为 0。
    pub http_port: u16,
    /// 组播 UDP 端口，生产为 `LOCALSEND_PORT`；不得为 0。
    pub mcast_port: u16,
    /// 是否启动组播发现（测试里关闭，避免争用 UDP 端口）。
    pub enable_discovery: bool,
}

/// 用户对接收请求的决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LsDecision {
    Accept,
    Reject,
}

/// 上报给上层的事件。
#[derive(Debug)]
pub enum LsEvent {
    PeerSeen(LsPeer),
    IncomingFiles {
        session: String,
        peer: LsPeer,
        files: Vec<LsFileMeta>,
        reply: oneshot::Sender<LsDecision>,
    },
    TextReceived {
        peer: LsPeer,
        text: String,
        reply: oneshot::Sender<LsDecision>,
    },
    FileReceived {
        session: String,
        peer: LsPeer,
        file: LsFileMeta,
        path: PathBuf,
    },
    Progress {
        session: String,
        file_id: String,
        done: u64,
        total: u64,
    },
    SessionFinished {
        session: String,
        ok: bool,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileState {
    Waiting,
    Receiving,
    Done,
}

struct SessionFile {
    meta: LsFileMeta,
    token: String,
    state: FileState,
    /// 正在写入的 `.part` 路径，取消时清理。
    part: Option<PathBuf>,
}

struct Session {
    id: String,
    peer: LsPeer,
    files: HashMap<String, SessionFile>,
    last_activity: Instant,
}

pub(crate) struct Inner {
    pub(crate) info: LsInfo,
    pub(crate) listening: bool,
    pub(crate) client: reqwest::Client,
    download_dir: PathBuf,
    events: mpsc::Sender<LsEvent>,
    peers: Mutex<HashMap<(IpAddr, String), (LsPeer, Instant)>>,
    session: Mutex<Option<Session>>,
    handle: axum_server::Handle,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    discovery: Mutex<Option<LsDiscovery>>,
}

/// LocalSend 兼容服务；`Clone` 共享同一内部状态。
#[derive(Clone)]
pub struct LocalSend {
    pub(crate) inner: Arc<Inner>,
}

impl LocalSend {
    /// 启动接收服务与发现。TCP 端口被占用时不报错：`listening()` 为 false，
    /// 发现只收不发，仍可作为发送方使用。
    pub async fn start(cfg: LsConfig, events: mpsc::Sender<LsEvent>) -> Result<LocalSend, LsError> {
        let ls_cert = cert::load_or_create(&cfg.cert_dir)?;
        let fingerprint = cert::fingerprint(&ls_cert.cert_der);
        std::fs::create_dir_all(&cfg.download_dir)?;

        let bound = TcpListener::bind(("0.0.0.0", cfg.http_port));
        let (listener, port) = match bound {
            Ok(l) => {
                l.set_nonblocking(true)?;
                let port = l.local_addr()?.port();
                (Some(l), port)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                tracing::warn!(
                    "localsend port {} is in use (LocalSend may be running); receiving and announcing are disabled",
                    cfg.http_port
                );
                (None, cfg.http_port)
            }
            Err(e) => return Err(LsError::Io(e)),
        };
        let listening = listener.is_some();

        let info = LsInfo {
            alias: cfg.alias,
            version: "2.0".into(),
            device_model: Some(cfg.device_model),
            device_type: Some(cfg.device_type),
            fingerprint,
            port,
            protocol: LsProtocol::Https,
            download: false,
        };
        let handle = axum_server::Handle::new();
        let inner = Arc::new(Inner {
            info: info.clone(),
            listening,
            client: discovery::lenient_client()?,
            download_dir: cfg.download_dir,
            events,
            peers: Mutex::new(HashMap::new()),
            session: Mutex::new(None),
            handle: handle.clone(),
            tasks: Mutex::new(Vec::new()),
            discovery: Mutex::new(None),
        });

        if let Some(listener) = listener {
            let tls = RustlsConfig::from_config(Arc::new(server_tls_config(&ls_cert)?));
            let app = router(inner.clone());
            let server = axum_server::from_tcp_rustls(listener, tls)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>());
            let task = tokio::spawn(async move {
                if let Err(e) = server.await {
                    tracing::warn!("localsend server stopped: {e}");
                }
            });
            lock(&inner.tasks).push(task);
        }

        if cfg.enable_discovery {
            let (tx, mut rx) = mpsc::channel(64);
            match discovery::spawn(info, cfg.mcast_port, listening, tx) {
                Ok(d) => {
                    *lock(&inner.discovery) = Some(d);
                    let me = inner.clone();
                    let task = tokio::spawn(async move {
                        while let Some(LsPeerEvent::Seen(peer)) = rx.recv().await {
                            me.note_peer(peer);
                        }
                    });
                    lock(&inner.tasks).push(task);
                }
                Err(e) => tracing::warn!("localsend discovery unavailable: {e}"),
            }
        }
        Ok(LocalSend { inner })
    }

    /// TCP 端口是否绑定成功（能否接收）。
    pub fn listening(&self) -> bool {
        self.inner.listening
    }

    /// 自身信息，`port` 为实际监听端口。
    pub fn info(&self) -> LsInfo {
        self.inner.info.clone()
    }

    /// 已发现的对端。
    pub fn peers(&self) -> Vec<LsPeer> {
        let now = Instant::now();
        lock(&self.inner.peers)
            .values()
            .filter(|(_, t)| now.duration_since(*t) < PEER_TTL)
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// 停止服务与发现。
    pub async fn shutdown(&self) {
        self.inner.handle.shutdown();
        *lock(&self.inner.discovery) = None;
        for t in lock(&self.inner.tasks).drain(..) {
            t.abort();
        }
    }
}

/// 显式指定 aws-lc-rs 作为密码库，不依赖进程级默认 provider（与 core 保持同一个密码库）。
fn server_tls_config(c: &cert::LsCert) -> Result<rustls::ServerConfig, LsError> {
    let tls_err = |e: rustls::Error| LsError::Tls(e.to_string());
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(c.key_der.clone());
    rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(tls_err)?
    .with_no_client_auth()
    .with_single_cert(
        vec![rustls::pki_types::CertificateDer::from(c.cert_der.clone())],
        key.into(),
    )
    .map_err(tls_err)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // 持锁段内不会 panic；即便中毒，数据仍可用
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    /// 记录对端并上报 `PeerSeen`（通道满则丢弃）。
    fn note_peer(&self, peer: LsPeer) {
        {
            let now = Instant::now();
            let mut peers = lock(&self.peers);
            let key = (peer.ip, peer.info.fingerprint.clone());
            if !peers.contains_key(&key) {
                peers.retain(|_, (_, t)| now.duration_since(*t) < PEER_TTL);
                if peers.len() >= PEERS_MAX {
                    // 容量满：淘汰最久未见的
                    if let Some(oldest) = peers
                        .iter()
                        .min_by_key(|(_, (_, t))| *t)
                        .map(|(k, _)| k.clone())
                    {
                        peers.remove(&oldest);
                    }
                }
            }
            peers.insert(key, (peer.clone(), now));
        }
        let _ = self.events.try_send(LsEvent::PeerSeen(peer));
    }
}

fn router(inner: Arc<Inner>) -> Router {
    Router::new()
        .route("/api/localsend/v2/register", post(register))
        .route("/api/localsend/v2/prepare-upload", post(prepare_upload))
        .route("/api/localsend/v2/upload", post(upload))
        .route("/api/localsend/v2/cancel", post(cancel))
        .layer(DefaultBodyLimit::max(PREPARE_BODY_LIMIT))
        .with_state(inner)
}

async fn register(
    State(s): State<Arc<Inner>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    let Ok(info) = serde_json::from_slice::<LsInfo>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    s.note_peer(LsPeer {
        info,
        ip: addr.ip(),
    });
    Json(s.info.clone()).into_response()
}

fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 当前会话是否禁止被新的 prepare-upload 取代：有文件正在接收，或仍在无活动窗口内。
fn session_blocks_takeover(sess: &Session) -> bool {
    sess.files.values().any(|f| f.state == FileState::Receiving)
        || sess.last_activity.elapsed() < SESSION_IDLE
}

async fn prepare_upload(
    State(s): State<Arc<Inner>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    let Ok(req) = serde_json::from_slice::<PrepareUploadRequest>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if req.files.is_empty() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let peer = LsPeer {
        info: req.info,
        ip: addr.ip(),
    };
    s.note_peer(peer.clone());

    // 提前拒绝，避免对用户弹出注定失败的请求
    if lock(&s.session)
        .as_ref()
        .is_some_and(session_blocks_takeover)
    {
        return StatusCode::CONFLICT.into_response();
    }

    let (reply, rx) = oneshot::channel();
    let text_only = req.files.len() == 1
        && req
            .files
            .values()
            .next()
            .is_some_and(|f| f.file_type == "text/plain" && f.preview.is_some());
    let session_id = uuid::Uuid::new_v4().to_string();
    let files: Vec<LsFileMeta> = req.files.values().cloned().collect();
    let event = if text_only {
        LsEvent::TextReceived {
            peer: peer.clone(),
            text: files[0].preview.clone().unwrap_or_default(),
            reply,
        }
    } else {
        LsEvent::IncomingFiles {
            session: session_id.clone(),
            peer: peer.clone(),
            files: files.clone(),
            reply,
        }
    };
    if s.events.send(event).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let decision = tokio::time::timeout(DECISION_TIMEOUT, rx).await;
    if !matches!(decision, Ok(Ok(LsDecision::Accept))) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if text_only {
        return StatusCode::NO_CONTENT.into_response();
    }

    let tokens: BTreeMap<String, String> =
        req.files.keys().map(|k| (k.clone(), new_token())).collect();
    let session_files = req
        .files
        .into_iter()
        .map(|(id, meta)| {
            let token = tokens.get(&id).cloned().unwrap_or_default();
            (
                id,
                SessionFile {
                    meta,
                    token,
                    state: FileState::Waiting,
                    part: None,
                },
            )
        })
        .collect();
    // 审批通过后才替换；审批期间可能已有别的会话建立，再判一次
    let superseded = {
        let mut g = lock(&s.session);
        if g.as_ref().is_some_and(session_blocks_takeover) {
            return StatusCode::CONFLICT.into_response();
        }
        g.replace(Session {
            id: session_id.clone(),
            peer,
            files: session_files,
            last_activity: Instant::now(),
        })
    };
    if let Some(old) = superseded {
        discard_session(&s, old, "superseded by a new session").await;
    }
    Json(PrepareUploadResponse {
        session_id,
        files: tokens,
    })
    .into_response()
}

/// 清理被丢弃会话遗留的 `.part` 并上报结束。
async fn discard_session(s: &Inner, sess: Session, reason: &str) {
    for f in sess.files.values() {
        if let Some(p) = &f.part {
            let _ = std::fs::remove_file(p);
        }
    }
    let _ = s
        .events
        .send(LsEvent::SessionFinished {
            session: sess.id,
            ok: false,
            error: Some(reason.into()),
        })
        .await;
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// 去掉路径分隔符、非法字符、控制与 bidi 字符，规避 Windows 保留名，并限制长度。
pub(crate) fn sanitize_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_control(*c))
        .map(|c| {
            if matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.is_empty() {
        return "file".to_string();
    }
    let mut out = truncate_keep_ext(&cleaned, FILE_NAME_MAX_BYTES);
    let base = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.len() == 4)
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0');
    if reserved {
        out.insert(0, '_');
    }
    out
}

/// 按 UTF-8 字节数截断主名，尽量保留扩展名。
fn truncate_keep_ext(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut keep = max.saturating_sub(ext.len());
    while !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}{}", &stem[..keep], ext)
}

/// 在目录中为文件名预留不重名的路径：创建 `.part` 文件占位，返回 (最终路径, part 路径, 已打开的 part)。
pub(crate) fn reserve_path(
    dir: &Path,
    name: &str,
) -> std::io::Result<(PathBuf, PathBuf, std::fs::File)> {
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = p.extension().and_then(|s| s.to_str());
    for i in 0..10_000u32 {
        let candidate = match (i, ext) {
            (0, _) => name.to_string(),
            (n, Some(e)) => format!("{stem} ({n}).{e}"),
            (n, None) => format!("{stem} ({n})"),
        };
        let final_path = dir.join(&candidate);
        if final_path.exists() {
            continue;
        }
        let part = dir.join(format!("{candidate}.part"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
        {
            Ok(f) => return Ok((final_path, part, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free file name",
    ))
}

/// 一次文件上传的现场守卫：只要没有 `disarm`（出错、客户端断开导致 handler 被 drop），
/// 就把文件状态退回 Waiting 并删除 `.part`，让发送方可以重试。
struct UploadGuard {
    inner: Arc<Inner>,
    sid: String,
    fid: String,
    part: Option<PathBuf>,
    armed: bool,
}

impl UploadGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UploadGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(sess) = lock(&self.inner.session)
            .as_mut()
            .filter(|x| x.id == self.sid)
        {
            if let Some(f) = sess.files.get_mut(&self.fid) {
                if f.state == FileState::Receiving {
                    f.state = FileState::Waiting;
                    f.part = None;
                }
            }
        }
        if let Some(part) = self.part.take() {
            // 先同步删一次（写盘线程已结束的常见情况），失败再交给后台重试
            if std::fs::remove_file(&part).is_ok() {
                return;
            }
            // 写盘线程可能尚未释放文件（Windows 下删除会失败），放到阻塞线程里重试
            let remove = move || {
                for _ in 0..20 {
                    if std::fs::remove_file(&part).is_ok() || !part.exists() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            };
            match tokio::runtime::Handle::try_current() {
                Ok(h) => {
                    h.spawn_blocking(remove);
                }
                Err(_) => remove(),
            }
        }
    }
}

async fn upload(
    State(s): State<Arc<Inner>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<HashMap<String, String>>,
    body: Body,
) -> StatusCode {
    let (Some(sid), Some(fid), Some(token)) = (q.get("sessionId"), q.get("fileId"), q.get("token"))
    else {
        return StatusCode::BAD_REQUEST;
    };
    let (meta, peer) = {
        let mut g = lock(&s.session);
        let Some(sess) = g
            .as_mut()
            .filter(|x| &x.id == sid && x.peer.ip == addr.ip())
        else {
            return StatusCode::FORBIDDEN;
        };
        let Some(file) = sess.files.get_mut(fid) else {
            return StatusCode::FORBIDDEN;
        };
        if &file.token != token {
            return StatusCode::FORBIDDEN;
        }
        if file.state != FileState::Waiting {
            return StatusCode::CONFLICT;
        }
        file.state = FileState::Receiving;
        sess.last_activity = Instant::now();
        (file.meta.clone(), sess.peer.clone())
    };
    let mut guard = UploadGuard {
        inner: s.clone(),
        sid: sid.clone(),
        fid: fid.clone(),
        part: None,
        armed: true,
    };

    let dir = s.download_dir.clone();
    let name = sanitize_file_name(&meta.file_name);
    let reserved = tokio::task::spawn_blocking(move || reserve_path(&dir, &name)).await;
    let Ok(Ok((final_path, part, file))) = reserved else {
        return StatusCode::INTERNAL_SERVER_ERROR;
    };
    guard.part = Some(part.clone());
    if let Some(sess) = lock(&s.session).as_mut() {
        if let Some(f) = sess.files.get_mut(fid) {
            f.part = Some(part.clone());
        }
    }

    // 用阻塞线程写盘并计算 SHA-256（不使用 tokio::fs::File）
    let (tx, mut rx) = mpsc::channel::<Bytes>(8);
    let writer = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut file = std::io::BufWriter::new(file);
        let mut hasher = Sha256::new();
        while let Some(chunk) = rx.blocking_recv() {
            hasher.update(&chunk);
            file.write_all(&chunk)?;
        }
        file.flush()?;
        Ok(hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    });
    let mut stream = body.into_data_stream();
    let mut done = 0u64;
    let mut failure: Option<StatusCode> = None;
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            failure = Some(StatusCode::INTERNAL_SERVER_ERROR);
            break;
        };
        done += chunk.len() as u64;
        if done > meta.size {
            // 超出声明大小：立即中止
            failure = Some(StatusCode::PAYLOAD_TOO_LARGE);
            break;
        }
        if tx.send(chunk).await.is_err() {
            failure = Some(StatusCode::INTERNAL_SERVER_ERROR);
            break;
        }
        if let Some(sess) = lock(&s.session).as_mut().filter(|x| &x.id == sid) {
            sess.last_activity = Instant::now();
        }
        let _ = s.events.try_send(LsEvent::Progress {
            session: sid.clone(),
            file_id: fid.clone(),
            done,
            total: meta.size,
        });
    }
    drop(tx);
    let digest = writer.await;
    if let Some(code) = failure {
        return code;
    }
    let Ok(Ok(digest)) = digest else {
        return StatusCode::INTERNAL_SERVER_ERROR;
    };
    if done != meta.size {
        return StatusCode::BAD_REQUEST;
    }
    if let Some(expected) = &meta.sha256 {
        if !expected.eq_ignore_ascii_case(&digest) {
            return StatusCode::UNPROCESSABLE_ENTITY;
        }
    }
    if tokio::fs::rename(&part, &final_path).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    guard.part = None;
    guard.disarm();

    // 会话可能在传输期间被取消
    let all_done = {
        let mut g = lock(&s.session);
        let Some(sess) = g.as_mut().filter(|x| &x.id == sid) else {
            drop(g);
            let _ = std::fs::remove_file(&final_path);
            return StatusCode::FORBIDDEN;
        };
        if let Some(f) = sess.files.get_mut(fid) {
            f.state = FileState::Done;
            f.part = None;
        }
        let all = sess.files.values().all(|f| f.state == FileState::Done);
        if all {
            *g = None;
        }
        all
    };
    let _ = s
        .events
        .send(LsEvent::FileReceived {
            session: sid.clone(),
            peer,
            file: meta,
            path: final_path,
        })
        .await;
    if all_done {
        let _ = s
            .events
            .send(LsEvent::SessionFinished {
                session: sid.clone(),
                ok: true,
                error: None,
            })
            .await;
    }
    StatusCode::OK
}

async fn cancel(
    State(s): State<Arc<Inner>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<HashMap<String, String>>,
) -> StatusCode {
    let Some(sid) = q.get("sessionId") else {
        return StatusCode::BAD_REQUEST;
    };
    let removed = {
        let mut g = lock(&s.session);
        match g.as_ref() {
            Some(x) if &x.id == sid && x.peer.ip == addr.ip() => g.take(),
            _ => None,
        }
    };
    let Some(sess) = removed else {
        return StatusCode::FORBIDDEN;
    };
    for f in sess.files.values() {
        if let Some(p) = &f.part {
            let _ = std::fs::remove_file(p);
        }
    }
    let _ = s
        .events
        .send(LsEvent::SessionFinished {
            session: sess.id,
            ok: false,
            error: Some("cancelled by sender".into()),
        })
        .await;
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_paths_and_dots() {
        assert_eq!(sanitize_file_name("../evil.txt"), "evil.txt");
        assert_eq!(sanitize_file_name("a\\b\\c.txt"), "c.txt");
        assert_eq!(sanitize_file_name(".."), "file");
        assert_eq!(sanitize_file_name(""), "file");
        assert_eq!(sanitize_file_name("dir/"), "file");
        assert_eq!(sanitize_file_name("ok name.tar.gz"), "ok name.tar.gz");
    }

    #[test]
    fn sanitize_handles_windows_and_unicode_hazards() {
        assert_eq!(
            sanitize_file_name("a:b*c?d\"e<f>g|h.txt"),
            "a_b_c_d_e_f_g_h.txt"
        );
        assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_file_name("lpt1"), "_lpt1");
        assert_eq!(sanitize_file_name("COM0.txt"), "COM0.txt");
        assert_eq!(sanitize_file_name("a\u{202E}txt.exe"), "atxt.exe");
        assert_eq!(sanitize_file_name("trailing. "), "trailing");
    }

    #[test]
    fn sanitize_limits_length_and_keeps_extension() {
        let long = format!("{}.jpeg", "字".repeat(100));
        let out = sanitize_file_name(&long);
        assert!(out.len() <= FILE_NAME_MAX_BYTES);
        assert!(out.ends_with(".jpeg"));
        let long = "a".repeat(500);
        assert_eq!(sanitize_file_name(&long).len(), FILE_NAME_MAX_BYTES);
    }

    #[test]
    fn reserve_path_appends_counter_on_collision() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), b"x").unwrap();
        let (f1, p1, _h1) = reserve_path(d.path(), "a.txt").unwrap();
        assert_eq!(f1.file_name().unwrap(), "a (1).txt");
        assert!(p1.exists());
        let (f2, _p2, _h2) = reserve_path(d.path(), "a.txt").unwrap();
        assert_eq!(f2.file_name().unwrap(), "a (2).txt");
        let (f3, _p3, _h3) = reserve_path(d.path(), "noext").unwrap();
        assert_eq!(f3.file_name().unwrap(), "noext");
    }
}
