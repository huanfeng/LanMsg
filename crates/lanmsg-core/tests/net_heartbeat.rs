// 连接保活：对端无 FIN 消失（断电 / 切网 / 进程被杀）后的检测与恢复。
// 用可“黑洞化”的 TCP 代理模拟：对既有连接只吞字节、不关 socket（不产生 FIN / RST），
// 新连接照常转发。时序整进程缩短（每个集成测试文件是独立进程）。
// 注释中文，标识符英文。

mod common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use lanmsg_api::types::{Event, MessageBodyView, Request, RequestAction, RequestKind, Response};
use lanmsg_proto::ids::{ConvId, UserId};

use common::{node_offline, wait_until, TestNode};

// 心跳 200ms / 空闲超时 3s / 替换阈值 600ms：
// 替换阈值远小于空闲超时，可区分“新连接替换”与“空闲超时”两条路径。
const HEARTBEAT: Duration = Duration::from_millis(200);
const IDLE_TIMEOUT: Duration = Duration::from_secs(3);
const REPLACE_IDLE: Duration = Duration::from_millis(600);

fn init_timing() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        lanmsg_core::net::timing::set_timing_for_tests(HEARTBEAT, IDLE_TIMEOUT, REPLACE_IDLE)
    });
}

/// 可黑洞化的代理。
struct Proxy {
    addr: SocketAddr,
    /// 每次 blackhole_existing 加一；连接建立时记下当时的值，之后值变大即被黑洞化。
    generation: Arc<AtomicUsize>,
    /// 为真时新入连接立即丢弃（模拟对端暂时不可达）。
    refuse_new: Arc<AtomicBool>,
}

impl Proxy {
    async fn spawn(target: SocketAddr) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("代理监听失败");
        let addr = listener.local_addr().unwrap();
        let generation = Arc::new(AtomicUsize::new(0));
        let refuse_new = Arc::new(AtomicBool::new(false));
        let (g, r) = (generation.clone(), refuse_new.clone());
        tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                if r.load(Ordering::SeqCst) {
                    continue; // 直接丢弃 client（连接被关闭）
                }
                let Ok(server) = TcpStream::connect(target).await else {
                    return;
                };
                let born = g.load(Ordering::SeqCst);
                let (cr, cw) = client.into_split();
                let (sr, sw) = server.into_split();
                tokio::spawn(forward(sr, cw, g.clone(), born));
                tokio::spawn(forward(cr, sw, g.clone(), born));
            }
        });
        Proxy {
            addr,
            generation,
            refuse_new,
        }
    }

    /// 既有连接从此只吞字节，不关闭、不转发。
    fn blackhole_existing(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

async fn forward(
    mut from: impl AsyncRead + Unpin,
    mut to: impl AsyncWrite + Unpin,
    generation: Arc<AtomicUsize>,
    born: usize,
) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if generation.load(Ordering::SeqCst) != born {
                    continue; // 黑洞：丢弃
                }
                if to.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
        }
    }
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

/// 已互联的两节点结为联系人。
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

async fn sees_peer(n: &TestNode, device: &str) -> bool {
    matches!(
        n.call(Request::ListPeers).await,
        Response::Peers { value } if value.iter().any(|p| p.device_id == device)
    )
}

/// A 经代理连 B 并结为联系人，返回 (a, b, 代理, a_dev, b_dev, dm 会话)。
async fn linked_via_proxy() -> (TestNode, TestNode, Proxy, String, String, String) {
    init_timing();
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    let a_user = a.init_first("甲").await;
    let b_user = b.init_first("乙").await;
    let (a_dev, b_dev) = (a.device_id().await, b.device_id().await);
    let proxy = Proxy::spawn(b.svc.local_addr()).await;
    a.call(Request::AddManualPeer {
        addr: proxy.addr.to_string(),
    })
    .await;
    wait_until(
        || async { sees_peer(&a, &b_dev).await && sees_peer(&b, &a_dev).await },
        5,
    )
    .await;
    befriend_linked(&mut a, &mut b).await;
    let conv = ConvId::dm(&user(&a_user), &user(&b_user)).to_hex();
    a.call(Request::OpenDm {
        user: b_user.clone(),
    })
    .await;
    (a, b, proxy, a_dev, b_dev, conv)
}

/// 双向互发消息，验证连接真正可用。
async fn assert_messages_flow(a: &TestNode, b: &TestNode, conv: &str, tag: &str) {
    b.call(Request::OpenDm { user: a.user_id() }).await;
    let (to_b, to_a) = (format!("{tag}-甲→乙"), format!("{tag}-乙→甲"));
    a.call(Request::SendText {
        conv: conv.to_string(),
        text: to_b.clone(),
    })
    .await;
    b.call(Request::SendText {
        conv: conv.to_string(),
        text: to_a.clone(),
    })
    .await;
    wait_until(
        || async { has_text(b, conv, &to_b).await && has_text(a, conv, &to_a).await },
        8,
    )
    .await;
}

/// 在 window 内收集指定对端的上下线事件序列（true = 上线）。
async fn collect_presence(n: &mut TestNode, device: &str, window: Duration) -> Vec<bool> {
    let mut seq = Vec::new();
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(ev) = tokio::time::timeout_at(deadline, n.events.recv()).await {
        match ev {
            Ok(Event::PeerOnline { peer }) if peer.device_id == device => seq.push(true),
            Ok(Event::PeerOffline { device: d }) if d == device => seq.push(false),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            _ => {}
        }
    }
    seq
}

