// P5 Task 14：网页传输服务端集成测试。
// 测试内的 Page 扮演浏览器：reqwest 走 HTTP，tokio-tungstenite 走 WebSocket，
// 密码学直接调用 core 的 web::crypto（页面侧 API）。
// 注释中文，标识符英文。

mod common;

use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use x25519_dalek::{PublicKey, StaticSecret};

use lanmsg_api::types::{
    ConvKind, Event, MessageBodyView, MessageState, Request, Response, WebSessionInfo,
};
use lanmsg_core::web::crypto::{derive, scrypt_code, SessionKeys, WebMode};
use lanmsg_core::web::transfer::{frame_chunk, parse_chunk, CHUNK_SIZE};

use common::{node_offline, wait_until, TestNode};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// HTTP 客户端。工作区内其他 crate 会让 reqwest 使用“无内置 provider”的 rustls，
/// 需要先安装进程级 provider，否则构造客户端会 panic（本测试只走明文 HTTP）。
fn client() -> reqwest::Client {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
    reqwest::Client::new()
}

fn b64u(b: &[u8]) -> String {
    BASE64URL_NOPAD.encode(b)
}

fn unb64u(s: &str) -> Vec<u8> {
    BASE64URL_NOPAD.decode(s.as_bytes()).unwrap()
}

/// 开启网页传输并把下载目录指到临时目录（避免写进真实下载目录）。
async fn start_web(n: &TestNode) -> (WebSessionInfo, u16, [u8; 32]) {
    let dl = n.dir.path().join("dl");
    let mut st = match n.call(Request::GetSettings).await {
        Response::Settings { value } => value,
        other => panic!("expected Settings, got {other:?}"),
    };
    st.download_dir = dl.to_string_lossy().into_owned();
    n.call(Request::SetSettings { settings: st }).await;
    let info = match n.call(Request::WebStart).await {
        Response::WebSession { value } => value,
        other => panic!("expected WebSession, got {other:?}"),
    };
    let (head, key) = info.url.split_once("/#k=").expect("url has key fragment");
    let port: u16 = head.rsplit(':').next().unwrap().parse().unwrap();
    let key: [u8; 32] = unb64u(key).try_into().unwrap();
    (info, port, key)
}

/// 浏览器一侧的会话状态。
struct Page {
    base: String,
    port: u16,
    sid: String,
    keys: SessionKeys,
    seq: u64,
    http: reqwest::Client,
    host_mac: Option<Vec<u8>>,
}

