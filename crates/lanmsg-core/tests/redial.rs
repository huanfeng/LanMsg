// 受信对端主动重连：无组播发现（Disabled）下，断开后任一方都按已知地址拨回；
// 地址持久化后重启仍可重连；网络变化入口清零退避并立即重拨。
// 用 connect_for_test 建链，避免静态对端（AddManualPeer）的重连掩盖受信重连。
// 注释中文，标识符英文。

mod common;

use std::time::Duration;

use lanmsg_api::types::{Event, Request, RequestAction, RequestKind, Response};
use lanmsg_core::platform::PlatformHooks;
use lanmsg_proto::ids::DeviceId;

use common::{node_offline, node_offline_with, wait_until, TestNode};

async fn sees_peer(n: &TestNode, device: &str) -> bool {
    matches!(
        n.call(Request::ListPeers).await,
        Response::Peers { value } if value.iter().any(|p| p.device_id == device)
    )
}

async fn sees_no_peer(n: &TestNode, device: &str) -> bool {
    !sees_peer(n, device).await
}

fn dev(id: &str) -> DeviceId {
    DeviceId::from_hex(id).expect("非法 device_id")
}

/// 两个已初始化节点：A 拨 B（不登记静态对端），结为联系人，等双方互见。
async fn befriended_pair() -> (TestNode, TestNode, String, String) {
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    let (a_dev, b_dev) = (a.device_id().await, b.device_id().await);
    a.svc
        .connect_for_test(b.svc.local_addr())
        .await
        .expect("拨号失败");
    wait_until(
        || async { sees_peer(&a, &b_dev).await && sees_peer(&b, &a_dev).await },
        5,
    )
    .await;
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
    (a, b, a_dev, b_dev)
}

#[tokio::test]
async fn trusted_peers_reconnect_after_disconnect_without_discovery() {
    let (a, b, a_dev, b_dev) = befriended_pair().await;
    a.svc.disconnect_for_test(&dev(&b_dev)).await;
    wait_until(|| async { sees_no_peer(&a, &b_dev).await }, 5).await;
    // 退避从 1s 起步：10 秒内必须自动回来，且双方都看得到对方。
    wait_until(
        || async { sees_peer(&a, &b_dev).await && sees_peer(&b, &a_dev).await },
        10,
    )
    .await;
}

/// 重启一方（监听端口会变）：它凭持久化的对端地址拨回仍在运行的另一方。
async fn restart_and_expect_reconnect(restart_a: bool) {
    let (a, b, a_dev, b_dev) = befriended_pair().await;
    let (restarting, staying, staying_dev, restarting_dev) = if restart_a {
        (a, b, b_dev, a_dev)
    } else {
        (b, a, a_dev, b_dev)
    };
    let dir = {
        let TestNode { svc, dir, .. } = restarting;
        svc.shutdown().await;
        dir
    };
    wait_until(
        || async { sees_no_peer(&staying, &restarting_dev).await },
        8,
    )
    .await;
    let reborn = node_offline_with(dir, PlatformHooks::default()).await;
    wait_until(
        || async {
            sees_peer(&reborn, &staying_dev).await && sees_peer(&staying, &restarting_dev).await
        },
        15,
    )
    .await;
}

#[tokio::test]
async fn restarted_smaller_or_larger_side_redials_from_persisted_address() {
    // 两个方向都要覆盖：不论重启的是 DeviceId 较小还是较大的一方都能拨回。
    restart_and_expect_reconnect(true).await;
    restart_and_expect_reconnect(false).await;
}

/// A 对已关闭的 B 持续拨号失败，直到累计尝试达到 `n`（退避随之增长）。
async fn wait_attempts(a: &TestNode, n: u64) {
    wait_until(|| async move { a.svc.redial_attempts_for_test() >= n }, 40).await;
}

