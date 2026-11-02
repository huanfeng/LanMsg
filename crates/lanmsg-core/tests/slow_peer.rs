// F2：慢对端不得阻塞事件循环。
// A 经可暂停的 TCP 代理连着对端 S；暂停后 A→S 方向的字节无人消费，A 对 S 的
// 写队列被占满。此时 S 发来 SyncOffer，A 需向 S 推送差量——旧实现在事件循环内
// await 推送，整个循环卡死，正常对端 B 发来的消息无法入库。
// 注释中文，标识符英文。

mod common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use lanmsg_api::types::{Event, MessageBodyView, Request, RequestAction, RequestKind, Response};
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId, UserId};
use lanmsg_proto::types::{SyncOffer, VersionVector};

use common::{befriend, node_offline, wait_until, TestNode};

/// 可暂停的转发代理：`paused` 为真时停止读取客户端（A）方向的字节。
async fn spawn_pausable_proxy(target: SocketAddr, paused: Arc<AtomicBool>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("代理监听失败");
    let addr = listener.local_addr().expect("代理地址");
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            let Ok(server) = TcpStream::connect(target).await else {
                return;
            };
            let (mut cr, mut cw) = client.into_split();
            let (mut sr, mut sw) = server.into_split();
            tokio::spawn(async move {
                let _ = tokio::io::copy(&mut sr, &mut cw).await;
            });
            let paused = paused.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16 * 1024];
                loop {
                    if paused.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        continue;
                    }
                    match cr.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if sw.write_all(&buf[..n]).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
    });
    addr
}

/// 已 link 的两节点结为联系人（与 common::befriend 相同，但不再 link）。
async fn befriend_linked(a: &mut TestNode, b: &mut TestNode) {
    a.call(Request::RequestContact { user: b.user_id() }).await;
    let ev = b
        .wait_event(
            |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, RequestKind::Contact)),
            5,
        )
        .await;
    let request = match ev {
        Event::IncomingRequest { request } => request.request,
        other => panic!("期望 IncomingRequest，实际 {other:?}"),
    };
    b.call(Request::RespondRequest {
        request,
        action: RequestAction::AddContact,
    })
    .await;
    a.wait_event(|e| matches!(e, Event::ContactsChanged), 5)
        .await;
    b.wait_event(|e| matches!(e, Event::ContactsChanged), 5)
        .await;
}

fn user(s: &str) -> UserId {
    UserId::from_hex(s).expect("非法 user_id")
}

async fn has_text(n: &TestNode, conv: &str, want: &str) -> bool {
    match n
        .call(Request::ListMessages {
            conv: conv.to_string(),
            before: None,
            limit: 50,
        })
        .await
    {
        Response::Messages { value } => value
            .iter()
            .any(|m| matches!(&m.body, MessageBodyView::Text { text } if text == want)),
        other => panic!("期望 Messages，实际 {other:?}"),
    }
}

/// A 对慢对端 S 的写队列占满且 S 请求差量推送时，正常对端 B 发给 A 的消息
/// 仍须在 5 秒内入库。
#[tokio::test]
async fn slow_peer_does_not_block_other_peers() {
    let mut a = node_offline().await;
    let mut s = node_offline().await;
    let mut b = node_offline().await;
    let a_user = a.init_first("甲").await;
    let s_user = s.init_first("慢").await;
    let b_user = b.init_first("乙").await;
    let a_dev = a.device_id().await;

    // A 经代理连 S（A 拨代理），结为联系人；A 与 B 直连并结为联系人。
    let paused = Arc::new(AtomicBool::new(false));
    let proxy = spawn_pausable_proxy(s.svc.local_addr(), paused.clone()).await;
    let s_dev = s.device_id().await;
    a.call(Request::AddManualPeer {
        addr: proxy.to_string(),
    })
    .await;
    wait_until(
        || async {
            matches!(
                s.call(Request::ListPeers).await,
                Response::Peers { value } if value.iter().any(|p| p.device_id == a_dev)
            ) && matches!(
                a.call(Request::ListPeers).await,
                Response::Peers { value } if value.iter().any(|p| p.device_id == s_dev)
            )
        },
        5,
    )
    .await;
    befriend_linked(&mut a, &mut s).await;
    befriend(&mut a, &mut b).await;

    // A 在与 S 的会话里有若干消息（S 的向量为空时需整批推送）。
    let as_conv = ConvId::dm(&user(&a_user), &user(&s_user));
    a.call(Request::OpenDm {
        user: s_user.clone(),
    })
    .await;
    for i in 0..3 {
        a.call(Request::SendText {
            conv: as_conv.to_hex(),
            text: format!("给S的消息{i}"),
        })
        .await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 暂停代理：A→S 无人消费；A 持续向 S 发大帧，直到写队列占满（计数不再增长）。
    paused.store(true, Ordering::SeqCst);
    let sent = Arc::new(AtomicUsize::new(0));
    let flood = {
        let svc = a.svc.clone();
        let sent = sent.clone();
        let s_id = DeviceId::from_hex(&s_dev).expect("S 设备 ID");
        tokio::spawn(async move {
            let big = "x".repeat(64 * 1024);
            loop {
                let frame = Frame::Error {
                    code: 0,
                    message: big.clone(),
                };
                if svc.send_frame_for_test(&s_id, frame).await.is_err() {
                    return;
                }
                sent.fetch_add(1, Ordering::SeqCst);
            }
        })
    };
    let mut last = usize::MAX;
    let mut stable = 0;
    while stable < 3 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let now = sent.load(Ordering::SeqCst);
        stable = if now == last { stable + 1 } else { 0 };
        last = now;
    }
    assert!(last > 256, "应已占满写队列，实际仅发出 {last} 帧");

    // S 请求差量（空向量）：A 须向 S 推送，写队列已满而阻塞。
    let a_id = DeviceId::from_hex(&a_dev).expect("A 设备 ID");
    s.svc
        .send_frame_for_test(
            &a_id,
            Frame::SyncOffer(SyncOffer {
                conv: as_conv,
                vv: VersionVector(Default::default()),
                meta: None,
            }),
        )
        .await
        .expect("S 发 SyncOffer 失败");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // B 给 A 发消息：5 秒内 A 必须收到。
    let ba_conv = ConvId::dm(&user(&b_user), &user(&a_user)).to_hex();
    b.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    b.call(Request::SendText {
        conv: ba_conv.clone(),
        text: "来自B".into(),
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !has_text(&a, &ba_conv, "来自B").await {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("慢对端阻塞了事件循环：5 秒内 A 未收到 B 的消息");

    flood.abort();
}