impl Page {
    /// hello + confirm；返回 (Page, confirm 的 HTTP 状态)。auth 决定认证是否正确。
    async fn handshake(port: u16, mode: WebMode, auth: [u8; 32]) -> (Page, reqwest::StatusCode) {
        let http = client();
        let base = format!("http://127.0.0.1:{port}");
        let sk = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let pk = PublicKey::from(&sk).to_bytes();
        let mut nonce = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let mode_s = if mode == WebMode::Key { "key" } else { "code" };
        let r = http
            .post(format!("{base}/api/web/hello"))
            .header("User-Agent", "TestBrowser/1.0")
            .json(&json!({"mode":mode_s,"page_pk":b64u(&pk),"page_nonce":b64u(&nonce)}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let h: Value = r.json().await.unwrap();
        let sid = h["session"].as_str().unwrap().to_string();
        let host_pk: [u8; 32] = unb64u(h["host_pk"].as_str().unwrap()).try_into().unwrap();
        let host_nonce: [u8; 16] = unb64u(h["host_nonce"].as_str().unwrap())
            .try_into()
            .unwrap();
        let keys = derive(
            mode,
            &auth,
            &sk.to_bytes(),
            &host_pk,
            &pk,
            &host_pk,
            &nonce,
            &host_nonce,
            &sid,
        )
        .unwrap();
        let r = http
            .post(format!("{base}/api/web/confirm"))
            .json(&json!({"session":sid,"page_mac":b64u(&keys.page_mac())}))
            .send()
            .await
            .unwrap();
        let status = r.status();
        (
            Page {
                base,
                port,
                sid,
                keys,
                seq: 0,
                http,
                host_mac: None,
            },
            status,
        )
    }

    /// 轮询 wait 直到不再 pending，返回 status 与 host_mac。
    async fn wait(&mut self) -> String {
        loop {
            let r = self
                .http
                .get(format!("{}/api/web/wait?session={}", self.base, self.sid))
                .send()
                .await
                .unwrap();
            let j: Value = r.json().await.unwrap();
            let st = j["status"].as_str().unwrap().to_string();
            if st != "pending" {
                if st == "allowed" {
                    self.host_mac = Some(unb64u(j["host_mac"].as_str().unwrap()));
                }
                return st;
            }
        }
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn seal(&mut self, mut v: Value) -> Vec<u8> {
        v["seq"] = json!(self.next_seq());
        self.keys.seal_to_host(&serde_json::to_vec(&v).unwrap())
    }

    fn open(&self, ct: &[u8]) -> Value {
        serde_json::from_slice(&self.keys.open_from_host(ct).unwrap()).unwrap()
    }

    fn req_header(&mut self, op: &str, fid: &str, idx: Option<u32>) -> String {
        let mut v = json!({"op":op,"fid":fid});
        if let Some(i) = idx {
            v["idx"] = json!(i);
        }
        b64u(&self.seal(v))
    }

    async fn upload_init(&mut self, key: &str, name: &str, size: u64) -> Value {
        let body = self.seal(
            json!({"client_key":key,"name":name,"size":size,"mime":"application/octet-stream"}),
        );
        let r = self
            .http
            .post(format!("{}/api/web/upload/init", self.base))
            .header("X-LanMsg-Session", &self.sid)
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        self.open(&r.bytes().await.unwrap())
    }

    fn chunk_body(&mut self, fid: &str, idx: u32, data: &[u8]) -> Vec<u8> {
        let seq = self.next_seq();
        self.keys.seal_to_host(&frame_chunk(seq, fid, idx, data))
    }

    async fn put_chunk_raw(&self, fid: &str, idx: u32, body: Vec<u8>) -> reqwest::StatusCode {
        self.http
            .put(format!("{}/api/web/upload/{fid}/{idx}", self.base))
            .header("X-LanMsg-Session", &self.sid)
            .body(body)
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn put_chunk(&mut self, fid: &str, idx: u32, data: &[u8]) {
        let body = self.chunk_body(fid, idx, data);
        assert_eq!(self.put_chunk_raw(fid, idx, body).await, 204);
    }

    async fn finish(&mut self, fid: &str) -> reqwest::Response {
        let h = self.req_header("finish", fid, None);
        self.http
            .post(format!("{}/api/web/upload/{fid}/finish", self.base))
            .header("X-LanMsg-Session", &self.sid)
            .header("X-LanMsg-Req", h)
            .send()
            .await
            .unwrap()
    }

    async fn download_raw(&self, fid: &str, idx: u32, req: &str) -> reqwest::Response {
        self.http
            .get(format!("{}/api/web/download/{fid}/{idx}", self.base))
            .header("X-LanMsg-Session", &self.sid)
            .header("X-LanMsg-Req", req)
            .send()
            .await
            .unwrap()
    }

    async fn ws(&self) -> Ws {
        let url = format!(
            "ws://127.0.0.1:{}/api/web/ws?session={}",
            self.port, self.sid
        );
        tokio_tungstenite::connect_async(url).await.unwrap().0
    }

    /// 尝试建立 WebSocket 并期望握手被拒绝，返回 HTTP 状态码。
    async fn ws_refused(&self) -> u16 {
        let url = format!(
            "ws://127.0.0.1:{}/api/web/ws?session={}",
            self.port, self.sid
        );
        match tokio_tungstenite::connect_async(url).await {
            Err(tokio_tungstenite::tungstenite::Error::Http(r)) => r.status().as_u16(),
            other => panic!("expected refused handshake, got {:?}", other.map(|_| ())),
        }
    }

    async fn ws_send(&mut self, ws: &mut Ws, v: Value) {
        let b = self.seal(v);
        ws.send(Message::Binary(b)).await.unwrap();
    }

    /// 读下一帧宿主 JSON（超时 panic）。
    async fn ws_next(&self, ws: &mut Ws) -> Value {
        loop {
            let m = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .expect("timed out waiting for ws frame")
                .expect("ws closed")
                .unwrap();
            if let Message::Binary(b) = m {
                return self.open(&b);
            }
        }
    }

    /// 一直读到满足条件的帧。
    async fn ws_until(&self, ws: &mut Ws, f: impl Fn(&Value) -> bool) -> Value {
        loop {
            let v = self.ws_next(ws).await;
            if f(&v) {
                return v;
            }
        }
    }
}

/// 完整握手并被允许：返回 Page。
async fn connect_allowed(n: &mut TestNode, port: u16, key: [u8; 32]) -> Page {
    let (mut page, st) = Page::handshake(port, WebMode::Key, key).await;
    assert_eq!(st, 200);
    let sid = page.sid.clone();
    let ev = n
        .wait_event(
            |e| matches!(e, Event::WebRequest { request, .. } if *request == sid),
            5,
        )
        .await;
    if let Event::WebRequest { user_agent, ip, .. } = ev {
        assert_eq!(user_agent, "TestBrowser/1.0");
        assert_eq!(ip, "127.0.0.1");
    }
    n.call(Request::WebRespond {
        request: sid,
        allow: true,
    })
    .await;
    assert_eq!(page.wait().await, "allowed");
    page
}

async fn guest_conv(n: &TestNode) -> String {
    match n.call(Request::ListConversations).await {
        Response::Conversations { value } => {
            value
                .into_iter()
                .find(|c| c.kind == ConvKind::Guest)
                .expect("guest conversation exists")
                .conv
        }
        other => panic!("expected Conversations, got {other:?}"),
    }
}

fn dl_dir(n: &TestNode) -> std::path::PathBuf {
    n.dir.path().join("dl").join("Web uploads")
}

#[tokio::test]
async fn key_mode_handshake_and_allow() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    let (_, port, key) = start_web(&n).await;
    let page = connect_allowed(&mut n, port, key).await;
    // 页面按 host_mac 校验宿主身份。
    assert_eq!(page.host_mac.as_deref(), Some(&page.keys.host_mac()[..]));
    // 访客会话标题由 core 返回空字符串，界面按会话类型本地化。
    let convs = match n.call(Request::ListConversations).await {
        Response::Conversations { value } => value,
        other => panic!("expected Conversations, got {other:?}"),
    };
    let g = convs.iter().find(|c| c.kind == ConvKind::Guest).unwrap();
    assert_eq!(g.title, "");
}

#[tokio::test]
async fn deny_returns_denied() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    let (_, port, key) = start_web(&n).await;
    let (mut page, st) = Page::handshake(port, WebMode::Key, key).await;
    assert_eq!(st, 200);
    let sid = page.sid.clone();
    n.wait_event(
        |e| matches!(e, Event::WebRequest { request, .. } if *request == sid),
        5,
    )
    .await;
    n.call(Request::WebRespond {
        request: sid,
        allow: false,
    })
    .await;
    assert_eq!(page.wait().await, "denied");
    // 被拒绝的会话没有任何后续权限。
    assert_eq!(page.ws_refused().await, 403);
}

#[tokio::test]
async fn hello_rate_limited_per_ip() {
    let n = node_offline().await;
    n.init_first("host").await;
    let (_, port, _) = start_web(&n).await;
    let http = client();
    let mut codes = Vec::new();
    for _ in 0..11 {
        let sk = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let r = http
            .post(format!("http://127.0.0.1:{port}/api/web/hello"))
            .json(
                &json!({"mode":"key","page_pk":b64u(&PublicKey::from(&sk).to_bytes()),
                "page_nonce":b64u(&[7u8; 16])}),
            )
            .send()
            .await
            .unwrap();
        codes.push(r.status().as_u16());
    }
    assert!(codes[..10].iter().all(|c| *c == 200), "{codes:?}");
    assert_eq!(codes[10], 429);
}

#[tokio::test]
async fn code_mode_failures_rotate_code_but_success_resets_count() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    let (info, port, _) = start_web(&n).await;
    // scrypt_salt 由 hello 应答给出，先做一次 hello 取得。
    async fn code_auth(port: u16, code: &str) -> [u8; 32] {
        let sk = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let r = client()
            .post(format!("http://127.0.0.1:{port}/api/web/hello"))
            .json(
                &json!({"mode":"code","page_pk":b64u(&PublicKey::from(&sk).to_bytes()),
                "page_nonce":b64u(&[1u8; 16])}),
            )
            .send()
            .await
            .unwrap();
        let j: Value = r.json().await.unwrap();
        let salt: [u8; 16] = unb64u(j["scrypt_salt"].as_str().unwrap())
            .try_into()
            .unwrap();
        scrypt_code(code, &salt)
    }
    let good = code_auth(port, &info.code).await; // hello #1
    let mut bad = good;
    bad[0] ^= 1;
    // 4 次失败 -> 1 次成功（清零）-> 4 次失败：不得轮换；hello 总数 1+4+1+4 = 10。
    for _ in 0..4 {
        let (_, st) = Page::handshake(port, WebMode::Code, bad).await;
        assert_eq!(st, 403);
    }
    let (_, st) = Page::handshake(port, WebMode::Code, good).await;
    assert_eq!(st, 200);
    for _ in 0..4 {
        let (_, st) = Page::handshake(port, WebMode::Code, bad).await;
        assert_eq!(st, 403);
    }
    n.no_event(|e| matches!(e, Event::WebSessionChanged { .. }), 300)
        .await;
    // 限速窗口内无法继续，换用新的节点验证“连续 5 次失败轮换”。
    let mut n2 = node_offline().await;
    n2.init_first("host2").await;
    let (info2, port2, _) = start_web(&n2).await;
    let good2 = code_auth(port2, &info2.code).await;
    let mut bad2 = good2;
    bad2[0] ^= 1;
    for _ in 0..5 {
        let (_, st) = Page::handshake(port2, WebMode::Code, bad2).await;
        assert_eq!(st, 403);
    }
    let ev = n2
        .wait_event(
            |e| matches!(e, Event::WebSessionChanged { info: Some(_) }),
            5,
        )
        .await;
    if let Event::WebSessionChanged { info: Some(new) } = ev {
        assert_ne!(new.code, info2.code, "code must rotate after 5 failures");
    }
}

#[tokio::test]
async fn text_both_directions_over_ws() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    let (_, port, key) = start_web(&n).await;
    let mut page = connect_allowed(&mut n, port, key).await;
    let mut ws = page.ws().await;
    let w = page.ws_next(&mut ws).await;
    assert_eq!(w["t"], "welcome");

