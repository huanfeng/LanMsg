use std::path::PathBuf;
use std::time::Duration;

use lanmsg_localsend::{
    lenient_client, LocalSend, LsConfig, LsDecision, LsError, LsEvent, LsPeer,
    PrepareUploadRequest, PrepareUploadResponse,
};
use tokio::sync::mpsc;

struct Node {
    ls: LocalSend,
    dir: tempfile::TempDir,
    /// 应答任务转发出来的、已处理过的事件
    seen: mpsc::UnboundedReceiver<LsEvent>,
}

impl Node {
    fn download_dir(&self) -> PathBuf {
        self.dir.path().join("dl")
    }
    fn peer(&self) -> LsPeer {
        LsPeer {
            info: self.ls.info(),
            ip: "127.0.0.1".parse().unwrap(),
        }
    }
}

fn config(dir: &tempfile::TempDir, http_port: u16) -> LsConfig {
    LsConfig {
        alias: "test".into(),
        device_model: "model".into(),
        device_type: "desktop".into(),
        download_dir: dir.path().join("dl"),
        cert_dir: dir.path().join("cert"),
        http_port,
        mcast_port: 53317,
        enable_discovery: false,
    }
}

/// 启动节点，所有接收请求按 `decision` 自动应答。
async fn node(decision: LsDecision) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    let ls = LocalSend::start(config(&dir, 0), tx).await.unwrap();
    assert!(ls.listening());
    let (seen_tx, seen) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                LsEvent::IncomingFiles {
                    session,
                    peer,
                    files,
                    reply,
                } => {
                    let _ = reply.send(decision);
                    let _ = seen_tx.send(LsEvent::IncomingFiles {
                        session,
                        peer,
                        files,
                        reply: tokio::sync::oneshot::channel().0,
                    });
                }
                LsEvent::TextReceived { peer, text, reply } => {
                    let _ = reply.send(decision);
                    let _ = seen_tx.send(LsEvent::TextReceived {
                        peer,
                        text,
                        reply: tokio::sync::oneshot::channel().0,
                    });
                }
                other => {
                    let _ = seen_tx.send(other);
                }
            }
        }
    });
    Node { ls, dir, seen }
}

