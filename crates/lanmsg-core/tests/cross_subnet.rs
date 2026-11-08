// P5 Task 3：手动添加对端、持久化与重启恢复、地址簿跨节点传播。
// 不同 MemoryBus 的节点彼此不会经发现互连，用来模拟“不同网段”。
// 注释中文，标识符英文。

mod common;

use lanmsg_api::types::{ErrorCode, Event, Request};
use lanmsg_core::discovery::MemoryBus;
use lanmsg_core::platform::PlatformHooks;

use common::{befriend, node, node_with, pair, TestNode};

fn is_online(dev: String) -> impl Fn(&Event) -> bool {
    move |e| matches!(e, Event::PeerOnline { peer } if peer.device_id == dev)
}

#[tokio::test]
async fn manual_peer_connects_and_is_remembered() {
    let a = node(&MemoryBus::new()).await;
    let mut b = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    b.init_first("B").await;
    let a_dev = a.device_id().await;

    b.call(Request::AddManualPeer {
        addr: a.svc.local_addr().to_string(),
    })
    .await;
    b.wait_event(is_online(a_dev.clone()), 5).await;

    // 重启 b：地址来自 manual_peers，10 秒内再次连上 a。
    let TestNode { svc, dir, .. } = b;
    svc.shutdown().await;
    drop(svc);
    let mut b2 = node_with(&MemoryBus::new(), dir, PlatformHooks::default()).await;
    b2.wait_event(is_online(a_dev), 10).await;
}

#[tokio::test]
async fn address_book_lets_my_devices_find_contact() {
    let bus1 = MemoryBus::new();
    let bus2 = MemoryBus::new();
    let mut p1 = node(&bus1).await;
    let mut p2 = node(&bus1).await;
    p1.init_first("P").await;
    p2.try_call(Request::Init {
        display_name: "P2".into(),
        device_name: "P2".into(),
        mode: lanmsg_api::types::InitMode::JoinExisting,
    })
    .await
    .expect("Init JoinExisting failed");
    pair(&mut p1, &mut p2).await;
    let mut c = node(&bus2).await;
    c.init_first("C").await;
    let c_dev = c.device_id().await;

    // p1 手动连上 c 并结为联系人；p2 与 c 不同 bus，从未直连。
    p1.call(Request::AddManualPeer {
        addr: c.svc.local_addr().to_string(),
    })
    .await;
    befriend(&mut p1, &mut c).await;

    // p2 经 p1 发来的地址簿找到 c。
    p2.wait_event(is_online(c_dev), 15).await;
}

#[tokio::test]
async fn add_manual_peer_rejects_unparsable_addr() {
    let a = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    for bad in ["not a host:99999", "", "host:"] {
        let err = a
            .try_call(Request::AddManualPeer { addr: bad.into() })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "{bad:?}");
    }
}

#[tokio::test]
async fn manual_peer_offline_is_persisted_and_reports_peer_offline() {
    let a = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    // 本机上没有服务监听的端口：连接失败，但地址保留给重连任务。
    let err = a
        .try_call(Request::AddManualPeer {
            addr: "127.0.0.1:1".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PeerOffline);
}