    // 页面 -> 宿主。
    page.ws_send(
        &mut ws,
        json!({"t":"text","cid":"c1","text":"hello from page"}),
    )
    .await;
    let ev = n
        .wait_event(|e| matches!(e, Event::MessageAdded { msg } if matches!(&msg.body, MessageBodyView::Text { text } if text == "hello from page")), 5)
        .await;
    let Event::MessageAdded { msg } = ev else {
        unreachable!()
    };
    assert_eq!(msg.state, MessageState::Delivered);
    let ack = page.ws_until(&mut ws, |v| v["t"] == "ack").await;
    assert_eq!(ack["cid"], "c1");
    let conv = guest_conv(&n).await;
    assert_eq!(msg.conv, conv);

    // 宿主 -> 页面。
    n.call(Request::SendText {
        conv: conv.clone(),
        text: "hello from host".into(),
    })
    .await;
    let m = page
        .ws_until(&mut ws, |v| v["t"] == "msg" && v["from"] == "host")
        .await;
    assert_eq!(m["text"], "hello from host");

    // 断线重连后整体重放，且 host seq 单调递增不重置。
    drop(ws);
    let mut ws2 = page.ws().await;
    let w2 = page.ws_next(&mut ws2).await;
    assert_eq!(w2["t"], "welcome");
    assert!(w2["seq"].as_u64().unwrap() > m["seq"].as_u64().unwrap());
    let replay = page.ws_until(&mut ws2, |v| v["from"] == "page").await;
    assert_eq!(replay["text"], "hello from page");

