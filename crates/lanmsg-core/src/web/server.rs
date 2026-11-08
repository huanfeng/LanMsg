//! Web 服务端：HTTP 路由、WebSocket 与安全响应头。
//! 业务逻辑都在 session.rs，这里只做协议到调用的映射。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, RawQuery, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use data_encoding::BASE64URL_NOPAD;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::web::session::{Shared, WaitStatus, WebErr, WsOut};
use crate::web::transfer::CHUNK_SIZE;

const SESSION_HEADER: &str = "x-lanmsg-session";
const REQ_HEADER: &str = "x-lanmsg-req";
/// 长轮询最长等待。
const WAIT_TIMEOUT: Duration = Duration::from_secs(25);

/// 页面安全头：CSP（frame-ancestors 只能放响应头）、nosniff、no-referrer。
const CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' blob:; \
style-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

pub(crate) fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/api/web/hello", post(hello))
        .route("/api/web/confirm", post(confirm))
        .route("/api/web/wait", get(wait))
        .route("/api/web/ws", get(ws_route))
        .route("/api/web/upload/init", post(upload_init))
        .route("/api/web/upload/:fid/finish", post(upload_finish))
        .route("/api/web/upload/:fid/:index", put(upload_chunk))
        .route("/api/web/download/:fid/:index", get(download))
        .fallback(get(static_file))
        .layer(DefaultBodyLimit::max(CHUNK_SIZE as usize + 4096))
        .layer(middleware::from_fn_with_state(shared.clone(), guard))
        .with_state(shared)
}

pub(crate) async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    mut shutdown: watch::Receiver<bool>,
) {
    let make = app.into_make_service_with_connect_info::<SocketAddr>();
    let res = axum::serve(listener, make)
        .with_graceful_shutdown(async move {
            let _ = shutdown.wait_for(|v| *v).await;
        })
        .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "web server stopped with error");
    }
}

/// 所有请求先判来源：非局域网一律 403；所有响应都带安全头。
async fn guard(
    State(sh): State<Arc<Shared>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let mut resp = if sh.allows(addr) {
        next.run(req).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    };
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

impl IntoResponse for WebErr {
    fn into_response(self) -> Response {
        match self {
            WebErr::Forbidden => {
                (StatusCode::FORBIDDEN, Json(json!({"error":"forbidden"}))).into_response()
            }
            WebErr::BadMac => {
                (StatusCode::FORBIDDEN, Json(json!({"error":"bad_mac"}))).into_response()
            }
            WebErr::NotFound => {
                (StatusCode::NOT_FOUND, Json(json!({"error":"not_found"}))).into_response()
            }
            WebErr::BadRequest => (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"bad_request"})),
            )
                .into_response(),
            WebErr::TooMany => (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":"too_many"})),
            )
                .into_response(),
            WebErr::Busy => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":"busy"})),
            )
                .into_response(),
            WebErr::Gone => (StatusCode::GONE, Json(json!({"error":"gone"}))).into_response(),
            WebErr::TooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({"error":"too_large"})),
            )
                .into_response(),
            WebErr::NoSpace => (
                StatusCode::INSUFFICIENT_STORAGE,
                Json(json!({"error":"no_space"})),
            )
                .into_response(),
            WebErr::Conflict(body) => sealed(StatusCode::CONFLICT, body),
            WebErr::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"internal"})),
            )
                .into_response(),
        }
    }
}

fn sealed(status: StatusCode, body: Vec<u8>) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        body,
    )
        .into_response()
}

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(HeaderName::from_bytes(name.as_bytes()).ok()?)?
        .to_str()
        .ok()
}

/// 取 `X-LanMsg-Session`；缺失一律 403。
fn session_id(h: &HeaderMap) -> Result<String, WebErr> {
    header_str(h, SESSION_HEADER)
        .map(str::to_string)
        .ok_or(WebErr::Forbidden)
}

