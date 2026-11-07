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

use lanmsg_api::types::{
    ErrorCode, Event, MessageBodyView, MessageState, Request, Response, SystemEventView,
};
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
    let b_dev = b.device_id().await;
    b.svc.shutdown().await;
    let TestNode { dir, .. } = b;
    // 必须等 A 侧确认旧连接已断开再重连：B 重启后设备 ID 不变，若 A 仍持有旧连接，
    // 重复连接裁决会丢弃新拨入的连接，而 AddManualPeer 是一次性拨号不重试，
    // 双方永远互见不到（高负载下偶发）。
    wait_until(
        || async {
            !matches!(
                a.try_call(Request::ListPeers).await,
                Ok(Response::Peers { value }) if value.iter().any(|p| p.device_id == b_dev)
            )
        },
        10,
    )
    .await;
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

// ===== P2 Task 10：已读、撤回、本地删除 =====

/// X 发 2 条给甲（A、B 两台我的设备在线）→ A MarkRead(第 2 条) →
/// B 的该会话 unread 变 0，X 侧两条消息状态 Read。
#[tokio::test]
async fn mark_read_syncs_to_my_devices_and_peer() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    befriend(&mut a, &mut x).await;

    let conv = ConvId::dm(&user(&x_user), &user(&a_user)).to_hex();
    x.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    for i in 0..2 {
        x.call(Request::SendText {
            conv: conv.clone(),
            text: format!("已读测试{i}"),
        })
        .await;
    }
    wait_until(|| async { list_texts(&a, &conv).await.len() == 2 }, 5).await;

    // B 在消息之后配对上线：经同步拿到会话与 2 条消息。
    let mut b = node_offline().await;
    init_join(&b).await;
    pair(&mut a, &mut b).await;
    wait_until(|| async { list_texts(&b, &conv).await.len() == 2 }, 5).await;

    // A 标记已读到第 2 条。
    let msgs = list_texts(&a, &conv).await;
    let upto = msgs[1].id.clone();
    a.call(Request::MarkRead {
        conv: conv.clone(),
        upto,
    })
    .await;

    // B 的会话 unread 变 0。
    wait_until(
        || async {
            matches!(
                b.try_call(Request::ListConversations).await,
                Ok(Response::Conversations { value })
                    if value.iter().any(|cv| cv.conv == conv && cv.unread == 0)
            )
        },
        5,
    )
    .await;

    // X 侧两条消息状态 Read。
    wait_until(
        || async {
            let msgs = list_texts(&x, &conv).await;
            msgs.len() == 2 && msgs.iter().all(|m| m.state == MessageState::Read)
        },
        5,
    )
    .await;
}

/// A 发消息后立即撤回 → X、B 侧均收到 MessageUpdated(state Recalled)，
/// 各端视图为撤回提示。
#[tokio::test]
async fn recall_within_window_hides_on_all_sides() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    befriend(&mut a, &mut x).await;

    let conv = ConvId::dm(&user(&a_user), &user(&x_user)).to_hex();
    a.call(Request::OpenDm {
        user: x_user.clone(),
    })
    .await;
    a.call(Request::SendText {
        conv: conv.clone(),
        text: "撤回我".into(),
    })
    .await;
    wait_until(|| async { !list_texts(&x, &conv).await.is_empty() }, 5).await;

    // B 在消息之后配对上线（同步拿到会话与消息）。
    let mut b = node_offline().await;
    init_join(&b).await;
    pair(&mut a, &mut b).await;
    wait_until(|| async { !list_texts(&b, &conv).await.is_empty() }, 5).await;

    let msg = list_texts(&a, &conv).await.remove(0);
    let recalled_id = msg.id.clone();
    a.call(Request::Recall {
        msg: recalled_id.clone(),
    })
    .await;

    // X、B 侧该消息 MessageUpdated state Recalled。
    x.wait_event(
        |e| {
            matches!(
                e,
                Event::MessageUpdated { msg } if msg.id == recalled_id && msg.state == MessageState::Recalled
            )
        },
        5,
    )
    .await;
    b.wait_event(
        |e| {
            matches!(
                e,
                Event::MessageUpdated { msg } if msg.id == recalled_id && msg.state == MessageState::Recalled
            )
        },
        5,
    )
    .await;

    // 三端视图均为撤回提示。
    for n in [&a, &x, &b] {
        let msgs = list_msgs(n, &conv).await;
        let v = msgs
            .iter()
            .find(|m| m.id == recalled_id)
            .expect("撤回消息应存在");
        assert_eq!(v.state, MessageState::Recalled);
        match &v.body {
            MessageBodyView::System {
                event: SystemEventView::Recalled { author_name, .. },
            } => {
                assert!(!author_name.is_empty(), "撤回提示应带作者名");
            }
            other => panic!("期望 System 撤回提示，实际 {other:?}"),
        }
    }
}

