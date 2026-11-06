// 连接事件顺序：对端的帧不得先于本端对该连接的 Up 事件到达上层。
//
// 背景：拨号方 DeviceId 较大时，出站与入站两侧都要先等 DUP_DIAL_GRACE_MS 才登记并发 Up。
// 两侧宽限的结束时刻有几毫秒抖动，先登记的一侧上层立刻发 CertUpdate / SyncOffer，
// 对端读循环收到后若在自己 Up 之前就把 Frame 事件送上去，路由层 `conns.peer()` 查不到
// 该连接，整帧被静默丢弃（真实现象：离线补齐场景里 C 漏掉 A 的私聊 offer 与联系人证书，
// 之后永不收敛）。
//
// 用一个只延迟“入站方 -> 拨号方”握手字节的 TCP 代理把抖动放大成确定性的 200ms 差：
// 拨号方 Y（DeviceId 较大）晚 200ms 才完成握手，入站方 X 的 Up 与首批帧都早于 Y 的 Up。
// 注释中文，标识符英文。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

use lanmsg_core::net::{ConnEvent, ConnManager, HelloInfo, PeerInfo};
use lanmsg_core::Identity;
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::sign::{DeviceKey, UserKey};
use lanmsg_proto::types::{DeviceCert, DeviceType, Status};

/// 握手阶段对“入站方 -> 拨号方”方向每块字节的额外延迟。
const HANDSHAKE_DELAY: Duration = Duration::from_millis(200);

struct Node {
    mgr: ConnManager,
    events: mpsc::Receiver<ConnEvent>,
    device: DeviceId,
}

async fn make_node() -> Node {
    let device = DeviceKey::generate();
    let user = UserKey::generate();
    let device_id = device.device_id();
    let cert = DeviceCert::issue(
        &user,
        device_id,
        "测试设备".into(),
        DeviceType::Desktop,
        1_700_000_000_000,
    );
    let identity = Arc::new(RwLock::new(Identity {
        device: DeviceKey::from_bytes(&device.to_bytes()),
        user: None,
        cert: Some(cert.clone()),
    }));
    let (hello_tx, hello_rx) = watch::channel(HelloInfo {
        display_name: "测试".into(),
        cert: Some(cert),
        listen_port: 0,
    });
    // hello_tx 必须存活到测试结束，否则 hello_rx 读到的是已关闭通道的旧值（仍可用），这里显式泄漏。
    std::mem::forget(hello_tx);
    let (events_tx, events) = mpsc::channel(64);
    let mgr = ConnManager::bind(
        identity,
        0,
        hello_rx,
        Arc::new(|_: &PeerInfo| true),
        events_tx,
    )
    .await
    .expect("bind 失败");
    Node {
        mgr,
        events,
        device: device_id,
    }
}

/// 返回 (DeviceId 较大者, DeviceId 较小者)。
async fn make_ordered_pair() -> (Node, Node) {
    loop {
        let (a, b) = (make_node().await, make_node().await);
        if a.device > b.device {
            return (a, b);
        } else if b.device > a.device {
            return (b, a);
        }
    }
}

/// 转发到 `target` 的代理；`delay_on` 为真时，目标 -> 客户端方向的字节每块先等 HANDSHAKE_DELAY。
async fn spawn_proxy(target: SocketAddr, delay_on: Arc<AtomicBool>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("代理监听失败");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let Ok(server) = TcpStream::connect(target).await else {
                return;
            };
            let (mut cr, mut cw) = client.into_split();
            let (mut sr, mut sw) = server.into_split();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16 * 1024];
                while let Ok(n) = cr.read(&mut buf).await {
                    if n == 0 || sw.write_all(&buf[..n]).await.is_err() {
                        return;
                    }
                }
            });
            let delay_on = delay_on.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16 * 1024];
                while let Ok(n) = sr.read(&mut buf).await {
                    if n == 0 {
                        return;
                    }
                    if delay_on.load(Ordering::SeqCst) {
                        tokio::time::sleep(HANDSHAKE_DELAY).await;
                    }
                    if cw.write_all(&buf[..n]).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    addr
}

/// 入站方先于拨号方 Up 并立即发帧：拨号方上层必须先看到 Up(入站方)，再看到其 Frame。
#[tokio::test]
async fn peer_frames_never_precede_up() {
    // y：拨号方（DeviceId 较大，出站要等宽限）；x：入站方（拨号方较大，入站也要等宽限）。
    let (y, mut x) = make_ordered_pair().await;
    let Node {
        mgr: mgr_y,
        events: mut events_y,
        device: dev_y,
    } = y;
    let dev_x = x.device;

    let delay_on = Arc::new(AtomicBool::new(true));
    let proxy = spawn_proxy(x.mgr.local_addr(), delay_on.clone()).await;

    // x 一旦 Up(y) 就关掉延迟并连发几帧，模拟上层立刻发 CertUpdate / SyncOffer。
    let mgr_x = x.mgr.clone();
    let sender = tokio::spawn(async move {
        loop {
            match tokio::time::timeout(Duration::from_secs(10), x.events.recv()).await {
                Ok(Some(ConnEvent::Up(p))) if p.device == dev_y => break,
                Ok(Some(_)) => {}
                other => panic!("x 应收到 Up(y)，实际 {other:?}"),
            }
        }
        delay_on.store(false, Ordering::SeqCst);
        for _ in 0..3 {
            mgr_x
                .send(&dev_y, Frame::Presence(Status::Online))
                .await
                .expect("x 发帧失败");
        }
        x
    });

    let dial = tokio::time::timeout(Duration::from_secs(10), mgr_y.connect(proxy, false)).await;
    assert!(
        matches!(dial, Ok(Ok(d)) if d == dev_x),
        "y 拨号失败: {dial:?}"
    );
    let _x = sender.await.expect("发送任务失败");

    // y 的事件序列：针对 x 的第一个事件必须是 Up，且之后三帧都到达。
    let mut order = Vec::new();
    let mut frames = 0;
    while frames < 3 {
        match tokio::time::timeout(Duration::from_secs(5), events_y.recv()).await {
            Ok(Some(ConnEvent::Up(p))) if p.device == dev_x => order.push("Up"),
            Ok(Some(ConnEvent::Frame(d, Frame::Presence(_)))) if d == dev_x => {
                order.push("Frame");
                frames += 1;
            }
            Ok(Some(_)) => {}
            other => panic!("y 事件流中断: {other:?}，已收 {order:?}"),
        }
    }
    assert_eq!(
        order.first(),
        Some(&"Up"),
        "Frame 先于 Up 到达上层: {order:?}"
    );
    assert_eq!(order.iter().filter(|e| **e == "Up").count(), 1, "{order:?}");
}
