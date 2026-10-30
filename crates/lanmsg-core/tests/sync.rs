// P2 Task 9：多设备同步、转交与离线补投集成测试。
// 覆盖：离线设备重连补齐、联系人消息转交我的其他设备、新设备配对后接收
// 联系人与历史、对端离线期间发送的消息重连后送达、三台设备收敛无重复、
// 陌生人待处理消息不转交，以及 conv_member_ok 成员规则单测。
// 注释中文，标识符英文。
// 两/三节点测试一律 node_offline 系 + link（手动互拨），规避发现与手动拨号
// 并存的双连接裁决竞态。

mod common;

use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use lanmsg_api::types::{Event, MessageBodyView, MessageState, Request, Response};
use lanmsg_core::platform::PlatformHooks;
use lanmsg_core::store::{ConvKindRow, ConvRow};
use lanmsg_core::sync::conv_member_ok;
use lanmsg_proto::ids::{ConvId, UserId};

use common::{befriend, link, node_offline, node_offline_with, pair, wait_until, TestNode};

// ===== 辅助 =====

/// B 以加入模式初始化（配对前置）。
async fn init_join(b: &TestNode) {
    b.try_call(Request::Init {
        display_name: "加入机".into(),
        device_name: "加入机".into(),
        mode: lanmsg_api::types::InitMode::JoinExisting,
    })
    .await
    .expect("Init JoinExisting 失败");
}

/// user_id hex → UserId。
fn user(s: &str) -> UserId {
    UserId::from_hex(s).expect("非法 user_id")
}

/// 列出会话中的消息视图。
async fn list_msgs(n: &TestNode, conv: &str) -> Vec<lanmsg_api::types::MessageView> {
    match n
        .try_call(Request::ListMessages {
            conv: conv.to_string(),
            before: None,
            limit: 50,
        })
        .await
        .expect("ListMessages 失败")
    {
        Response::Messages { value } => value,
        other => panic!("期望 Messages，实际 {other:?}"),
    }
}

/// 列出会话中的文本消息。
async fn list_texts(n: &TestNode, conv: &str) -> Vec<lanmsg_api::types::MessageView> {
    list_msgs(n, conv)
        .await
        .into_iter()
        .filter(|m| matches!(m.body, MessageBodyView::Text { .. }))
        .collect()
}

