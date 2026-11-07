// 不在本机网段的已知地址：只调节节奏（不主动高频拨号），身份由握手内校验保证；
// 网络变化时立即试一次，身份正确则跨网段也能连上。
// 强制“不在网段”是进程级开关，所以单独成一个测试文件（独立进程）。
// 注释中文，标识符英文。

mod common;

use std::time::Duration;

use lanmsg_core::Service;

use common::{befriended_pair, dev, sees_no_peer, sees_peer, wait_until};

#[tokio::test]
async fn off_subnet_address_waits_then_connects_on_network_change() {
    Service::force_off_subnet_for_test(true);
    let (a, b, a_dev, b_dev) = befriended_pair().await;
    a.svc.disconnect_for_test(&dev(&b_dev)).await;
    wait_until(|| async { sees_no_peer(&a, &b_dev).await }, 5).await;
    // 不在同网段：不主动拨号，用最长退避间隔等待。
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(a.svc.redial_attempts_for_test(), 0);
    assert_eq!(b.svc.redial_attempts_for_test(), 0);
    assert!(sees_no_peer(&a, &b_dev).await);
    // 网络变化：立即试一次，身份正确则连上（同网段判断不是安全条件）。
    a.call(lanmsg_api::types::Request::NetworkChanged).await;
    b.call(lanmsg_api::types::Request::NetworkChanged).await;
    wait_until(
        || async { sees_peer(&a, &b_dev).await && sees_peer(&b, &a_dev).await },
        10,
    )
    .await;
}