    // 文件夹不支持。
    let err = n
        .try_call(Request::SendFolder {
            conv,
            path: "/tmp".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, lanmsg_api::types::ErrorCode::InvalidArgument);
}

#[tokio::test]
async fn upload_resumes_missing_chunks_only() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    let (_, port, key) = start_web(&n).await;
    let mut page = connect_allowed(&mut n, port, key).await;

    let cs = CHUNK_SIZE as usize;
    let mut content = vec![0u8; cs * 3];
    rand::rngs::OsRng.fill_bytes(&mut content);
    let chunk = |i: usize| content[i * cs..(i + 1) * cs].to_vec();

    let ws = page.ws().await;
    let init = page
        .upload_init("k1", "big.bin", content.len() as u64)
        .await;
    assert_eq!(init["client_key"], "k1");
    assert_eq!(init["have"], json!([]));
    let fid = init["fid"].as_str().unwrap().to_string();
    page.put_chunk(&fid, 0, &chunk(0)).await;
    page.put_chunk(&fid, 2, &chunk(2)).await;
    // 断开 WS，同一 client_key 重新 init：只剩缺块。
    drop(ws);
    let init2 = page
        .upload_init("k1", "big.bin", content.len() as u64)
        .await;
    assert_eq!(init2["fid"], fid.as_str());
    assert_eq!(init2["have"], json!([0, 2]));

