// 单机端到端验收缺陷修复的集成测试（见 docs/superpowers/reports/2026-11-06-e2e-acceptance.md）。
// 一律 node_offline 系 + link（手动互拨）。注释中文，标识符英文。

mod common;

use lanmsg_api::types::{MessageBodyView, Request, Response};
use lanmsg_proto::ids::{ConvId, UserId};

use common::{befriend, node_offline, pair, wait_until, TestNode};

async fn join_init(b: &TestNode) {
    b.try_call(Request::Init {
        display_name: "加入机".into(),
        device_name: "加入机".into(),
        mode: lanmsg_api::types::InitMode::JoinExisting,
    })
    .await
    .expect("Init JoinExisting 失败");
}

fn user(s: &str) -> UserId {
    UserId::from_hex(s).expect("非法 user_id")
}

async fn texts(n: &TestNode, conv: &str) -> Vec<String> {
    match n
        .try_call(Request::ListMessages {
            conv: conv.to_string(),
            before: None,
            limit: 50,
        })
        .await
    {
        Ok(Response::Messages { value }) => value
            .into_iter()
            .filter_map(|m| match m.body {
                MessageBodyView::Text { text } => Some(text),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// 缺陷 1：A1、A2 同用户在线，B 仅与 A1 相连；A1 发给 B 的消息与 B 的回复
/// 都应实时出现在 A2 的该 DM 会话中（无需重启/重连）。
#[tokio::test]
async fn online_sibling_receives_third_party_dm_in_realtime() {
    let mut a1 = node_offline().await;
    let mut a2 = node_offline().await;
    let a_user = a1.init_first("甲").await;
    join_init(&a2).await;
    pair(&mut a1, &mut a2).await;
    let mut b = node_offline().await;
    let b_user = b.init_first("乙").await;
    befriend(&mut a1, &mut b).await;

    // 等联系人同步到 A2。
    wait_until(
        || async {
            matches!(
                a2.try_call(Request::ListContacts).await,
                Ok(Response::Contacts { value }) if value.iter().any(|v| v.user_id == b_user)
            )
        },
        5,
    )
    .await;

    let conv = ConvId::dm(&user(&a_user), &user(&b_user)).to_hex();
    a1.call(Request::OpenDm {
        user: b_user.clone(),
    })
    .await;
    a1.call(Request::SendText {
        conv: conv.clone(),
        text: "去".into(),
    })
    .await;
    wait_until(
        || async { texts(&a2, &conv).await.contains(&"去".to_string()) },
        5,
    )
    .await;

    // B 回复（B 只连着 A1）。
    b.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    b.call(Request::SendText {
        conv: conv.clone(),
        text: "回".into(),
    })
    .await;
    wait_until(
        || async { texts(&a2, &conv).await.contains(&"回".to_string()) },
        5,
    )
    .await;
}