#[tokio::test]
async fn network_changed_redials_without_waiting_for_the_grown_backoff() {
    let (a, b, _a_dev, b_dev) = befriended_pair().await;
    // B 永久关闭：A 的拨号持续失败，退避逐步增长。
    b.svc.shutdown().await;
    wait_until(|| async { sees_no_peer(&a, &b_dev).await }, 8).await;
    // 第 5 次失败后下一次拨号至少 8s 后；网络变化（2s 去抖）必须让它早得多发生。
    wait_attempts(&a, 5).await;
    let before = a.svc.redial_attempts_for_test();
    a.call(Request::NetworkChanged).await;
    let deadline = std::time::Instant::now() + Duration::from_millis(4500);
    while a.svc.redial_attempts_for_test() == before {
        assert!(
            std::time::Instant::now() < deadline,
            "NetworkChanged 后没有提前重拨"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn network_changed_bursts_are_debounced_into_one_signal() {
    let a = node_offline().await;
    for _ in 0..5 {
        a.call(Request::NetworkChanged).await;
    }
    assert_eq!(a.svc.redial_net_signals_for_test(), 0, "窗口内不应立即触发");
    tokio::time::sleep(Duration::from_millis(3000)).await;
    assert_eq!(
        a.svc.redial_net_signals_for_test(),
        1,
        "多次通知应合并成一次"
    );
    a.call(Request::NetworkChanged).await;
    tokio::time::sleep(Duration::from_millis(3000)).await;
    assert_eq!(
        a.svc.redial_net_signals_for_test(),
        2,
        "窗口过后的新通知再触发一次"
    );
}

#[tokio::test]
async fn dialing_checks_the_expected_device_and_leaks_nothing_on_mismatch() {
    let a = node_offline().await;
    let mut b = node_offline().await;
    let c = node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    let c_dev = dev(&c.device_id().await);
    let err = a
        .svc
        .connect_expecting_for_test(b.svc.local_addr(), c_dev)
        .await;
    assert!(err.is_err(), "期望设备与实际不符时必须拒绝");
    // 对端没有收到 Hello，因此不会登记、不会上线。
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(sees_no_peer(&b, &a.device_id().await).await, "B 不应看到 A");
    assert!(sees_no_peer(&a, &b.device_id().await).await, "A 不应看到 B");
    b.no_event(|e| matches!(e, Event::PeerOnline { .. }), 300)
        .await;
    // 期望正确时照常连上。
    let b_dev = dev(&b.device_id().await);
    a.svc
        .connect_expecting_for_test(b.svc.local_addr(), b_dev)
        .await
        .expect("期望正确时应连上");
}

#[tokio::test]
async fn revoked_device_is_not_dialed_again_after_backoff() {
    let (a, b, _a_dev, b_dev) = befriended_pair().await;
    b.svc.shutdown().await;
    wait_until(|| async { sees_no_peer(&a, &b_dev).await }, 8).await;
    wait_attempts(&a, 2).await;
    // 此刻 A 的任务在退避中；吊销后即使退避结束也不能再拨。
    a.svc.store_for_test().revoke_device(&dev(&b_dev)).unwrap();
    let after_revoke = a.svc.redial_attempts_for_test();
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert_eq!(a.svc.redial_attempts_for_test(), after_revoke);
}

#[tokio::test]
async fn invisible_does_not_dial_non_self_trusted_devices() {
    let (a, b, _a_dev, b_dev) = befriended_pair().await;
    a.call(Request::SetStatus {
        status: lanmsg_api::types::StatusDto::Invisible,
    })
    .await;
    b.svc.shutdown().await;
    wait_until(|| async { sees_no_peer(&a, &b_dev).await }, 8).await;
    tokio::time::sleep(Duration::from_millis(3000)).await;
    assert_eq!(a.svc.redial_attempts_for_test(), 0, "隐身时不应外联联系人");
}

#[tokio::test]
async fn address_outside_local_subnets_is_not_dialed() {
    let (a, b, _a_dev, b_dev) = befriended_pair().await;
    // 把 B 的持久化地址改成不在任何本机网段的地址（模拟换了网络）。
    let store = a.svc.store_for_test();
    let mut row = store.get_device(&dev(&b_dev)).unwrap().expect("B 应已入库");
    row.last_addr = Some("203.0.113.9:7000".to_string());
    store.upsert_device(&row).unwrap();
    b.svc.shutdown().await;
    wait_until(|| async { sees_no_peer(&a, &b_dev).await }, 8).await;
    tokio::time::sleep(Duration::from_millis(3000)).await;
    assert_eq!(
        a.svc.redial_attempts_for_test(),
        0,
        "不同网段的地址不应被拨号"
    );
}
