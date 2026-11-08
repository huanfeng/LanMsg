//! LocalSend 接收服务（HTTPS）与对外门面 `LocalSend`。

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::net::{SocketAddr, TcpListener};
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
    /// 审批中占位：尚无文件，不接受 upload。
    pending: bool,
    files: HashMap<String, SessionFile>,
    last_activity: Instant,
}

pub(crate) struct Inner {
    pub(crate) info: LsInfo,
    pub(crate) listening: bool,
    pub(crate) client: reqwest::Client,
    download_dir: PathBuf,
    events: mpsc::Sender<LsEvent>,
    peers: Mutex<HashMap<String, LsPeer>>,
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
        lock(&self.inner.peers).values().cloned().collect()
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

/// 显式指定 ring 作为密码库，不依赖进程级默认 provider（依赖树里同时存在 ring 与 aws-lc-rs）。
fn server_tls_config(c: &cert::LsCert) -> Result<rustls::ServerConfig, LsError> {
    let tls_err = |e: rustls::Error| LsError::Tls(e.to_string());
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(c.key_der.clone());
    rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
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
        lock(&self.peers).insert(peer.info.fingerprint.clone(), peer.clone());
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

    // 占位：同一时刻只允许一个会话
    {
        let mut g = lock(&s.session);
        if let Some(cur) = g.as_ref() {
            if cur.last_activity.elapsed() < SESSION_IDLE {
                return StatusCode::CONFLICT.into_response();
            }
        }
        *g = Some(Session {
            id: String::new(),
            peer: peer.clone(),
            pending: true,
            files: HashMap::new(),
            last_activity: Instant::now(),
        });
    }
    let release_pending = |s: &Inner| {
        let mut g = lock(&s.session);
        if g.as_ref().is_some_and(|x| x.pending) {
            *g = None;
        }
    };

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
        release_pending(&s);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let decision = tokio::time::timeout(DECISION_TIMEOUT, rx).await;
    if !matches!(decision, Ok(Ok(LsDecision::Accept))) {
        release_pending(&s);
        return StatusCode::FORBIDDEN.into_response();
    }
    if text_only {
        release_pending(&s);
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
    *lock(&s.session) = Some(Session {
        id: session_id.clone(),
        peer,
        pending: false,
        files: session_files,
        last_activity: Instant::now(),
    });
    Json(PrepareUploadResponse {
        session_id,
        files: tokens,
    })
    .into_response()
}

/// 去掉路径分隔符、`..` 与控制字符，得到可安全落盘的单个文件名。
pub(crate) fn sanitize_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
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

/// 把文件状态退回 Waiting（让发送方可重试），并清理 part。
fn reset_file(s: &Inner, sid: &str, fid: &str, part: &Path) {
    let _ = std::fs::remove_file(part);
    if let Some(sess) = lock(&s.session).as_mut().filter(|x| x.id == sid) {
        if let Some(f) = sess.files.get_mut(fid) {
            f.state = FileState::Waiting;
            f.part = None;
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
            .filter(|x| !x.pending && &x.id == sid && x.peer.ip == addr.ip())
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

    let dir = s.download_dir.clone();
    let name = sanitize_file_name(&meta.file_name);
    let reserved = tokio::task::spawn_blocking(move || reserve_path(&dir, &name)).await;
    let Ok(Ok((final_path, part, file))) = reserved else {
        if let Some(sess) = lock(&s.session).as_mut() {
            if let Some(f) = sess.files.get_mut(fid) {
                f.state = FileState::Waiting;
            }
        }
        return StatusCode::INTERNAL_SERVER_ERROR;
    };
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
    let mut stream_ok = true;
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            stream_ok = false;
            break;
        };
        done += chunk.len() as u64;
        if tx.send(chunk).await.is_err() {
            stream_ok = false;
            break;
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
    let Ok(Ok(digest)) = digest else {
        reset_file(&s, sid, fid, &part);
        return StatusCode::INTERNAL_SERVER_ERROR;
    };
    if !stream_ok {
        reset_file(&s, sid, fid, &part);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    if let Some(expected) = &meta.sha256 {
        if !expected.eq_ignore_ascii_case(&digest) {
            reset_file(&s, sid, fid, &part);
            return StatusCode::UNPROCESSABLE_ENTITY;
        }
    }
    if tokio::fs::rename(&part, &final_path).await.is_err() {
        reset_file(&s, sid, fid, &part);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

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
            Some(x) if !x.pending && &x.id == sid && x.peer.ip == addr.ip() => g.take(),
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