fn query_param(q: &Option<String>, key: &str) -> Option<String> {
    q.as_deref()?
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

fn b64u(b: &[u8]) -> String {
    BASE64URL_NOPAD.encode(b)
}

fn parse_idx(s: &str) -> Result<u32, WebErr> {
    s.parse().map_err(|_| WebErr::BadRequest)
}

// ---------- 静态文件与握手 ----------

async fn static_file(uri: axum::http::Uri) -> Response {
    match lanmsg_web_assets::get(uri.path()) {
        Some((body, mime)) => ([(header::CONTENT_TYPE, mime)], body).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn hello(
    State(sh): State<Arc<Shared>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, WebErr> {
    let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("");
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let out = sh.hello(addr.ip(), ua, s("mode"), s("page_pk"), s("page_nonce"))?;
    Ok(Json(json!({
        "session": out.session,
        "host_pk": b64u(&out.host_pk),
        "host_nonce": b64u(&out.host_nonce),
        "scrypt_salt": b64u(&out.scrypt_salt),
    })))
}

async fn confirm(
    State(sh): State<Arc<Shared>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, WebErr> {
    let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("");
    sh.confirm(addr.ip(), s("session"), s("page_mac"))?;
    Ok(Json(json!({"status":"pending"})))
}

async fn wait(
    State(sh): State<Arc<Shared>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    RawQuery(q): RawQuery,
) -> Result<Json<Value>, WebErr> {
    let sid = query_param(&q, "session").ok_or(WebErr::BadRequest)?;
    let mut rx = sh.wait_rx(addr.ip(), &sid)?;
    let deadline = tokio::time::Instant::now() + WAIT_TIMEOUT;
    loop {
        let cur = rx.borrow_and_update().clone();
        match cur {
            WaitStatus::Allowed(mac) => {
                return Ok(Json(json!({"status":"allowed","host_mac":b64u(&mac)})))
            }
            WaitStatus::Denied => return Ok(Json(json!({"status":"denied"}))),
            WaitStatus::Pending => {}
        }
        if tokio::time::timeout_at(deadline, rx.changed())
            .await
            .is_err()
        {
            return Ok(Json(json!({"status":"pending"})));
        }
    }
}

// ---------- WebSocket ----------

async fn ws_route(
    State(sh): State<Arc<Shared>>,
    RawQuery(q): RawQuery,
    ws: WebSocketUpgrade,
) -> Result<Response, WebErr> {
    let sid = query_param(&q, "session").ok_or(WebErr::Forbidden)?;
    // 升级请求同样要带认证：req = b64u(seal({op:"ws", seq}))，seq 不可重放。
    let req = query_param(&q, "req").ok_or(WebErr::Forbidden)?;
    sh.check_req(&sid, &req, "ws", None, None)?;
    let (conn, rx, shutdown) = sh.ws_attach(&sid)?;
    // 页面单条消息不超过 64KB 文本（封包后略大），限制 2MiB 防内存放大。
    Ok(ws
        .max_message_size(2 << 20)
        .max_frame_size(2 << 20)
        .on_upgrade(move |socket| ws_loop(sh, sid, conn, rx, shutdown, socket)))
}

async fn ws_loop(
    sh: Arc<Shared>,
    sid: String,
    conn: u64,
    mut out_rx: tokio::sync::mpsc::UnboundedReceiver<WsOut>,
    mut shutdown: watch::Receiver<bool>,
    socket: WebSocket,
) {
    let (mut tx, mut rx) = socket.split();
    loop {
        tokio::select! {
            out = out_rx.recv() => match out {
                Some(WsOut::Frame(b)) => {
                    if tx.send(Message::Binary(b)).await.is_err() {
                        break;
                    }
                }
                Some(WsOut::Close) | None => {
                    let _ = tx.send(Message::Close(None)).await;
                    break;
                }
            },
            inc = rx.next() => match inc {
                Some(Ok(Message::Binary(b))) => {
                    if sh.ws_incoming(&sid, &b).is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            _ = shutdown.changed() => break,
        }
    }
    sh.ws_detach(&sid, conn);
}

// ---------- 上传 / 下载 ----------

async fn upload_init(
    State(sh): State<Arc<Shared>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, WebErr> {
    let sid = session_id(&headers)?;
    let out = sh.upload_init(&sid, &body).await?;
    Ok(sealed(StatusCode::OK, out))
}

async fn upload_chunk(
    State(sh): State<Arc<Shared>>,
    headers: HeaderMap,
    Path((fid, index)): Path<(String, String)>,
    body: Bytes,
) -> Result<StatusCode, WebErr> {
    let sid = session_id(&headers)?;
    sh.put_chunk(&sid, &fid, parse_idx(&index)?, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn upload_finish(
    State(sh): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(fid): Path<String>,
) -> Result<Response, WebErr> {
    let sid = session_id(&headers)?;
    let req = header_str(&headers, REQ_HEADER).ok_or(WebErr::Forbidden)?;
    sh.check_req(&sid, req, "finish", Some(&fid), None)?;
    let out = sh.finish(&sid, &fid).await?;
    Ok(sealed(StatusCode::OK, out))
}

async fn download(
    State(sh): State<Arc<Shared>>,
    headers: HeaderMap,
    Path((fid, index)): Path<(String, String)>,
) -> Result<Response, WebErr> {
    let sid = session_id(&headers)?;
    let idx = parse_idx(&index)?;
    let req = header_str(&headers, REQ_HEADER).ok_or(WebErr::Forbidden)?;
    sh.check_req(&sid, req, "download", Some(&fid), Some(idx))?;
    let out = sh.download(&sid, &fid, idx).await?;
    Ok(sealed(StatusCode::OK, out))
}
