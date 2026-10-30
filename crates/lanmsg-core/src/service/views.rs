// P1 核心骨架：Service 视图（Task 10）。
// message_view / conv_summary 将存储层记录转换为 API 层 DTO。
// 注释中文，标识符英文。

use lanmsg_api::types::{
    ConvKind, ConversationSummary, MessageBodyView, MessageState, MessageView,
};
use lanmsg_proto::ids::{DeviceId, UserId};
use lanmsg_proto::types::MsgBody;

use crate::store::{ConvRow, MsgStateRow, Store, StoredMessage};

/// 将存储消息转换为消息视图。
///
/// - `author_device_name` 取 devices 表 name（无则 device short）。
/// - `is_mine` = 作者用户为自己（通过 devices 表的 user 字段比较）。
/// - `body`：Text → `Text`，其余 → `Unsupported`。
/// - `state` 由 `MsgStateRow` 一一映射。
pub fn message_view(store: &Store, self_device: &DeviceId, m: &StoredMessage) -> MessageView {
    let env = &m.env;
    let author_device = env.id.author;

    let self_row = store.get_device(self_device).ok().flatten();
    let self_user = self_row.as_ref().map(|r| &r.user);

    let author_row = store.get_device(&author_device).ok().flatten();
    let author_user = author_row.as_ref().map(|r| &r.user);

    let is_mine = self_user
        .zip(author_user)
        .map(|(s, a)| s == a)
        .unwrap_or(false);

    let (author_device_name, author_user_str) = match &author_row {
        Some(r) => (r.name.clone(), r.user.to_hex()),
        None => (author_device.short(), String::new()),
    };

    let body = match &env.body {
        MsgBody::Text { text } => MessageBodyView::Text { text: text.clone() },
        _ => MessageBodyView::Unsupported,
    };

    let state = match m.state {
        MsgStateRow::Sending => MessageState::Sending,
        MsgStateRow::Sent => MessageState::Sent,
        MsgStateRow::Delivered => MessageState::Delivered,
        MsgStateRow::Read => MessageState::Read,
        MsgStateRow::Failed => MessageState::Failed,
        MsgStateRow::Recalled => MessageState::Recalled,
    };

    MessageView {
        id: env.id.to_string(),
        conv: env.conv.to_hex(),
        author_user: author_user_str,
        author_device: author_device.to_hex(),
        author_device_name,
        is_mine,
        hlc_millis: env.hlc.millis,
        body,
        state,
    }
}

/// 将会话行转换为会话摘要。
///
/// - DM 标题 = 对端用户显示名（无则 user short）。
/// - SelfDevices 标题 = `"我的设备"`。
/// - `last_text`/`last_hlc_millis` 取最新一条消息。
/// - `unread` 本阶段恒 0（P2 实现）。
pub fn conv_summary(
    store: &Store,
    _self_user: Option<&UserId>,
    conv: &ConvRow,
    peer_online: bool,
) -> ConversationSummary {
    let title = match conv.kind {
        crate::store::ConvKindRow::Dm => conv
            .peer_user
            .as_ref()
            .and_then(|u| store.get_user(u).ok().flatten())
            .map(|u| u.display_name.clone())
            .unwrap_or_else(|| conv.peer_user.map(|u| u.short()).unwrap_or_default()),
        crate::store::ConvKindRow::SelfDevices => "我的设备".into(),
        _ => conv.title.clone().unwrap_or_default(),
    };

    let messages = store.list_messages(&conv.conv, None, 1).unwrap_or_default();
    let (last_text, last_hlc_millis) = messages
        .into_iter()
        .next()
        .map(|m| {
            let text = match &m.env.body {
                MsgBody::Text { text } => Some(text.clone()),
                _ => None,
            };
            (text, Some(m.env.hlc.millis))
        })
        .unwrap_or((None, None));

    let kind = match conv.kind {
        crate::store::ConvKindRow::SelfDevices => ConvKind::SelfDevices,
        crate::store::ConvKindRow::Dm => ConvKind::Dm,
        crate::store::ConvKindRow::Guest => ConvKind::Guest,
        crate::store::ConvKindRow::LocalSend => ConvKind::LocalSend,
        crate::store::ConvKindRow::Station => ConvKind::Station,
    };

    ConversationSummary {
        conv: conv.conv.to_hex(),
        kind,
        title,
        last_text,
        last_hlc_millis,
        unread: 0,
        peer_online,
    }
}

// Task 11 将追加 `peer_view(p: &PeerInfo, self_user: Option<&UserId>) -> PeerView`（addr 用 dial_addr）。