/// 回归：已读回执 / 撤回是隐藏消息，双方互相已读、互相收到对方 Read 后，
/// 都不得把它们作为 MessageAdded/MessageUpdated 推给界面（否则界面渲染出
/// “不支持的消息类型”）。同时断言预期的可见 MessageAdded 确实到达，排除事件整体丢失。
#[tokio::test]
async fn hidden_bodies_never_reach_ui_events() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    befriend(&mut a, &mut x).await;

    let conv = ConvId::dm(&user(&a_user), &user(&x_user)).to_hex();
    a.call(Request::OpenDm {
        user: x_user.clone(),
    })
    .await;
    a.call(Request::SendText {
        conv: conv.clone(),
        text: "待撤回".into(),
    })
    .await;
    x.call(Request::SendText {
        conv: conv.clone(),
        text: "对方回复".into(),
    })
    .await;
    wait_until(|| async { list_texts(&x, &conv).await.len() == 2 }, 5).await;
    wait_until(|| async { list_texts(&a, &conv).await.len() == 2 }, 5).await;

    // 双方都标已读（各向对方发 Read），A 再撤回自己的消息（向 X 发 Recall）。
    for n in [&a, &x] {
        let last = list_texts(n, &conv).await.pop().expect("有消息");
        n.call(Request::MarkRead {
            conv: conv.clone(),
            upto: last.id,
        })
        .await;
    }
    let mine = list_texts(&a, &conv)
        .await
        .into_iter()
        .find(|m| m.is_mine)
        .expect("A 的消息");
    a.call(Request::Recall {
        msg: mine.id.clone(),
    })
    .await;
    wait_until(
        || async {
            list_msgs(&x, &conv)
                .await
                .iter()
                .any(|m| m.id == mine.id && m.state == MessageState::Recalled)
        },
        5,
    )
    .await;
    // 再等回执往返完成。
    tokio::time::sleep(Duration::from_millis(500)).await;

    for (name, n, want) in [("A", &mut a, "对方回复"), ("X", &mut x, "待撤回")] {
        let mut saw_visible = false;
        loop {
            match n.events.try_recv() {
                Ok(Event::MessageAdded { msg }) | Ok(Event::MessageUpdated { msg }) => {
                    assert!(
                        !matches!(msg.body, MessageBodyView::Unsupported),
                        "{name} 收到隐藏消息泄漏的界面事件: {msg:?}"
                    );
                    if matches!(&msg.body, MessageBodyView::Text { text } if text == want) {
                        saw_visible = true;
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    panic!("{name} 事件通道落后 {n} 条，断言不可靠")
                }
                Err(broadcast::error::TryRecvError::Closed) => break,
            }
        }
        assert!(saw_visible, "{name} 应收到可见消息事件 {want}");
    }
}