async fn next_matching(n: &mut Node, f: impl Fn(&LsEvent) -> bool) -> LsEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ev = n.seen.recv().await.unwrap();
            if f(&ev) {
                return ev;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn send_text_is_accepted_with_204() {
    let a = node(LsDecision::Accept).await;
    let mut b = node(LsDecision::Accept).await;
    a.ls.send_text(&b.peer(), "hello 你好").await.unwrap();
    let ev = next_matching(&mut b, |e| matches!(e, LsEvent::TextReceived { .. })).await;
    let LsEvent::TextReceived { text, .. } = ev else {
        unreachable!()
    };
    assert_eq!(text, "hello 你好");
}

#[tokio::test]
async fn send_file_roundtrip_and_sanitized_name() {
    let a = node(LsDecision::Accept).await;
    let mut b = node(LsDecision::Accept).await;
    let src = a.dir.path().join("evil.txt");
    let content = vec![7u8; 300_000];
    std::fs::write(&src, &content).unwrap();
    let (ptx, mut prx) = mpsc::channel(256);
    a.ls.send_files(&b.peer(), &[src], ptx).await.unwrap();
    let ev = next_matching(&mut b, |e| matches!(e, LsEvent::FileReceived { .. })).await;
    let LsEvent::FileReceived { path, .. } = ev else {
        unreachable!()
    };
    assert_eq!(std::fs::read(&path).unwrap(), content);
    assert_eq!(path.parent().unwrap(), b.download_dir());
    next_matching(&mut b, |e| {
        matches!(e, LsEvent::SessionFinished { ok: true, .. })
    })
    .await;
    assert!(prx.try_recv().is_ok());
    // 再发一次，重名追加序号
    let src2 = a.dir.path().join("evil.txt");
    let (ptx, _prx) = mpsc::channel(256);
    a.ls.send_files(&b.peer(), &[src2], ptx).await.unwrap();
    assert!(b.download_dir().join("evil (1).txt").exists());
}

fn raw_request(
    peer: &LsPeer,
    from: &LsPeer,
    name: &str,
    sha: Option<&str>,
) -> PrepareUploadRequest {
    let _ = peer;
    serde_json::from_value(serde_json::json!({
        "info": from.info,
        "files": { "f1": { "id": "f1", "fileName": name, "size": 3, "fileType": "text/plain",
                           "sha256": sha } }
    }))
    .unwrap()
}

async fn raw_prepare(
    a: &Node,
    b: &Node,
    name: &str,
    sha: Option<&str>,
) -> (reqwest::Client, String, PrepareUploadResponse) {
    let client = lenient_client().unwrap();
    let req = raw_request(&b.peer(), &a.peer(), name, sha);
    let url = format!("https://127.0.0.1:{}/api/localsend/v2", b.ls.info().port);
    let resp = client
        .post(format!("{url}/prepare-upload"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    (client, url, resp.json().await.unwrap())
}

#[tokio::test]
async fn path_traversal_name_is_sanitized() {
    let a = node(LsDecision::Accept).await;
    let b = node(LsDecision::Accept).await;
    let (client, url, resp) = raw_prepare(&a, &b, "../evil.txt", None).await;
    let r = client
        .post(format!("{url}/upload"))
        .query(&[
            ("sessionId", resp.session_id.as_str()),
            ("fileId", "f1"),
            ("token", resp.files["f1"].as_str()),
        ])
        .body("abc")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        std::fs::read(b.download_dir().join("evil.txt")).unwrap(),
        b"abc"
    );
    assert!(!b.dir.path().join("evil.txt").exists());
}

#[tokio::test]
async fn reject_returns_403() {
    let a = node(LsDecision::Accept).await;
    let b = node(LsDecision::Reject).await;
    let err = a.ls.send_text(&b.peer(), "x").await.unwrap_err();
    assert!(matches!(err, LsError::Status(403)), "{err:?}");
    let src = a.dir.path().join("f.bin");
    std::fs::write(&src, b"abc").unwrap();
    let (ptx, _prx) = mpsc::channel(8);
    let err = a.ls.send_files(&b.peer(), &[src], ptx).await.unwrap_err();
    assert!(matches!(err, LsError::Status(403)), "{err:?}");
}

#[tokio::test]
async fn upload_with_wrong_token_is_403_and_missing_params_400() {
    let a = node(LsDecision::Accept).await;
    let b = node(LsDecision::Accept).await;
    let (client, url, resp) = raw_prepare(&a, &b, "x.txt", None).await;
    let r = client
        .post(format!("{url}/upload"))
        .query(&[
            ("sessionId", resp.session_id.as_str()),
            ("fileId", "f1"),
            ("token", "wrong"),
        ])
        .body("abc")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = client
        .post(format!("{url}/upload"))
        .body("abc")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn sha256_mismatch_is_422_and_cleans_part() {
    let a = node(LsDecision::Accept).await;
    let b = node(LsDecision::Accept).await;
    let (client, url, resp) = raw_prepare(&a, &b, "x.txt", Some(&"0".repeat(64))).await;
    let r = client
        .post(format!("{url}/upload"))
        .query(&[
            ("sessionId", resp.session_id.as_str()),
            ("fileId", "f1"),
            ("token", resp.files["f1"].as_str()),
        ])
        .body("abc")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    let leftovers: Vec<_> = std::fs::read_dir(b.download_dir()).unwrap().collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test]
async fn second_session_is_409_and_cancel_frees_slot() {
    let a = node(LsDecision::Accept).await;
    let mut b = node(LsDecision::Accept).await;
    let (client, url, resp) = raw_prepare(&a, &b, "x.txt", None).await;
    let req = raw_request(&b.peer(), &a.peer(), "y.txt", None);
    let r = client
        .post(format!("{url}/prepare-upload"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    let r = client
        .post(format!("{url}/cancel"))
        .query(&[("sessionId", resp.session_id.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    next_matching(&mut b, |e| {
        matches!(e, LsEvent::SessionFinished { ok: false, .. })
    })
    .await;
    let r = client
        .post(format!("{url}/prepare-upload"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn register_records_peer_and_returns_own_info() {
    let a = node(LsDecision::Accept).await;
    let b = node(LsDecision::Accept).await;
    let client = lenient_client().unwrap();
    let url = format!(
        "https://127.0.0.1:{}/api/localsend/v2/register",
        b.ls.info().port
    );
    let info: lanmsg_localsend::LsInfo = client
        .post(url)
        .json(&a.ls.info())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(info.fingerprint, b.ls.info().fingerprint);
    assert_eq!(b.ls.peers().len(), 1);
}

#[tokio::test]
async fn port_busy_disables_announce_but_keeps_sending() {
    let occupied = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let busy_port = occupied.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel(8);
    let ls = LocalSend::start(config(&dir, busy_port), tx).await.unwrap();
    assert!(!ls.listening());

    let mut b = node(LsDecision::Accept).await;
    ls.send_text(&b.peer(), "from busy").await.unwrap();
    let ev = next_matching(&mut b, |e| matches!(e, LsEvent::TextReceived { .. })).await;
    let LsEvent::TextReceived { text, .. } = ev else {
        unreachable!()
    };
    assert_eq!(text, "from busy");
}
