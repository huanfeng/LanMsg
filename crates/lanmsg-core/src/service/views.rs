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

// Task 11：peer_view 将连接层 PeerInfo 转换为对端设备视图（addr 用 dial_addr）。
// P2 Task 5：is_contact 按联系人表真实状态填写；新增 contact_view / device_view /
// request_view 供 ListContacts / ListRequests 使用。
pub use lanmsg_api::types::PeerView;
use lanmsg_api::types::{ContactView, DeviceView, PathClassDto, RequestKind, RequestView};

use crate::net::PeerInfo;
use crate::store::{ContactRow, DeviceRow, RequestRow};

/// 将发现表条目（近期发现但未连接的设备）转换为对端设备视图：
/// `addr` 用发现来源地址，`path` 按 `classify(.., via_multicast = true)` 判定。
pub fn discovered_peer_view(
    store: &Store,
    ann: &lanmsg_proto::types::Announce,
    addr: std::net::SocketAddr,
    self_user: Option<&UserId>,
) -> PeerView {
    let cert = &ann.cert;
    let path = crate::net::classify(addr.ip(), &crate::net::local_ifaces(), true);
    PeerView {
        device_id: cert.device.to_hex(),
        device_short: cert.device.short(),
        user_id: cert.user.to_hex(),
        display_name: ann.display_name.clone(),
        device_name: cert.device_name.clone(),
        device_type: format!("{:?}", cert.device_type),
        addr: addr.to_string(),
        path: path_dto(path),
        is_contact: store
            .get_contact(&cert.user)
            .ok()
            .flatten()
            .is_some_and(|r| r.state == crate::store::ContactState::Accepted),
        is_mine: self_user.is_some_and(|u| u == &cert.user),
    }
}

/// 路径等级 → DTO。
fn path_dto(p: lanmsg_proto::types::PathClass) -> PathClassDto {
    match p {
        lanmsg_proto::types::PathClass::Lan => PathClassDto::Lan,
        lanmsg_proto::types::PathClass::Vpn => PathClassDto::Vpn,
        lanmsg_proto::types::PathClass::Internet => PathClassDto::Internet,
        lanmsg_proto::types::PathClass::Relay => PathClassDto::Relay,
    }
}

/// 将对端连接信息转换为对端设备视图（is_contact 查联系人表）。
pub fn peer_view(store: &Store, p: &PeerInfo, self_user: Option<&UserId>) -> PeerView {
    let cert = p.cert.as_ref();
    let is_contact = cert
        .map(|c| {
            store
                .get_contact(&c.user)
                .ok()
                .flatten()
                .is_some_and(|r| r.state == crate::store::ContactState::Accepted)
        })
        .unwrap_or(false);
    PeerView {
        device_id: p.device.to_hex(),
        device_short: p.device.short(),
        user_id: cert.map(|c| c.user.to_hex()).unwrap_or_default(),
        display_name: p.display_name.clone(),
        device_name: cert.map(|c| c.device_name.clone()).unwrap_or_default(),
        device_type: cert
            .map(|c| format!("{:?}", c.device_type))
            .unwrap_or_default(),
        addr: p.dial_addr.to_string(),
        path: match p.path {
            lanmsg_proto::types::PathClass::Lan => PathClassDto::Lan,
            lanmsg_proto::types::PathClass::Vpn => PathClassDto::Vpn,
            lanmsg_proto::types::PathClass::Internet => PathClassDto::Internet,
            lanmsg_proto::types::PathClass::Relay => PathClassDto::Relay,
        },
        is_contact,
        is_mine: self_user.is_some_and(|u| cert.is_some_and(|c| c.user == *u)),
    }
}

/// 将设备行转换为设备视图（online 按当前已连接设备集合判定；is_self 标本机）。
pub fn device_view(d: &DeviceRow, peers: &[PeerInfo], self_device: &DeviceId) -> DeviceView {
    DeviceView {
        device_id: d.device.to_hex(),
        device_short: d.device.short(),
        name: d.name.clone(),
        device_type: format!("{:?}", d.device_type),
        online: peers.iter().any(|p| p.device == d.device),
        path: d.last_path.clone().map(|p| match p {
            lanmsg_proto::types::PathClass::Lan => PathClassDto::Lan,
            lanmsg_proto::types::PathClass::Vpn => PathClassDto::Vpn,
            lanmsg_proto::types::PathClass::Internet => PathClassDto::Internet,
            lanmsg_proto::types::PathClass::Relay => PathClassDto::Relay,
        }),
        is_self: d.device == *self_device,
    }
}

/// 将联系人行转换为联系人视图（devices 取设备表，online 为任一设备已连接）。
pub fn contact_view(
    store: &Store,
    peers: &[PeerInfo],
    row: &ContactRow,
    self_device: &DeviceId,
) -> ContactView {
    let devices = store.devices_of_user(&row.user).unwrap_or_default();
    ContactView {
        user_id: row.user.to_hex(),
        user_short: row.user.short(),
        display_name: store
            .get_user(&row.user)
            .ok()
            .flatten()
            .map(|u| u.display_name)
            .unwrap_or_else(|| row.user.short()),
        online: devices
            .iter()
            .any(|d| peers.iter().any(|p| p.device == d.device)),
        devices: devices
            .iter()
            .map(|d| device_view(d, peers, self_device))
            .collect(),
        auto_accept: row.auto_accept,
        ask_above_bytes: row.ask_above_bytes,
    }
}

/// 将请求行转换为请求视图（from_name 取 users 表，无则 user short）。
pub fn request_view(store: &Store, r: &RequestRow) -> RequestView {
    let (from_name, from_short) = match store.get_user(&r.from_user).ok().flatten() {
        Some(u) => (u.display_name, r.from_user.short()),
        None => (r.from_user.short(), r.from_user.short()),
    };
    RequestView {
        request: r.request_id.clone(),
        kind: match r.kind {
            crate::store::RequestKindRow::Contact => RequestKind::Contact,
            crate::store::RequestKindRow::Message => RequestKind::Message,
            crate::store::RequestKindRow::File => RequestKind::File,
        },
        from_user: r.from_user.to_hex(),
        from_name,
        from_short,
        preview: r.preview.clone(),
        created_at: r.created_at,
    }
}