/// 回归：以隐藏消息（Read 回执）作为 MarkRead 的 upto 不得再产生新的 Read，
/// 否则界面把回执当新消息再标已读会形成无限循环。
#[tokio::test]
async fn mark_read_on_hidden_message_does_not_loop() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    befriend(&mut a, &mut x).await;

    let conv = ConvId::dm(&user(&a_user), &user(&x_user)).to_hex();
    x.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    x.call(Request::SendText {
        conv: conv.clone(),
        text: "回执测试".into(),
    })
    .await;
    wait_until(|| async { list_texts(&a, &conv).await.len() == 1 }, 5).await;
    let upto = list_texts(&a, &conv).await[0].id.clone();
    a.call(Request::MarkRead {
        conv: conv.clone(),
        upto,
    })
    .await;

    // 统计 A 自己发出的信封数（含隐藏 Read）。
    let store = a.svc.store_for_test();
    let a_dev = lanmsg_proto::ids::DeviceId::from_hex(&a.device_id().await).expect("device id");
    let count = || {
        (1..=50u64)
            .take_while(|seq| {
                store
                    .get_message(&lanmsg_proto::ids::MsgId {
                        author: a_dev,
                        seq: *seq,
                    })
                    .ok()
                    .flatten()
                    .is_some()
            })
            .count()
    };
    let before = count();
    assert!(before >= 1, "A 应已发出 Read 回执");

    // 把 A 自己发出的最后一条（Read 回执）当 upto 再标已读：必须是空操作。
    let read_id = lanmsg_proto::ids::MsgId {
        author: a_dev,
        seq: before as u64,
    };
    a.call(Request::MarkRead {
        conv: conv.clone(),
        upto: read_id.to_string(),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(count(), before, "对隐藏消息 MarkRead 不应产生新 Read");
}

/// 构造并注入一条由 X 设备密钥签名的 Recall（绕过本地校验，模拟对端发来无效撤回）。
async fn inject_recall(x: &TestNode, a: &TestNode, conv: &str, target: &str, hlc_millis: i64) {
    use lanmsg_proto::frame::Frame;
    use lanmsg_proto::ids::{DeviceId, MsgId};
    use lanmsg_proto::sign::{sign_with_zeroed_sig, DeviceKey};
    use lanmsg_proto::types::{Envelope, Hlc, MsgBody, Signature64, SyncPush};

    let conv_id = ConvId::from_hex(conv).expect("conv");
    let x_dev = DeviceId::from_hex(&x.device_id().await).expect("x device");
    let a_dev = DeviceId::from_hex(&a.device_id().await).expect("a device");
    let (seq, conv_seq) = x
        .svc
        .store_for_test()
        .next_seqs(&x_dev, &conv_id)
        .expect("next_seqs");
    let key_bytes: [u8; 32] = std::fs::read(x.dir.path().join("keys").join("device.key"))
        .expect("device key")
        .try_into()
        .expect("key len");
    let mut env = Envelope {
        conv: conv_id,
        id: MsgId { author: x_dev, seq },
        conv_seq,
        hlc: Hlc {
            millis: hlc_millis,
            counter: 0,
        },
        body: MsgBody::Recall {
            target: target.parse().expect("msg id"),
        },
        enc: None,
        sig: Signature64([0u8; 64]),
    };
    let key = DeviceKey::from_bytes(&key_bytes);
    sign_with_zeroed_sig(&mut env, |b| key.sign(b));
    x.svc
        .send_frame_for_test(
            &a_dev,
            Frame::SyncPush(SyncPush {
                envelopes: vec![env],
            }),
        )
        .await
        .expect("send");
}

/// 回归：无效撤回（非作者本人 / 超时）到达后，存储层入库时先把目标置 Recalled 并发
/// 过“已撤回”摘要；判定无效、undo 之后必须再发摘要，预览恢复为原内容。
#[tokio::test]
async fn invalid_recall_restores_conversation_preview() {
    for case in ["not_author", "expired"] {
        let mut a = node_offline().await;
        let a_user = a.init_first("甲").await;
        let mut x = node_offline().await;
        let x_user = x.init_first("小张").await;
        befriend(&mut a, &mut x).await;
        let conv = ConvId::dm(&user(&a_user), &user(&x_user)).to_hex();
        a.call(Request::OpenDm {
            user: x_user.clone(),
        })
        .await;
        // 目标：not_author 用 A 的消息（X 无权撤回）；expired 用 X 自己的消息 + 3 分钟后的撤回。
        let (sender, text) = if case == "not_author" {
            (&a, "保留我")
        } else {
            (&x, "保留我")
        };
        sender
            .call(Request::SendText {
                conv: conv.clone(),
                text: text.into(),
            })
            .await;
        wait_until(|| async { list_texts(&a, &conv).await.len() == 1 }, 5).await;
        wait_until(|| async { list_texts(&x, &conv).await.len() == 1 }, 5).await;
        let target = list_texts(&a, &conv).await.remove(0);
        let millis = if case == "not_author" {
            target.hlc_millis + 1000
        } else {
            target.hlc_millis + 3 * 60 * 1000
        };
        while a.events.try_recv().is_ok() {}

        inject_recall(&x, &a, &conv, &target.id, millis).await;

        // 先出现入库时的“已撤回”摘要，随后必须出现恢复后的原预览。
        a.wait_event(
            |e| {
                matches!(e, Event::ConversationUpdated { summary }
                    if summary.conv == conv && summary.last_kind == Some(lanmsg_api::types::LastMessageKind::Recalled))
            },
            5,
        )
        .await;
        a.wait_event(
            |e| {
                matches!(e, Event::ConversationUpdated { summary }
                    if summary.conv == conv
                        && summary.last_kind == Some(lanmsg_api::types::LastMessageKind::Text)
                        && summary.last_text.as_deref() == Some(text))
            },
            5,
        )
        .await;
        let msgs = list_texts(&a, &conv).await;
        assert_eq!(msgs.len(), 1, "{case}: 目标消息应仍可见");
        assert_ne!(msgs[0].state, MessageState::Recalled, "{case}");
    }
}

/// 撤回他人消息被拒绝（InvalidArgument）。
#[tokio::test]
async fn recall_others_message_rejected() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    befriend(&mut a, &mut x).await;

    let conv = ConvId::dm(&user(&x_user), &user(&a_user)).to_hex();
    x.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    x.call(Request::SendText {
        conv: conv.clone(),
        text: "别人的消息".into(),
    })
    .await;
    wait_until(|| async { !list_texts(&a, &conv).await.is_empty() }, 5).await;

    let msg = list_texts(&a, &conv).await.remove(0);
    let err = a
        .try_call(Request::Recall { msg: msg.id })
        .await
        .expect_err("撤回他人消息应被拒绝");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert_eq!(err.message, "only own messages can be recalled");
}