    // 缺块时 finish 返回 409 且回显 fid。
    let r = page.finish(&fid).await;
    assert_eq!(r.status(), 409);
    let miss = page.open(&r.bytes().await.unwrap());
    assert_eq!(miss["missing"], json!([1]));
    assert_eq!(miss["fid"], fid.as_str());

    page.put_chunk(&fid, 1, &chunk(1)).await;
    let r = page.finish(&fid).await;
    assert_eq!(r.status(), 200);
    let done = page.open(&r.bytes().await.unwrap());
    assert_eq!(done["fid"], fid.as_str());

    let got = std::fs::read(dl_dir(&n).join("big.bin")).unwrap();
    assert_eq!(got, content);
    // 宿主侧出现文件消息；part 文件已移走。
    n.wait_event(|e| matches!(e, Event::MessageAdded { msg } if matches!(msg.body, MessageBodyView::Files { .. })), 5)
        .await;
    assert!(!dl_dir(&n).join(format!(".{fid}.part")).exists());
}

#[tokio::test]
async fn host_file_downloadable_by_chunks() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    let (_, port, key) = start_web(&n).await;
    let mut page = connect_allowed(&mut n, port, key).await;
    let mut ws = page.ws().await;
    page.ws_until(&mut ws, |v| v["t"] == "welcome").await;

    let mut content = vec![0u8; CHUNK_SIZE as usize * 2 + 123];
    rand::rngs::OsRng.fill_bytes(&mut content);
    let path = n.dir.path().join("host-file.bin");
    std::fs::write(&path, &content).unwrap();
    let conv = guest_conv(&n).await;
    n.call(Request::SendFiles {
        conv,
        paths: vec![path.to_string_lossy().into_owned()],
    })
    .await;

    let m = page
        .ws_until(&mut ws, |v| v["t"] == "msg" && v["kind"] == "file")
        .await;
    let f = &m["file"];
    assert_eq!(f["name"], "host-file.bin");
    assert_eq!(f["size"], content.len() as u64);
    assert_eq!(f["chunks"], 3);
    let fid = f["fid"].as_str().unwrap().to_string();

    let mut got = Vec::new();
    let mut seqs = std::collections::HashSet::new();
    for idx in 0..3u32 {
        let h = page.req_header("download", &fid, Some(idx));
        let r = page.download_raw(&fid, idx, &h).await;
        assert_eq!(r.status(), 200);
        let ct = r.bytes().await.unwrap();
        let pt = page.keys.open_from_host(&ct).unwrap();
        let p = parse_chunk(&pt).unwrap();
        assert_eq!((p.fid, p.idx), (fid.as_str(), idx));
        assert!(seqs.insert(p.seq));
        got.extend_from_slice(p.data);
    }
    assert_eq!(got, content);

    // 重试同一分块：重新封包，seq 与密文都不同。
    let h = page.req_header("download", &fid, Some(0));
    let a = page.download_raw(&fid, 0, &h).await.bytes().await.unwrap();
    let h = page.req_header("download", &fid, Some(0));
    let b = page.download_raw(&fid, 0, &h).await.bytes().await.unwrap();
    assert_ne!(a, b);
    let sa = parse_chunk(&page.keys.open_from_host(&a).unwrap())
        .unwrap()
        .seq;
    let sb = parse_chunk(&page.keys.open_from_host(&b).unwrap())
        .unwrap()
        .seq;
    assert_ne!(sa, sb);

    // 宿主发送的消息被置为 Delivered。
    wait_until(
        || async {
            matches!(
                n.call(Request::ListMessages { conv: guest_conv(&n).await, before: None, limit: 20 }).await,
                Response::Messages { value } if value.iter().any(|m| m.is_mine && m.state == MessageState::Delivered)
            )
        },
        5,
    )
    .await;
}