/// 对端无 FIN 消失、且短时无法重连：双方须在空闲超时内都收到 Down；
/// 对端恢复后自动重连（A 的静态对端任务）并能互发消息。
#[tokio::test]
async fn silent_peer_detected_by_idle_timeout_then_recovers() {
    let (mut a, mut b, proxy, a_dev, b_dev, conv) = linked_via_proxy().await;
    assert_messages_flow(&a, &b, &conv, "断前").await;

    proxy.refuse_new.store(true, Ordering::SeqCst);
    proxy.blackhole_existing();
    let started = std::time::Instant::now();

    // 双方都应在 IDLE_TIMEOUT 附近收到 Down（给 2s 余量；不早于心跳间隔，排除误判）。
    a.wait_event(
        |e| matches!(e, Event::PeerOffline { device } if *device == b_dev),
        5,
    )
    .await;
    b.wait_event(
        |e| matches!(e, Event::PeerOffline { device } if *device == a_dev),
        5,
    )
    .await;
    let took = started.elapsed();
    assert!(
        took >= IDLE_TIMEOUT - Duration::from_millis(500),
        "不应早于空闲超时判死: {took:?}"
    );
    assert!(!sees_peer(&a, &b_dev).await && !sees_peer(&b, &a_dev).await);

    // 对端“恢复”：代理重新转发，A 的静态对端任务退避重拨。
    proxy.refuse_new.store(false, Ordering::SeqCst);
    wait_until(
        || async { sees_peer(&a, &b_dev).await && sees_peer(&b, &a_dev).await },
        10,
    )
    .await;
    assert_messages_flow(&a, &b, &conv, "恢复后").await;
}

/// 对端重启 / 换网后以同一设备身份立刻重新拨入（旧连接尚未被空闲超时判死）：
/// 新连接须替换旧连接——上层恰好看到一次 Down 再一次 Up，且消息可互通。
/// 这里 A 是原拨号方，旧实现因 `existing_dialer <= new_dialer` 会永远丢弃重连。
#[tokio::test]
async fn redial_replaces_silent_connection_with_down_then_up() {
    let (mut a, mut b, proxy, a_dev, b_dev, conv) = linked_via_proxy().await;
    assert_messages_flow(&a, &b, &conv, "断前").await;
    // 清掉建链阶段积压的事件，只看黑洞之后的序列。
    let _ = collect_presence(&mut a, &b_dev, Duration::from_millis(100)).await;
    let _ = collect_presence(&mut b, &a_dev, Duration::from_millis(100)).await;

    proxy.blackhole_existing(); // 新连接仍可转发

    // 窗口远小于 IDLE_TIMEOUT：能完成 Down+Up 只可能走“新连接替换”路径。
    let window = IDLE_TIMEOUT - Duration::from_millis(800);
    let (seq_a, seq_b) = tokio::join!(
        collect_presence(&mut a, &b_dev, window),
        collect_presence(&mut b, &a_dev, window)
    );
    assert_eq!(seq_a, vec![false, true], "A 侧应恰好 Down 再 Up: {seq_a:?}");
    assert_eq!(seq_b, vec![false, true], "B 侧应恰好 Down 再 Up: {seq_b:?}");

    assert_messages_flow(&a, &b, &conv, "替换后").await;
}

/// 存活连接上的重复拨号（双方互设静态对端、静态任务周期重拨）被裁决丢弃：
/// 全程不产生任何 Down，也不产生重复 Up。
#[tokio::test]
async fn redial_of_live_connection_never_flaps() {
    init_timing();
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    let (a_dev, b_dev) = (a.device_id().await, b.device_id().await);
    let (a_addr, b_addr) = (a.svc.local_addr(), b.svc.local_addr());
    // 用回环地址而非 0.0.0.0，保证两侧都互设静态对端。
    let loop_a = SocketAddr::from(([127, 0, 0, 1], a_addr.port()));
    let loop_b = SocketAddr::from(([127, 0, 0, 1], b_addr.port()));
    a.call(Request::AddManualPeer {
        addr: loop_b.to_string(),
    })
    .await;
    b.call(Request::AddManualPeer {
        addr: loop_a.to_string(),
    })
    .await;
    wait_until(
        || async { sees_peer(&a, &b_dev).await && sees_peer(&b, &a_dev).await },
        5,
    )
    .await;

    // 观察 3.5 个静态重拨周期（含多次心跳）。
    let (seq_a, seq_b) = tokio::join!(
        collect_presence(&mut a, &b_dev, Duration::from_millis(3500)),
        collect_presence(&mut b, &a_dev, Duration::from_millis(3500))
    );
    // 起点之前已发生的 Up 可能仍在通道里（最多一次），之后不得有 Down 或第二次 Up。
    assert!(
        seq_a.iter().filter(|up| !**up).count() == 0 && seq_a.len() <= 1,
        "{seq_a:?}"
    );
    assert!(
        seq_b.iter().filter(|up| !**up).count() == 0 && seq_b.len() <= 1,
        "{seq_b:?}"
    );
}
