// 拨号与入站握手超时：TCP 已建立但应用层沉默的对端，不得永久挂起拨号方或路由循环，
// 也不得让只连接不发数据的入站连接永久占用 fd。时序整进程缩短（独立进程）。
// 注释中文，标识符英文。

mod common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use lanmsg_api::types::{MessageBodyView, Request, Response};
use lanmsg_core::discovery::MemoryBus;
use lanmsg_proto::consts::PROTO_VERSION;
use lanmsg_proto::ids::{ConvId, DeviceId, UserId};
use lanmsg_proto::sign::{sign_with_zeroed_sig, to_cbor, DeviceKey, UserKey};
use lanmsg_proto::types::{Announce, DeviceCert, DeviceType, Signature64, Status};

use common::{befriend, node, node_offline, node_offline_static, wait_until, TestNode};

const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(1);

fn init_timing() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        lanmsg_core::net::timing::set_timing_for_tests(
            Duration::from_millis(200),
            Duration::from_secs(3),
            Duration::from_millis(600),
            DIAL_TIMEOUT,
            ACCEPT_TIMEOUT,
        )
    });
}

/// 只 accept、不读不写的“沉默对端”：每个被接受的连接交给测试持有。
struct SilentPeer {
    addr: SocketAddr,
    accepted: Arc<AtomicUsize>,
    streams: mpsc::UnboundedReceiver<TcpStream>,
}

impl SilentPeer {
    async fn spawn() -> SilentPeer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let (tx, streams) = mpsc::unbounded_channel();
        let count = accepted.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                let _ = tx.send(s);
            }
        });
        SilentPeer {
            addr,
            accepted,
            streams,
        }
    }

    async fn next_stream(&mut self) -> TcpStream {
        tokio::time::timeout(Duration::from_secs(5), self.streams.recv())
            .await
            .expect("5 秒内应有拨入")
            .unwrap()
    }
}

/// 读到对端关闭（EOF 或错误）为止，返回耗时。
async fn until_closed(s: &mut TcpStream, limit: Duration) -> Duration {
    let started = Instant::now();
    let mut sink = Vec::new();
    tokio::time::timeout(limit, s.read_to_end(&mut sink))
        .await
        .unwrap_or_else(|_| panic!("{limit:?} 内连接未被关闭"))
        .ok();
    started.elapsed()
}

/// 拨号方：TCP 已建立但对端沉默，须在 DIAL_TIMEOUT 后放弃，且静态对端任务继续重试。
#[tokio::test]
async fn silent_static_peer_dial_times_out_then_retries() {
    init_timing();
    let mut silent = SilentPeer::spawn().await;
    let node = node_offline_static(vec![silent.addr]).await;

    let mut first = silent.next_stream().await;
    let took = until_closed(&mut first, DIAL_TIMEOUT + Duration::from_secs(3)).await;
    assert!(
        took >= DIAL_TIMEOUT - Duration::from_millis(500),
        "不应早于拨号超时放弃: {took:?}"
    );
    // 第二次拨号：退避 RETRY_MIN 后重试。
    let _second = silent.next_stream().await;
    assert!(silent.accepted.load(Ordering::SeqCst) >= 2);
    node.svc.shutdown().await;
}

/// 入站：只连接不发数据的客户端，ACCEPT_HANDSHAKE_TIMEOUT 后被关闭。
#[tokio::test]
async fn silent_inbound_client_closed_after_handshake_timeout() {
    init_timing();
    let node = node_offline().await;
    let port = node.svc.local_addr().port();
    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let took = until_closed(&mut client, ACCEPT_TIMEOUT + Duration::from_secs(2)).await;
    assert!(
        took >= ACCEPT_TIMEOUT - Duration::from_millis(300),
        "不应早于握手超时关闭: {took:?}"
    );
    node.svc.shutdown().await;
}

/// Service::shutdown 后，进行中的静态对端拨号立即被打断（连接被关闭，而非等到拨号超时）。
#[tokio::test]
async fn shutdown_interrupts_static_peer_dial() {
    init_timing();
    let mut silent = SilentPeer::spawn().await;
    let node = node_offline_static(vec![silent.addr]).await;
    let mut first = silent.next_stream().await;

    node.svc.shutdown().await;
    let took = until_closed(&mut first, Duration::from_secs(1)).await;
    assert!(took < Duration::from_secs(1), "{took:?}");
}

/// 构造指向 `port` 的合法通告，设备 ID 大于 `above`（使对方“较小者拨号”规则下由其拨出）。
fn forged_announce(above: DeviceId, port: u16) -> Vec<u8> {
    loop {
        let user = UserKey::generate();
        let dev = DeviceKey::generate();
        if dev.device_id() <= above {
            continue;
        }
        let cert = DeviceCert::issue(
            &user,
            dev.device_id(),
            "沉默设备".into(),
            DeviceType::Desktop,
            1_700_000_000_000,
        );
        let mut ann = Announce {
            proto: PROTO_VERSION,
            cert,
            display_name: "沉默".into(),
            avatar_hash: None,
            ports: vec![port],
            status: Status::Online,
            sig: Signature64([0u8; 64]),
        };
        sign_with_zeroed_sig(&mut ann, |b| dev.sign(b));
        return to_cbor(&ann);
    }
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

/// 发现触发的拨号对准沉默对端时：路由循环不被阻塞（另一正常对端的消息照常收发），
/// 且同一地址的重复通告不会重复发起拨号。
#[tokio::test]
async fn discovery_dial_to_silent_peer_does_not_block_routing() {
    init_timing();
    let bus = MemoryBus::new();
    // A 在总线上，B 离线仅手动互连，避免自动发现与手动拨号的双连接竞态。
    let mut a = node(&bus).await;
    let mut b = node_offline().await;
    let a_user = a.init_first("甲").await;
    let b_user = b.init_first("乙").await;
    befriend(&mut a, &mut b).await;

    let mut silent = SilentPeer::spawn().await;
    let a_dev = DeviceId::from_hex(&a.device_id().await).unwrap();
    let pkt = forged_announce(a_dev, silent.addr.port());
    bus.inject_for_tests(pkt.clone(), silent.addr);
    let mut dialed = silent.next_stream().await;
    let dial_started = Instant::now();
    // 同地址重复通告：在途拨号期间不得再拨。
    bus.inject_for_tests(pkt, silent.addr);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(silent.accepted.load(Ordering::SeqCst), 1, "在途拨号应去重");

    // 拨号挂起期间（DIAL_TIMEOUT 内），A 的路由循环仍须处理 B 发来的消息。
    let conv = ConvId::dm(
        &UserId::from_hex(&a_user).unwrap(),
        &UserId::from_hex(&b_user).unwrap(),
    )
    .to_hex();
    a.call(Request::OpenDm {
        user: b_user.clone(),
    })
    .await;
    b.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    b.call(Request::SendText {
        conv: conv.clone(),
        text: "拨号期间".into(),
    })
    .await;
    wait_until(|| async { has_text(&a, &conv, "拨号期间").await }, 2).await;
    assert!(
        dial_started.elapsed() < DIAL_TIMEOUT,
        "消息应在拨号超时前送达: {:?}",
        dial_started.elapsed()
    );

    // 拨号最终超时放弃。
    until_closed(&mut dialed, DIAL_TIMEOUT + Duration::from_secs(2)).await;
    a.svc.shutdown().await;
    b.svc.shutdown().await;
}