/// A 本地删除（含我的设备）→ B 不再显示；X 侧仍在。
#[tokio::test]
async fn delete_local_also_my_devices() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    befriend(&mut a, &mut x).await;

    let conv = ConvId::dm(&user(&x_user), &user(&a_user)).to_hex();
    x.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    x.call(Request::SendText {
        conv: conv.clone(),
        text: "删除我".into(),
    })
    .await;
    wait_until(|| async { !list_texts(&a, &conv).await.is_empty() }, 5).await;

    // B 配对上线，经同步拿到该消息。
    let mut b = node_offline().await;
    init_join(&b).await;
    pair(&mut a, &mut b).await;
    wait_until(|| async { !list_texts(&b, &conv).await.is_empty() }, 5).await;

    let msg = list_texts(&a, &conv).await.remove(0);
    a.call(Request::DeleteLocal {
        msgs: vec![msg.id],
        also_my_devices: true,
    })
    .await;

    // A、B 均不再显示。
    wait_until(|| async { list_texts(&b, &conv).await.is_empty() }, 5).await;
    assert!(list_texts(&a, &conv).await.is_empty(), "A 侧删除后不应显示");
    // X 侧仍在。
    assert_eq!(list_texts(&x, &conv).await.len(), 1, "X 侧消息应保留");
}

/// A 本地删除后 B 与 A 同步（已删除信封仍参与同步），A 的列表仍不含该消息。
#[tokio::test]
async fn deleted_message_not_resurrected_by_sync() {
    let mut a = node_offline().await;
    let a_user = a.init_first("甲").await;
    let mut x = node_offline().await;
    let x_user = x.init_first("小张").await;
    befriend(&mut a, &mut x).await;

    let conv = ConvId::dm(&user(&x_user), &user(&a_user)).to_hex();
    x.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    x.call(Request::SendText {
        conv: conv.clone(),
        text: "将被删除的消息".into(),
    })
    .await;
    wait_until(|| async { !list_texts(&a, &conv).await.is_empty() }, 5).await;

    let msg = list_texts(&a, &conv).await.remove(0);
    a.call(Request::DeleteLocal {
        msgs: vec![msg.id],
        also_my_devices: false,
    })
    .await;
    assert!(list_texts(&a, &conv).await.is_empty());

    // B 与 A 同步（配对建立连接并拉取差量，已删除信封仍参与同步）。
    let mut b = node_offline().await;
    init_join(&b).await;
    pair(&mut a, &mut b).await;
    wait_until(|| async { list_texts(&b, &conv).await.len() == 1 }, 5).await;

    // A 的列表未复活。
    assert!(
        list_texts(&a, &conv).await.is_empty(),
        "同步后 A 的已删除消息不应复活"
    );
}