/// 依次收事件直到凑齐 `want` 条「非本机设备的 Text MessageAdded」，超时 panic。
/// （三台设备同属一个用户，需按 author_device 区分是否本机所发。）
async fn count_foreign_text_added(
    rx: &mut broadcast::Receiver<Event>,
    self_device: &str,
    want: usize,
) {
    let mut count = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while count < want {
        let remain = deadline.saturating_duration_since(Instant::now());
        assert!(!remain.is_zero(), "5 秒内未收齐 {want} 条外部 MessageAdded");
        match tokio::time::timeout(remain, rx.recv()).await {
            Ok(Ok(Event::MessageAdded { msg })) => {
                if msg.author_device != self_device
                    && matches!(msg.body, MessageBodyView::Text { .. })
                {
                    count += 1;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) => panic!("事件通道已关闭"),
            Err(_) => panic!("5 秒内未收齐 {want} 条外部 MessageAdded（实收 {count}）"),
        }
    }
}

// ===== 测试 =====

/// B 下线期间 A 在 SelfDevices 发 3 条；B 用同 data_dir 重启后 5 秒内补齐，
/// A 侧 3 条状态 Delivered。
#[tokio::test]
async fn offline_device_catches_up_on_reconnect() {
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    let a_user = a.init_first("甲").await;
    init_join(&b).await;
    pair(&mut a, &mut b).await;

    // B 下线（保留 data_dir 稍后重启）。
    b.svc.shutdown().await;
    let TestNode { dir, .. } = b;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let self_conv = ConvId::self_devices(&user(&a_user)).to_hex();
    for i in 0..3 {
        a.call(Request::SendText {
            conv: self_conv.clone(),
            text: format!("离线消息{i}"),
        })
        .await;
    }
    // 无目标在线：A 侧保持 Sending。
    let msgs = list_texts(&a, &self_conv).await;
    assert_eq!(msgs.len(), 3);
    assert!(
        msgs.iter().all(|m| m.state == MessageState::Sending),
        "离线发送应保持 Sending"
    );

    // B 重启并重连。
    let b2 = node_offline_with(dir, PlatformHooks::default()).await;
    link(&a, &b2).await;

    // 5 秒内 B 补齐 3 条文本。
    wait_until(|| async { list_texts(&b2, &self_conv).await.len() == 3 }, 5).await;

    // A 侧 3 条置 Delivered。
    wait_until(
        || async {
            let msgs = list_texts(&a, &self_conv).await;
            msgs.len() == 3 && msgs.iter().all(|m| m.state == MessageState::Delivered)
        },
        5,
    )
    .await;
}

/// B 离线时联系人 X 给我发消息（到达 A）；B 上线后其 Dm(X) 会话出现该消息，
/// 作者设备为 X 的设备。
#[tokio::test]
async fn contact_message_relayed_to_my_other_device() {
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    let a_user = a.init_first("甲").await;
    init_join(&b).await;
    pair(&mut a, &mut b).await;

    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    let x_device = x.device_id().await;
    befriend(&mut a, &mut x).await;

    // B 下线。
    b.svc.shutdown().await;
    let TestNode { dir, .. } = b;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // X 给我发消息（X 与 A 已链接，消息直达 A）。
    let x_conv = ConvId::dm(&user(&x_user), &user(&a_user)).to_hex();
    x.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    x.call(Request::SendText {
        conv: x_conv.clone(),
        text: "转交测试".into(),
    })
    .await;
    wait_until(|| async { !list_texts(&a, &x_conv).await.is_empty() }, 5).await;

    // B 重启上线。
    let b2 = node_offline_with(dir, PlatformHooks::default()).await;
    link(&a, &b2).await;

    // B 的 Dm(X) 会话出现该消息，作者设备为 X 的设备。
    let my_conv = ConvId::dm(&user(&a_user), &user(&x_user)).to_hex();
    wait_until(
        || async {
            list_texts(&b2, &my_conv)
                .await
                .iter()
                .any(|m| matches!(&m.body, MessageBodyView::Text { text } if text == "转交测试"))
        },
        5,
    )
    .await;
    let msgs = list_texts(&b2, &my_conv).await;
    let relayed = msgs
        .iter()
        .find(|m| matches!(&m.body, MessageBodyView::Text { text } if text == "转交测试"))
        .expect("转交消息应存在");
    assert_eq!(relayed.author_device, x_device, "作者设备应为 X 的设备");
}

/// A 与 X 已是联系人且有历史；新设备 C 与 A 配对后：C 的 ListContacts 含 X、
/// Dm(X) 会话有完整历史、无 IncomingRequest 事件。
#[tokio::test]
async fn relayed_contact_message_is_accepted_on_new_device() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    befriend(&mut a, &mut x).await;

    // 历史：X 发 1 条、A 回 1 条。
    let conv = ConvId::dm(&user(&a_user), &user(&x_user)).to_hex();
    x.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    x.call(Request::SendText {
        conv: conv.clone(),
        text: "历史一".into(),
    })
    .await;
    wait_until(|| async { !list_texts(&a, &conv).await.is_empty() }, 5).await;
    a.call(Request::SendText {
        conv: conv.clone(),
        text: "历史二".into(),
    })
    .await;
    wait_until(|| async { list_texts(&x, &conv).await.len() == 2 }, 5).await;

    // 新设备 C 与 A 配对。
    let mut c = node_offline().await;
    init_join(&c).await;
    pair(&mut a, &mut c).await;

    // C 的 ListContacts 含 X。
    wait_until(
        || async {
            matches!(
                c.try_call(Request::ListContacts).await,
                Ok(Response::Contacts { value }) if value.iter().any(|v| v.user_id == x_user)
            )
        },
        5,
    )
    .await;

    // C 的 Dm(X) 会话有完整历史（2 条文本）。
    wait_until(|| async { list_texts(&c, &conv).await.len() == 2 }, 5).await;

    // 无 IncomingRequest 事件。
    c.no_event(|e| matches!(e, Event::IncomingRequest { .. }), 800)
        .await;
}

/// A、X 联系人；X 离线期间 A 发 2 条（状态 Sending）；X 重启后收到 2 条，
/// A 侧置 Delivered。
#[tokio::test]
async fn messages_sent_while_peer_offline_delivered_on_reconnect() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    befriend(&mut a, &mut x).await;

    let conv = ConvId::dm(&user(&a_user), &user(&x_user)).to_hex();
    // X 下线。
    x.svc.shutdown().await;
    let TestNode { dir, .. } = x;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A 发 2 条，保持 Sending。
    for i in 0..2 {
        a.call(Request::SendText {
            conv: conv.clone(),
            text: format!("离线{i}"),
        })
        .await;
    }
    let msgs = list_texts(&a, &conv).await;
    assert_eq!(msgs.len(), 2);
    assert!(msgs.iter().all(|m| m.state == MessageState::Sending));

    // X 重启并重连。
    let x2 = node_offline_with(dir, PlatformHooks::default()).await;
    link(&x2, &a).await;

    // X 收到 2 条。
    wait_until(|| async { list_texts(&x2, &conv).await.len() == 2 }, 5).await;
    // A 侧 2 条置 Delivered。
    wait_until(
        || async {
            let msgs = list_texts(&a, &conv).await;
            msgs.len() == 2 && msgs.iter().all(|m| m.state == MessageState::Delivered)
        },
        5,
    )
    .await;
}

