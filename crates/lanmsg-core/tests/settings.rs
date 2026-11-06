// P2 Task 4：设置、设备改名（重签证书）与资料（SetProfile）集成测试。
// 两节点经 AddManualPeer 互拨建连，验证设置往返、CertUpdate 接收侧生效与 JoinExisting 语义。
// 注释中文，标识符英文。

mod common;

use lanmsg_api::types::{ErrorCode, Event, InitMode, Request, Response};
use lanmsg_core::platform::PlatformHooks;

use common::{node_offline, node_offline_with, wait_until};

/// 取 SelfInfo。
async fn self_info(n: &common::TestNode) -> lanmsg_api::types::SelfInfo {
    match n.call(Request::GetSelf).await {
        Response::SelfInfo { value } => value,
        other => panic!("期望 SelfInfo，实际 {other:?}"),
    }
}

/// 建立 a→b 连接（a 拨 b 的地址），等双方在对方 ListPeers 可见且连接裁决收敛。
async fn connect_pair(a: &mut common::TestNode, a_dev: &str, b: &common::TestNode, b_dev: &str) {
    let addr = b.svc.local_addr().to_string();
    a.call(Request::AddManualPeer { addr }).await;
    a.wait_event(
        |e| matches!(e, Event::PeerOnline { peer } if peer.device_id == b_dev),
        5,
    )
    .await;
    // b 侧 accept 同样产生 Up；等 b 也看到 a，cert 交换完成、可互发帧。
    wait_until(
        || async {
            b.try_call(Request::ListPeers)
                .await
                .ok()
                .and_then(|r| match r {
                    Response::Peers { value } => Some(value.iter().any(|p| p.device_id == a_dev)),
                    _ => None,
                })
                .unwrap_or(false)
        },
        5,
    )
    .await;
    // 等连接裁决收敛（Memory 发现与手动拨号并存时可能短暂双连接），避免广播帧走被淘汰的连接。
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
}

#[tokio::test]
async fn settings_roundtrip_and_device_rename_reissues_cert() {
    let mut a = node_offline().await;
    let b = node_offline().await;
    a.init_first("张三").await;
    b.init_first("李四").await;
    let a_dev = self_info(&a).await.device_id;
    let b_dev = self_info(&b).await.device_id;

    connect_pair(&mut a, &a_dev, &b, &b_dev).await;

    // GetSettings 返回默认值（阈值符合规范）。
    let s = match a.call(Request::GetSettings).await {
        Response::Settings { value } => value,
        other => panic!("期望 Settings，实际 {other:?}"),
    };
    assert_eq!(s.auto_image_max, 10 * 1024 * 1024);
    assert_eq!(s.auto_file_max, 50 * 1024 * 1024);
    assert!(s.auto_file_wifi_only);
    assert!(!s.download_dir.is_empty());
    assert!(!s.device_name.is_empty());

    // 非法设备名（超长）→ InvalidArgument。
    let mut bad = s.clone();
    bad.device_name = "x".repeat(33);
    let err = a
        .try_call(Request::SetSettings { settings: bad })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);

    // SetSettings 改设备名。
    let mut s2 = s.clone();
    s2.device_name = "新名字".into();
    a.call(Request::SetSettings {
        settings: s2.clone(),
    })
    .await;

    // GetSettings 返回新名字。
    assert_eq!(
        match a.call(Request::GetSettings).await {
            Response::Settings { value } => value,
            other => panic!("期望 Settings，实际 {other:?}"),
        }
        .device_name,
        "新名字"
    );

    // 对端 ListPeers 中 a 的 device_name 更新（CertUpdate 已到达并生效）。
    wait_until(
        || async {
            b.try_call(Request::ListPeers)
                .await
                .ok()
                .and_then(|r| match r {
                    Response::Peers { value } => Some(
                        value
                            .iter()
                            .any(|p| p.device_id == a_dev && p.device_name == "新名字"),
                    ),
                    _ => None,
                })
                .unwrap_or(false)
        },
        5,
    )
    .await;
}

#[tokio::test]
async fn set_profile_rejects_empty_and_updates_peers() {
    let mut a = node_offline().await;
    let b = node_offline().await;
    a.init_first("张三").await;
    b.init_first("李四").await;
    let a_dev = self_info(&a).await.device_id;
    let b_dev = self_info(&b).await.device_id;

    connect_pair(&mut a, &a_dev, &b, &b_dev).await;

    // 空显示名 → InvalidArgument。
    let err = a
        .try_call(Request::SetProfile {
            display_name: "".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);

    // 过长或含控制字符的显示名同样拒绝。
    for bad in ["字".repeat(33), "a\nb".to_string()] {
        let err = a
            .try_call(Request::SetProfile { display_name: bad })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    // 改显示名。
    a.call(Request::SetProfile {
        display_name: "张三2".into(),
    })
    .await;

    // B 重启重连后，ListPeers 中 a 的 display_name 应为“张三2”（握手取 A 的新 HelloInfo）。
    b.svc.shutdown().await;
    let dir_b = b.dir;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let mut b2 = node_offline_with(dir_b, PlatformHooks::default()).await;
    b2.call(Request::AddManualPeer {
        addr: a.svc.local_addr().to_string(),
    })
    .await;
    b2.wait_event(
        |e| matches!(e, Event::PeerOnline { peer } if peer.device_id == a_dev),
        5,
    )
    .await;
    wait_until(
        || async {
            b2.try_call(Request::ListPeers)
                .await
                .ok()
                .and_then(|r| match r {
                    Response::Peers { value } => Some(
                        value
                            .iter()
                            .any(|p| p.device_id == a_dev && p.display_name == "张三2"),
                    ),
                    _ => None,
                })
                .unwrap_or(false)
        },
        5,
    )
    .await;
}

#[tokio::test]
async fn join_existing_marks_pending_without_identity() {
    // Disabled 发现：无自动发现干扰。
    let n = node_offline().await;

    // 未初始化 JoinExisting → Ok。
    n.call(Request::Init {
        display_name: "名字".into(),
        device_name: "待接入设备".into(),
        mode: InitMode::JoinExisting,
    })
    .await;

    // GetSelf.initialized == false。
    let info = self_info(&n).await;
    assert!(!info.initialized);

    // 再 Init JoinExisting 仍 Ok。
    n.call(Request::Init {
        display_name: "名字".into(),
        device_name: "待接入设备".into(),
        mode: InitMode::JoinExisting,
    })
    .await;

    // Init FirstDevice → Ok。
    n.call(Request::Init {
        display_name: "名字".into(),
        device_name: "首台设备".into(),
        mode: InitMode::FirstDevice,
    })
    .await;

    // 再 Init JoinExisting → AlreadyInitialized。
    let err = n
        .try_call(Request::Init {
            display_name: "名字".into(),
            device_name: "待接入设备".into(),
            mode: InitMode::JoinExisting,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AlreadyInitialized);
}