#[tokio::test]
async fn request_auth_binds_op_path_and_rejects_replay() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    let (_, port, key) = start_web(&n).await;
    let mut page = connect_allowed(&mut n, port, key).await;
    let data = vec![5u8; 10];
    let init = page.upload_init("k", "a.bin", 10).await;
    let fid = init["fid"].as_str().unwrap().to_string();

    // PUT：明文里的 fid / idx 与 URL 不一致 -> 400。
    let body = page.chunk_body("other-fid", 0, &data);
    assert_eq!(page.put_chunk_raw(&fid, 0, body).await, 400);
    let body = page.chunk_body(&fid, 1, &data);
    assert_eq!(page.put_chunk_raw(&fid, 0, body).await, 400);
    // 重放同一密文 -> 拒绝。
    let body = page.chunk_body(&fid, 0, &data);
    assert_eq!(page.put_chunk_raw(&fid, 0, body.clone()).await, 204);
    assert_eq!(page.put_chunk_raw(&fid, 0, body).await, 403);

    // finish 头里的 op 与路径不一致（download 头用于 finish）-> 403。
    let h = page.req_header("download", &fid, None);
    let r = page
        .http
        .post(format!("{}/api/web/upload/{fid}/finish", page.base))
        .header("X-LanMsg-Session", &page.sid)
        .header("X-LanMsg-Req", h)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    // 重放同一个 finish 头 -> 403（第一次 200，第二次 403）。
    let h = page.req_header("finish", &fid, None);
    let send = |h: String| {
        page.http
            .post(format!("{}/api/web/upload/{fid}/finish", page.base))
            .header("X-LanMsg-Session", &page.sid)
            .header("X-LanMsg-Req", h)
            .send()
    };
    assert_eq!(send(h.clone()).await.unwrap().status(), 200);
    assert_eq!(send(h).await.unwrap().status(), 403);

    // 下载：头里的 idx 与路径不一致 -> 403。
    let h = page.req_header("download", &fid, Some(1));
    let r = page.download_raw(&fid, 0, &h).await;
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn requests_without_session_header_are_403() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    let (_, port, key) = start_web(&n).await;
    let page = connect_allowed(&mut n, port, key).await;
    let c = &page.http;
    let base = &page.base;
    let r = c
        .post(format!("{base}/api/web/upload/init"))
        .body(vec![0u8; 40])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = c
        .put(format!("{base}/api/web/upload/abc/0"))
        .body(vec![0u8; 40])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = c
        .post(format!("{base}/api/web/upload/abc/finish"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = c
        .get(format!("{base}/api/web/download/abc/0"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    // 带了一个不存在的会话头同样 403。
    let r = c
        .post(format!("{base}/api/web/upload/init"))
        .header("X-LanMsg-Session", "00".repeat(16))
        .body(vec![0u8; 40])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn security_headers_on_every_response() {
    let n = node_offline().await;
    n.init_first("host").await;
    let (_, port, _) = start_web(&n).await;
    let base = format!("http://127.0.0.1:{port}");
    let http = client();
    for path in ["/", "/app.js", "/no-such-file", "/api/web/wait?session=zz"] {
        let r = http.get(format!("{base}{path}")).send().await.unwrap();
        let h = r.headers();
        let csp = h.get("content-security-policy").unwrap().to_str().unwrap();
        assert!(csp.contains("frame-ancestors 'none'"), "{path}: {csp}");
        assert!(csp.contains("default-src 'self'"), "{path}: {csp}");
        assert_eq!(
            h.get("x-content-type-options").unwrap(),
            "nosniff",
            "{path}"
        );
        assert_eq!(h.get("referrer-policy").unwrap(), "no-referrer", "{path}");
        assert_eq!(h.get("cache-control").unwrap(), "no-store", "{path}");
    }
    let r = http.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        http.get(format!("{base}/no-such-file"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}

#[tokio::test]
async fn idle_timeout_cleans_partial_uploads() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    n.svc.set_web_idle_timeout_for_test(Duration::from_secs(1));
    let (_, port, key) = start_web(&n).await;
    let mut page = connect_allowed(&mut n, port, key).await;
    let mut ws = page.ws().await;
    page.ws_until(&mut ws, |v| v["t"] == "welcome").await;

    let init = page
        .upload_init("k", "half.bin", CHUNK_SIZE as u64 * 2)
        .await;
    let fid = init["fid"].as_str().unwrap().to_string();
    page.put_chunk(&fid, 0, &vec![1u8; CHUNK_SIZE as usize])
        .await;
    let part = dl_dir(&n).join(format!(".{fid}.part"));
    assert!(part.exists());

    // 不再发任何请求：超时后收到 bye，.part 被删除。
    let bye = page.ws_until(&mut ws, |v| v["t"] == "bye").await;
    assert_eq!(bye["t"], "bye");
    wait_until(|| async { !part.exists() }, 5).await;
    // 会话已失效。
    assert_eq!(page.ws_refused().await, 403);
}

#[tokio::test]
async fn web_stop_ends_sessions_and_emits_none() {
    let mut n = node_offline().await;
    n.init_first("host").await;
    let (_, port, key) = start_web(&n).await;
    let page = connect_allowed(&mut n, port, key).await;
    n.call(Request::WebStop).await;
    n.wait_event(|e| matches!(e, Event::WebSessionChanged { info: None }), 5)
        .await;
    wait_until(
        || async {
            client()
                .get(format!("{}/", page.base))
                .send()
                .await
                .is_err()
        },
        5,
    )
    .await;
}