/// A、B、C 三台我的设备互相全连接；各发 5 条 SelfDevices 消息 → 每台 15 条
/// 文本，外部作者 MessageAdded 事件各 10 个。
#[tokio::test]
async fn three_devices_converge_without_duplicates() {
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    let mut c = node_offline().await;
    let a_user = a.init_first("甲").await;
    init_join(&b).await;
    init_join(&c).await;
    pair(&mut a, &mut b).await;
    pair(&mut a, &mut c).await;
    // 配对完成后各设备已与配对对象重连；补上 B–C 直连。
    link(&b, &c).await;

    // 事件订阅在发消息前。
    let mut ev_a = a.svc.subscribe();
    let mut ev_b = b.svc.subscribe();
    let mut ev_c = c.svc.subscribe();

    let self_conv = ConvId::self_devices(&user(&a_user)).to_hex();
    for i in 0..5 {
        for n in [&a, &b, &c] {
            n.call(Request::SendText {
                conv: self_conv.clone(),
                text: format!("消息{i}"),
            })
            .await;
        }
    }

    // 每台 15 条文本（配对产生的 DeviceAdded 为 System 体，不计入）。
    for n in [&a, &b, &c] {
        wait_until(|| async { list_texts(n, &self_conv).await.len() == 15 }, 5).await;
    }

    // 外部设备所发的 Text MessageAdded 事件各 10 个（5 条 × 另外两台）。
    let a_dev = a.device_id().await;
    let b_dev = b.device_id().await;
    let c_dev = c.device_id().await;
    count_foreign_text_added(&mut ev_a, &a_dev, 10).await;
    count_foreign_text_added(&mut ev_b, &b_dev, 10).await;
    count_foreign_text_added(&mut ev_c, &c_dev, 10).await;
}

/// 陌生人 Y 给 A 发消息（待处理）→ B（我的另一台设备）重启同步后仍无该会话、
/// 无请求。
#[tokio::test]
async fn pending_stranger_messages_are_not_relayed() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut b = node_offline().await;
    init_join(&b).await;
    pair(&mut a, &mut b).await;

    let y = node_offline().await;
    let y_user = y.init_first("陌生人").await;
    link(&y, &a).await;

    // 陌生人 Y 给 A 发消息 → A 侧进入待处理。
    let conv = ConvId::dm(&user(&y_user), &user(&a_user)).to_hex();
    y.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    y.call(Request::SendText {
        conv: conv.clone(),
        text: "陌生人消息".into(),
    })
    .await;
    a.wait_event(|e| matches!(e, Event::IncomingRequest { .. }), 5)
        .await;

    // B 重启并重连，触发一轮完整同步。
    b.svc.shutdown().await;
    let TestNode { dir, .. } = b;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut b2 = node_offline_with(dir, PlatformHooks::default()).await;
    link(&a, &b2).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // B 无该会话。
    let convs = match b2.try_call(Request::ListConversations).await.unwrap() {
        Response::Conversations { value } => value,
        other => panic!("期望 Conversations，实际 {other:?}"),
    };
    assert!(
        !convs.iter().any(|cv| cv.conv == conv),
        "待处理陌生人的会话不应转交: {convs:?}"
    );
    // B 无请求。
    b2.no_event(|e| matches!(e, Event::IncomingRequest { .. }), 500)
        .await;
}

/// conv_member_ok 成员规则：SelfDevices 拒绝他人；Dm 接受自己与对端用户、
/// 拒绝第三者；其他 kind 一律拒绝。
#[test]
fn conv_member_rules() {
    let self_u = UserId([1u8; 32]);
    let peer_u = UserId([2u8; 32]);
    let other_u = UserId([3u8; 32]);

    let self_conv = ConvRow {
        conv: ConvId::self_devices(&self_u),
        kind: ConvKindRow::SelfDevices,
        peer_user: None,
        title: None,
        created_at: 0,
    };
    assert!(conv_member_ok(&self_conv, &self_u, &self_u));
    assert!(!conv_member_ok(&self_conv, &peer_u, &self_u));

    let dm = ConvRow {
        conv: ConvId::dm(&self_u, &peer_u),
        kind: ConvKindRow::Dm,
        peer_user: Some(peer_u),
        title: None,
        created_at: 0,
    };
    assert!(conv_member_ok(&dm, &self_u, &self_u));
    assert!(conv_member_ok(&dm, &peer_u, &self_u));
    assert!(!conv_member_ok(&dm, &other_u, &self_u));

    let guest = ConvRow {
        conv: ConvId([9u8; 32]),
        kind: ConvKindRow::Guest,
        peer_user: None,
        title: None,
        created_at: 0,
    };
    assert!(!conv_member_ok(&guest, &self_u, &self_u));
}
