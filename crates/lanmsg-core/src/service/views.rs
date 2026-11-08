// P1 核心骨架：Service 视图（Task 10）。
// message_view / conv_summary 将存储层记录转换为 API 层 DTO。
// 注释中文，标识符英文。

use std::path::PathBuf;

use lanmsg_api::types::{
    BlobState, ConvKind, ConversationSummary, FileItemView, LastMessageKind, MessageBodyView,
    MessageState, MessageView, StationItemView, StationView, SystemEventView,
};
use lanmsg_proto::ids::{DeviceId, MsgId, UserId};
use lanmsg_proto::station::{StationContent, StationInfo};
use lanmsg_proto::types::{BlobMeta, MsgBody, SystemEvent};

use crate::service::Inner;
use crate::settings::SETTING_DATA_DIR;
use crate::store::{
    ConvRow, MsgStateRow, PreparingKind, PreparingRow, StationItemRow, Store, StoredMessage,
};

/// 由 Blob 元数据构造文件项视图（P2 Task 13 完整公式）。
///
/// - `state`：会话内有 preparing 行且条目名匹配 → `Preparing`；本地有 →
///   `Local`；该 hash 传输活动中（Queued/Running）→ `Downloading`；有已连接
///   的持有者 → `Remote`；否则 `SourceOffline`。
/// - `thumb_path`：meta.thumb 存在时写 `<data_dir>/cache/thumbs/<hash>.jpg`
///   （已存在则跳过写）并返回该路径。
/// - 来源：`source_name` = 作者显示名，`source_is_me` = 作者是否本机用户；
///   `holder_name`：本地没有时为 {首个在线持有者设备名 | 首个持有者设备名 |
///   作者设备名}，本地有时为 None。
///
/// `peers` 为当前已连接对端（在线持有者判定）；`author_display` 为作者显示名，
/// `author_is_me` 为作者是否本机用户。
pub(crate) fn file_item_view(
    store: &Store,
    peers: &[PeerInfo],
    author_device: &DeviceId,
    meta: &BlobMeta,
    author_display: &str,
    author_is_me: bool,
) -> FileItemView {
    let preparing = store.list_preparing().unwrap_or_default();
    file_item_view_with(
        store,
        peers,
        author_device,
        meta,
        author_display,
        author_is_me,
        &preparing,
    )
}

/// 同 `file_item_view`，但 preparing 行由调用方传入（批量构造时只查一次）。
pub(crate) fn file_item_view_with(
    store: &Store,
    peers: &[PeerInfo],
    author_device: &DeviceId,
    meta: &BlobMeta,
    author_display: &str,
    author_is_me: bool,
    preparing: &[PreparingRow],
) -> FileItemView {
    let local = store.get_local(&meta.hash).ok().flatten();
    let online_holder = store
        .holders(&meta.hash)
        .unwrap_or_default()
        .into_iter()
        .find(|d| peers.iter().any(|p| p.device == *d));
    // 会话内有同名条目的 preparing 行 → 正在准备（发送方重启场景兜底）。
    let in_preparing = preparing.iter().any(|row| {
        row.paths.iter().any(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy() == meta.name.as_str())
                .unwrap_or(false)
        })
    });
    let active_transfer = store
        .transfer_for_hash(&meta.hash)
        .ok()
        .flatten()
        .is_some_and(|t| {
            matches!(
                t.state,
                crate::store::TransferState::Queued | crate::store::TransferState::Running
            )
        });

    let state = if in_preparing {
        BlobState::Preparing
    } else if local.is_some() {
        BlobState::Local
    } else if active_transfer {
        BlobState::Downloading
    } else if online_holder.is_some() {
        BlobState::Remote
    } else {
        BlobState::SourceOffline
    };

    // 缩略图：meta.thumb 有内容时落盘到缓存目录（已存在则跳过写）。
    let thumb_path = match meta.thumb.as_deref() {
        Some(bytes) if !bytes.is_empty() => {
            let data_dir = store
                .get_setting(SETTING_DATA_DIR)
                .ok()
                .flatten()
                .map(PathBuf::from)
                .unwrap_or_default();
            let dir = data_dir.join("cache").join("thumbs");
            let path = dir.join(format!("{}.jpg", meta.hash.to_hex()));
            if !path.exists() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(&path, bytes);
            }
            Some(path.to_string_lossy().into_owned())
        }
        _ => None,
    };

    // 来源描述：设备名兜底链（在线持有者 → 首个持有者 → 作者设备）。
    let device_name_of = |d: &DeviceId| {
        store
            .get_device(d)
            .ok()
            .flatten()
            .map(|r| r.name)
            .unwrap_or_else(|| d.short())
    };
    let holders = store.holders(&meta.hash).unwrap_or_default();
    let holder_name = if local.is_some() {
        None
    } else {
        Some(
            online_holder
                .as_ref()
                .map(device_name_of)
                .unwrap_or_else(|| {
                    holders
                        .first()
                        .map(device_name_of)
                        .unwrap_or_else(|| device_name_of(author_device))
                }),
        )
    };

    FileItemView {
        hash: meta.hash.to_hex(),
        name: meta.name.clone(),
        size: meta.size,
        mime: meta.mime.clone(),
        width: meta.media.as_ref().map(|m| m.width),
        height: meta.media.as_ref().map(|m| m.height),
        duration_ms: meta.media.as_ref().and_then(|m| m.duration_ms),
        thumb_path,
        local_path: local
            .as_ref()
            .map(|l| l.path.to_string_lossy().into_owned()),
        state,
        source_name: name_or(author_display, author_device.short()),
        source_is_me: author_is_me,
        holder_name,
    }
}

/// 中转站视图：主机在线 = 主机是本机或已连接；条目数不含已删除。
pub(crate) fn station_view(inner: &Inner, s: &StationInfo) -> StationView {
    let me = inner
        .identity
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .device_id();
    let host_name = inner
        .store
        .get_device(&s.host)
        .ok()
        .flatten()
        .map(|d| d.name)
        .unwrap_or_else(|| s.host.short());
    StationView {
        station: s.station.to_hex(),
        name: s.name.clone(),
        host_device: host_name,
        host_online: s.host == me || inner.conns.is_connected(&s.host),
        is_local: s.host == me,
        item_count: inner.store.station_item_count(&s.station).unwrap_or(0),
    }
}

/// 中转站条目视图。Folder 取第一个文件的元数据，名字换成文件夹名、大小为总和。
pub(crate) fn station_item_view(inner: &Inner, row: &StationItemRow) -> StationItemView {
    let me = inner
        .identity
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .device_id();
    let item = &row.item;
    let added_by = inner
        .store
        .get_device(&item.added_by)
        .ok()
        .flatten()
        .map(|d| d.name)
        .unwrap_or_else(|| item.added_by.short());
    let peers = inner.conns.peers();
    let (text, meta) = match &item.content {
        StationContent::Text { text } => (Some(text.clone()), None),
        StationContent::File { blob } => (None, Some(blob.clone())),
        StationContent::Folder { name, entries } => (
            None,
            entries.first().map(|first| {
                let mut m = first.blob.clone();
                m.name = name.clone();
                m.size = entries.iter().map(|e| e.blob.size).sum();
                m
            }),
        ),
    };
    let file = meta.map(|m| {
        file_item_view(
            inner.store.as_ref(),
            &peers,
            &item.added_by,
            &m,
            &added_by,
            item.added_by == me,
        )
    });
    StationItemView {
        item: item.item.to_hex(),
        added_by,
        added_at: item.added_at,
        expires_at: item.expires_at,
        text,
        file,
        pinned: item.pinned,
        unavailable: (!item.available).then_some(true),
    }
}

/// preparing 行的消息视图（文件名与大小即时 stat 文件系统；文件缺失时
/// size 记 0）。阻塞函数（读文件元数据），调用方需在 spawn_blocking 中执行。
/// `state` 正常为 Sending；准备失败时调用方传 Failed。
///
/// paths 约定（与 Messaging::send_prepare 一致）：Folder 时 `paths[0]` 为
/// 根目录，其余为条目文件；Files 时全部为文件。
pub fn preparing_view(
    store: &Store,
    row: &PreparingRow,
    self_user: Option<&UserId>,
    state: MessageState,
) -> MessageView {
    let author_row = store.get_device(&row.author).ok().flatten();
    let is_mine = author_row
        .as_ref()
        .zip(self_user)
        .map(|(r, s)| r.user == *s)
        .unwrap_or(false);
    let author_device_name = author_row
        .as_ref()
        .map(|r| r.name.clone())
        .unwrap_or_else(|| row.author.short());
    let author_user_str = author_row
        .as_ref()
        .map(|r| r.user.to_hex())
        .unwrap_or_default();
    // 文件项的来源名：作者显示名（users 表优先），缺失时退回设备名（至少是设备短 ID）。
    let author_display = author_row
        .as_ref()
        .and_then(|r| store.get_user(&r.user).ok().flatten())
        .map(|u| u.display_name)
        .unwrap_or_else(|| author_device_name.clone());
    let source_name = name_or(&author_display, row.author.short());

    let (folder_name, paths): (Option<String>, &[PathBuf]) = match row.kind {
        PreparingKind::Files => (None, &row.paths),
        PreparingKind::Folder => (
            row.paths.first().map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }),
            row.paths.get(1..).unwrap_or(&[]),
        ),
    };
    let items = paths
        .iter()
        .map(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            let mime = crate::blob::mime::mime_from_name(&name);
            FileItemView {
                hash: String::new(),
                name,
                size,
                mime: mime.into(),
                width: None,
                height: None,
                duration_ms: None,
                thumb_path: None,
                local_path: None,
                state: BlobState::Preparing,
                source_name: source_name.clone(),
                source_is_me: is_mine,
                holder_name: None,
            }
        })
        .collect();
    let body = match folder_name {
        None => MessageBodyView::Files { items },
        Some(name) => MessageBodyView::Folder { name, items },
    };

    MessageView {
        id: MsgId {
            author: row.author,
            seq: row.seq,
        }
        .to_string(),
        conv: row.conv.to_hex(),
        author_user: author_user_str,
        author_device: row.author.to_hex(),
        author_device_name,
        is_mine,
        hlc_millis: row.hlc.millis,
        body,
        state,
    }
}

/// 名字为空时用兜底值（设备或用户短 ID），保证界面文案里的名字非空。
fn name_or(name: &str, fallback: String) -> String {
    if name.trim().is_empty() {
        fallback
    } else {
        name.to_string()
    }
}

/// 系统事件的结构化视图；界面按本地化资源拼文案。
fn system_event_view(store: &Store, event: &SystemEvent) -> SystemEventView {
    match event {
        SystemEvent::DeviceAdded { device, name } => SystemEventView::DeviceAdded {
            name: name_or(name, device.short()),
        },
        SystemEvent::DeviceRemoved { device } => SystemEventView::DeviceRemoved {
            name: store
                .get_device(device)
                .ok()
                .flatten()
                .map(|r| name_or(&r.name, device.short()))
                .unwrap_or_else(|| device.short()),
        },
        SystemEvent::ContactAdded { user } => SystemEventView::ContactAdded {
            name: store
                .get_user(user)
                .ok()
                .flatten()
                .map(|u| name_or(&u.display_name, user.short()))
                .unwrap_or_else(|| user.short()),
        },
        SystemEvent::ContactUpdated(rec) => SystemEventView::ContactUpdated {
            name: name_or(&rec.display_name, rec.user.short()),
        },
        SystemEvent::ProfileUpdated { display_name } => SystemEventView::NicknameUpdated {
            name: name_or(display_name, "?".into()),
        },
        SystemEvent::LocalDeleted { msgs } => SystemEventView::MessagesDeleted {
            count: msgs.len() as u32,
        },
        SystemEvent::LocalSendPin { name } => SystemEventView::LocalSendPin {
            name: name_or(name, "?".into()),
        },
        SystemEvent::LocalSendBusy { name } => SystemEventView::LocalSendBusy {
            name: name_or(name, "?".into()),
        },
    }
}

/// 将存储消息转换为消息视图。
///
/// - `author_device_name` 取 devices 表 name（无则 device short）。
/// - `is_mine` = 作者用户为自己（通过 devices 表的 user 字段比较）。
/// - `body`：Text → `Text`；Files/Folder 经 file_item_view（Task 13 完整
///   公式）；System → `System{event}`；其余 → `Unsupported`。
/// - `state` 由 `MsgStateRow` 一一映射。
pub fn message_view(
    store: &Store,
    self_device: &DeviceId,
    peers: &[PeerInfo],
    m: &StoredMessage,
) -> MessageView {
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

    // devices 表里没有的作者可能是 LocalSend 对端的伪设备：用其别名作显示名。
    let (author_device_name, author_user_str) = match &author_row {
        Some(r) => (r.name.clone(), r.user.to_hex()),
        None => (
            store
                .ls_peer_by_pseudo(&author_device)
                .ok()
                .flatten()
                .map(|p| name_or(&p.alias, author_device.short()))
                .unwrap_or_else(|| author_device.short()),
            String::new(),
        ),
    };

    // 撤回提示与文件来源描述用作者显示名（users 表优先，无则设备名）。
    let author_display_name = author_user
        .and_then(|u| store.get_user(u).ok().flatten())
        .map(|ur| ur.display_name)
        .unwrap_or_else(|| author_device_name.clone());

    // 撤回的消息正文替换为系统提示（先于 Text 判定）；其余非 Text 体按
    // Files/Folder 展开，System 生成文本，其他为 Unsupported。
    let body = match (&env.body, m.state) {
        (_, MsgStateRow::Recalled) => MessageBodyView::System {
            event: SystemEventView::Recalled {
                author_name: name_or(&author_display_name, author_device.short()),
                by_me: is_mine,
            },
        },
        (MsgBody::Text { text }, _) => MessageBodyView::Text { text: text.clone() },
        (MsgBody::Files { items }, _) => MessageBodyView::Files {
            items: items
                .iter()
                .map(|b| {
                    file_item_view(
                        store,
                        peers,
                        &author_device,
                        b,
                        &author_display_name,
                        is_mine,
                    )
                })
                .collect(),
        },
        (MsgBody::Folder { name, entries }, _) => MessageBodyView::Folder {
            name: name.clone(),
            items: entries
                .iter()
                .map(|e| {
                    file_item_view(
                        store,
                        peers,
                        &author_device,
                        &e.blob,
                        &author_display_name,
                        is_mine,
                    )
                })
                .collect(),
        },
        (MsgBody::System { event }, _) => MessageBodyView::System {
            event: system_event_view(store, event),
        },
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

/// 对外事件用的消息视图：隐藏消息（已读回执、撤回、资料更新等）返回 None，
/// 所有发 MessageAdded/MessageUpdated 的地方都经此构造，避免隐藏消息漏到界面。
pub fn visible_message_view(
    store: &Store,
    self_device: &DeviceId,
    peers: &[PeerInfo],
    m: &StoredMessage,
) -> Option<MessageView> {
    if lanmsg_proto::body_is_hidden(&m.env.body) {
        return None;
    }
    Some(message_view(store, self_device, peers, m))
}

/// 最后一条消息的结构化类型；已撤回的消息不暴露原内容。
fn last_message_kind(m: &StoredMessage) -> LastMessageKind {
    if m.state == MsgStateRow::Recalled {
        return LastMessageKind::Recalled;
    }
    match &m.env.body {
        MsgBody::Text { .. } => LastMessageKind::Text,
        // 空文件列表（理论上不会出现）：按无名单文件处理，由各端显示不带名字的“[文件]”。
        MsgBody::Files { items } => {
            let count = items.len() as u32;
            if !items.is_empty() && items.iter().all(|b| b.mime.starts_with("image/")) {
                LastMessageKind::Image { count }
            } else if !items.is_empty() && items.iter().all(|b| b.mime.starts_with("video/")) {
                LastMessageKind::Video { count }
            } else {
                LastMessageKind::File {
                    name: items.first().map(|b| b.name.clone()).unwrap_or_default(),
                    count,
                }
            }
        }
        MsgBody::Folder { name, .. } => LastMessageKind::Folder { name: name.clone() },
        _ => LastMessageKind::System,
    }
}

/// 将会话行转换为会话摘要。
///
/// - DM 标题 = 对端用户显示名（无则 user short）。
/// - SelfDevices 标题 = 空串（界面按 kind 本地化）。
/// - `last_text`/`last_kind`/`last_hlc_millis` 取最新一条可见消息（隐藏的不计）。
/// - `unread` = 他人发出、晚于已读水位、未隐藏未删除未召回的条数。
pub fn conv_summary(
    store: &Store,
    self_user: Option<&UserId>,
    conv: &ConvRow,
    peer_online: bool,
) -> ConversationSummary {
    // LocalSend 会话的在线状态来自对端最近一次被发现的时间。
    let peer_online = peer_online
        || (conv.kind == crate::store::ConvKindRow::LocalSend
            && store.ls_conv_online(&conv.conv, crate::service::now_millis()));
    let title = match conv.kind {
        crate::store::ConvKindRow::Dm => conv
            .peer_user
            .as_ref()
            .and_then(|u| store.get_user(u).ok().flatten())
            .map(|u| u.display_name.clone())
            .unwrap_or_else(|| conv.peer_user.map(|u| u.short()).unwrap_or_default()),
        // 自设备会话标题留空，各端按 kind 显示本地化的“我的设备”。
        crate::store::ConvKindRow::SelfDevices => String::new(),
        _ => conv.title.clone().unwrap_or_default(),
    };

    let messages = store.list_messages(&conv.conv, None, 1).unwrap_or_default();
    let (last_text, last_kind, last_hlc_millis) = messages
        .into_iter()
        .next()
        .map(|m| {
            let text = match (&m.env.body, m.state) {
                (_, MsgStateRow::Recalled) => None,
                (MsgBody::Text { text }, _) => Some(text.clone()),
                _ => None,
            };
            (text, Some(last_message_kind(&m)), Some(m.env.hlc.millis))
        })
        .unwrap_or((None, None, None));

    // 中转站会话：最新条目的文字前 50 字或文件名。
    let last_text = if conv.kind == crate::store::ConvKindRow::Station {
        store
            .station_items(&conv.conv, 0, false)
            .ok()
            .and_then(|rows| {
                rows.into_iter()
                    .max_by_key(|r| (r.item.updated_at, r.item.added_at))
            })
            .map(|r| match r.item.content {
                StationContent::Text { text } => text.chars().take(50).collect(),
                StationContent::File { blob } => blob.name,
                StationContent::Folder { name, .. } => name,
            })
    } else {
        last_text
    };

    let kind = match conv.kind {
        crate::store::ConvKindRow::SelfDevices => ConvKind::SelfDevices,
        crate::store::ConvKindRow::Dm => ConvKind::Dm,
        crate::store::ConvKindRow::Guest => ConvKind::Guest,
        crate::store::ConvKindRow::LocalSend => ConvKind::LocalSend,
        crate::store::ConvKindRow::Station => ConvKind::Station,
    };

    // 未读数按会话已读水位计算；未初始化（无 self_user）时恒 0。
    let unread = match self_user {
        Some(u) => {
            let after = store
                .last_read(&conv.conv)
                .unwrap_or(lanmsg_proto::types::Hlc {
                    millis: 0,
                    counter: 0,
                });
            store.unread_count(&conv.conv, u, after).unwrap_or(0)
        }
        None => 0,
    };

    ConversationSummary {
        conv: conv.conv.to_hex(),
        kind,
        title,
        last_text,
        last_kind,
        last_hlc_millis,
        unread,
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
/// `addr` 用发现来源地址，`path` 按 `classify(.., via_multicast)` 判定：
/// 组播来源判局域网，单播来源（例如用户配置的跨网段目标）按真实网卡判定。
pub fn discovered_peer_view(
    store: &Store,
    ann: &lanmsg_proto::types::Announce,
    addr: std::net::SocketAddr,
    via_multicast: bool,
    self_user: Option<&UserId>,
) -> PeerView {
    let cert = &ann.cert;
    let path = crate::net::classify(addr.ip(), &crate::net::path::cached_ifaces(), via_multicast);
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

/// 将设备行转换为设备视图（online：本机恒为 true，其余按当前已连接设备集合判定）。
pub fn device_view(d: &DeviceRow, peers: &[PeerInfo], self_device: &DeviceId) -> DeviceView {
    let is_self = d.device == *self_device;
    DeviceView {
        device_id: d.device.to_hex(),
        device_short: d.device.short(),
        name: d.name.clone(),
        device_type: format!("{:?}", d.device_type),
        // 本机不在 peers 里，需单独判定，否则本机永远显示离线。
        online: is_self || peers.iter().any(|p| p.device == d.device),
        path: d.last_path.clone().map(|p| match p {
            lanmsg_proto::types::PathClass::Lan => PathClassDto::Lan,
            lanmsg_proto::types::PathClass::Vpn => PathClassDto::Vpn,
            lanmsg_proto::types::PathClass::Internet => PathClassDto::Internet,
            lanmsg_proto::types::PathClass::Relay => PathClassDto::Relay,
        }),
        is_self,
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
        source_addr: None,
        addr_changed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ConvKindRow, MsgStateRow};
    use lanmsg_proto::ids::{BlobHash, ConvId};
    use lanmsg_proto::types::{Envelope, Hlc, Signature64};

    const CONV: ConvId = ConvId([1u8; 32]);
    const AUTHOR: DeviceId = DeviceId([2u8; 32]);

    fn conv_row() -> ConvRow {
        ConvRow {
            conv: CONV,
            kind: ConvKindRow::Guest,
            peer_user: None,
            title: Some("t".into()),
            created_at: 0,
        }
    }

    fn blob(name: &str, mime: &str, tag: u8) -> BlobMeta {
        BlobMeta {
            hash: BlobHash([tag; 32]),
            size: 1,
            name: name.into(),
            mime: mime.into(),
            media: None,
            thumb: None,
        }
    }

    fn put(s: &Store, seq: u64, body: MsgBody) -> MsgId {
        let id = MsgId {
            author: AUTHOR,
            seq,
        };
        let env = Envelope {
            conv: CONV,
            id: id.clone(),
            conv_seq: seq,
            hlc: Hlc {
                millis: 100 + seq as i64,
                counter: 0,
            },
            body,
            enc: None,
            sig: Signature64([0u8; 64]),
        };
        s.insert_envelope(&env, MsgStateRow::Delivered, 1).unwrap();
        id
    }

    fn last(s: &Store) -> ConversationSummary {
        conv_summary(s, None, &conv_row(), false)
    }

    #[test]
    fn last_kind_is_none_without_messages() {
        let s = Store::open_in_memory().unwrap();
        let sum = last(&s);
        assert_eq!(sum.last_kind, None);
        assert_eq!(sum.last_text, None);
    }

    #[test]
    fn last_kind_text_image_video_file_folder() {
        let s = Store::open_in_memory().unwrap();
        put(&s, 1, MsgBody::Text { text: "hi".into() });
        let sum = last(&s);
        assert_eq!(sum.last_kind, Some(LastMessageKind::Text));
        assert_eq!(sum.last_text.as_deref(), Some("hi"));

        put(
            &s,
            2,
            MsgBody::Files {
                items: vec![
                    blob("a.png", "image/png", 1),
                    blob("b.jpg", "image/jpeg", 2),
                ],
            },
        );
        let sum = last(&s);
        assert_eq!(sum.last_kind, Some(LastMessageKind::Image { count: 2 }));
        assert_eq!(sum.last_text, None);

        put(
            &s,
            3,
            MsgBody::Files {
                items: vec![blob("v.mp4", "video/mp4", 3)],
            },
        );
        assert_eq!(
            last(&s).last_kind,
            Some(LastMessageKind::Video { count: 1 })
        );

        put(
            &s,
            4,
            MsgBody::Files {
                items: vec![
                    blob("r.pdf", "application/pdf", 4),
                    blob("a.png", "image/png", 5),
                ],
            },
        );
        assert_eq!(
            last(&s).last_kind,
            Some(LastMessageKind::File {
                name: "r.pdf".into(),
                count: 2
            })
        );

        put(
            &s,
            5,
            MsgBody::Folder {
                name: "照片".into(),
                entries: vec![],
            },
        );
        assert_eq!(
            last(&s).last_kind,
            Some(LastMessageKind::Folder {
                name: "照片".into()
            })
        );
    }

    #[test]
    fn empty_files_message_falls_back_to_nameless_file() {
        let s = Store::open_in_memory().unwrap();
        put(&s, 1, MsgBody::Files { items: vec![] });
        assert_eq!(
            last(&s).last_kind,
            Some(LastMessageKind::File {
                name: String::new(),
                count: 0
            })
        );
    }

    #[test]
    fn recalled_last_message_does_not_leak_content() {
        let s = Store::open_in_memory().unwrap();
        let id = put(
            &s,
            1,
            MsgBody::Text {
                text: "秘密".into(),
            },
        );
        s.set_message_state(&id, MsgStateRow::Recalled).unwrap();
        let sum = last(&s);
        assert_eq!(sum.last_kind, Some(LastMessageKind::Recalled));
        assert_eq!(sum.last_text, None);
    }

    #[test]
    fn hidden_messages_are_not_previewed() {
        let s = Store::open_in_memory().unwrap();
        put(
            &s,
            1,
            MsgBody::Text {
                text: "可见".into(),
            },
        );
        put(
            &s,
            2,
            MsgBody::Read {
                upto: Hlc {
                    millis: 1,
                    counter: 0,
                },
            },
        );
        let sum = last(&s);
        assert_eq!(sum.last_kind, Some(LastMessageKind::Text));
        assert_eq!(sum.last_text.as_deref(), Some("可见"));
    }

    fn device_row(id: u8) -> crate::store::DeviceRow {
        let device = DeviceId([id; 32]);
        let user = UserId([9u8; 32]);
        crate::store::DeviceRow {
            device,
            user,
            cert: lanmsg_proto::types::DeviceCert {
                device,
                user,
                device_name: "d".into(),
                device_type: lanmsg_proto::types::DeviceType::Desktop,
                created_at: 0,
                key_epoch: 0,
                sig: Signature64([0u8; 64]),
            },
            name: "d".into(),
            device_type: lanmsg_proto::types::DeviceType::Desktop,
            last_addr: None,
            last_path: None,
            last_seen: None,
            revoked: false,
        }
    }

    #[test]
    fn self_device_is_online_even_without_peers() {
        let me = device_row(1);
        let other = device_row(2);
        assert!(device_view(&me, &[], &me.device).online);
        assert!(!device_view(&other, &[], &me.device).online);
    }

    // ---- 系统事件 / 撤回 / 会话标题的结构化 ----

    use lanmsg_proto::ids::UserId;
    use lanmsg_proto::types::{ContactRecord, ContactStateWire};

    fn device_event_view(s: &Store, ev: SystemEvent) -> SystemEventView {
        system_event_view(s, &ev)
    }

    #[test]
    fn system_event_view_per_variant() {
        let s = Store::open_in_memory().unwrap();
        let dev = DeviceId([9u8; 32]);
        let user = UserId([8u8; 32]);
        // 名字齐全：原样带出。
        assert_eq!(
            device_event_view(
                &s,
                SystemEvent::DeviceAdded {
                    device: dev,
                    name: "手机".into()
                }
            ),
            SystemEventView::DeviceAdded {
                name: "手机".into()
            }
        );
        // 设备未知（已被删）：用设备短 ID 兜底，名字非空。
        assert_eq!(
            device_event_view(&s, SystemEvent::DeviceRemoved { device: dev }),
            SystemEventView::DeviceRemoved { name: dev.short() }
        );
        // 用户未知：用户短 ID 兜底。
        assert_eq!(
            device_event_view(&s, SystemEvent::ContactAdded { user }),
            SystemEventView::ContactAdded { name: user.short() }
        );
        let rec = ContactRecord {
            user,
            display_name: String::new(),
            state: ContactStateWire::Accepted,
            devices: Vec::new(),
            auto_accept: false,
            ask_above_bytes: None,
            updated: Hlc {
                millis: 0,
                counter: 0,
            },
        };
        assert_eq!(
            device_event_view(&s, SystemEvent::ContactUpdated(rec)),
            SystemEventView::ContactUpdated { name: user.short() }
        );
        assert_eq!(
            device_event_view(
                &s,
                SystemEvent::ProfileUpdated {
                    display_name: "小李".into()
                }
            ),
            SystemEventView::NicknameUpdated {
                name: "小李".into()
            }
        );
        assert_eq!(
            device_event_view(&s, SystemEvent::LocalDeleted { msgs: Vec::new() }),
            SystemEventView::MessagesDeleted { count: 0 }
        );
    }

    #[test]
    fn names_are_never_empty() {
        let s = Store::open_in_memory().unwrap();
        let dev = DeviceId([9u8; 32]);
        match device_event_view(
            &s,
            SystemEvent::DeviceAdded {
                device: dev,
                name: "  ".into(),
            },
        ) {
            SystemEventView::DeviceAdded { name } => assert_eq!(name, dev.short()),
            other => panic!("unexpected {other:?}"),
        }
        match device_event_view(
            &s,
            SystemEvent::ProfileUpdated {
                display_name: String::new(),
            },
        ) {
            SystemEventView::NicknameUpdated { name } => assert!(!name.is_empty()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn recalled_by_me_sets_flag() {
        let s = Store::open_in_memory().unwrap();
        let id = put(&s, 1, MsgBody::Text { text: "x".into() });
        s.set_message_state(&id, MsgStateRow::Recalled).unwrap();
        let stored = s.get_message(&id).unwrap().unwrap();
        // 作者设备不在 devices 表：既非“我”，名字用设备短 ID 兜底。
        let v = message_view(&s, &DeviceId([7u8; 32]), &[], &stored);
        match v.body {
            MessageBodyView::System {
                event: SystemEventView::Recalled { author_name, by_me },
            } => {
                assert!(!by_me);
                assert_eq!(author_name, AUTHOR.short());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// 作者设备与本机设备同属一个用户 → by_me 为真，名字取用户显示名。
    #[test]
    fn recalled_by_own_user_sets_by_me_true() {
        use crate::store::{DeviceRow, UserRow};
        use lanmsg_proto::sign::UserKey;
        use lanmsg_proto::types::DeviceType;
        let s = Store::open_in_memory().unwrap();
        let user_key = UserKey::generate();
        let user = user_key.user_id();
        s.upsert_user(&UserRow {
            user,
            display_name: "小李".into(),
            is_self: true,
            updated_at: 0,
        })
        .unwrap();
        let self_dev = DeviceId([7u8; 32]);
        for dev in [AUTHOR, self_dev] {
            s.upsert_device(&DeviceRow {
                device: dev,
                user,
                cert: lanmsg_proto::types::DeviceCert::issue(
                    &user_key,
                    dev,
                    "pc".into(),
                    DeviceType::Desktop,
                    1,
                ),
                name: "pc".into(),
                device_type: DeviceType::Desktop,
                last_addr: None,
                last_path: None,
                last_seen: None,
                revoked: false,
            })
            .unwrap();
        }
        let id = put(&s, 1, MsgBody::Text { text: "x".into() });
        s.set_message_state(&id, MsgStateRow::Recalled).unwrap();
        let stored = s.get_message(&id).unwrap().unwrap();
        let v = message_view(&s, &self_dev, &[], &stored);
        match v.body {
            MessageBodyView::System {
                event: SystemEventView::Recalled { author_name, by_me },
            } => {
                assert!(by_me);
                assert_eq!(author_name, "小李");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// preparing_view 产出的每个文件项都有非空的来源名；本人发出时 source_is_me 为真。
    #[test]
    fn preparing_items_always_have_source_name() {
        use crate::store::PreparingRow;
        let s = Store::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("lanmsg-prep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f1 = dir.join("a.txt");
        std::fs::write(&f1, b"x").unwrap();
        let row = PreparingRow {
            conv: CONV,
            author: AUTHOR,
            seq: 1,
            conv_seq: 1,
            hlc: Hlc {
                millis: 1,
                counter: 0,
            },
            kind: PreparingKind::Files,
            paths: vec![f1, dir.join("missing.bin")],
            created_at: 0,
        };
        // 作者设备不在 devices 表：来源名退回设备短 ID，非本人。
        let v = preparing_view(&s, &row, None, MessageState::Sending);
        match v.body {
            MessageBodyView::Files { items } => {
                assert_eq!(items.len(), 2);
                for it in items {
                    assert_eq!(it.source_name, AUTHOR.short());
                    assert!(!it.source_is_me);
                    assert_eq!(it.holder_name, None);
                }
            }
            other => panic!("unexpected {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn self_devices_title_is_empty() {
        let s = Store::open_in_memory().unwrap();
        let mut row = conv_row();
        row.kind = ConvKindRow::SelfDevices;
        row.title = Some("whatever".into());
        let sum = conv_summary(&s, None, &row, false);
        assert_eq!(sum.kind, ConvKind::SelfDevices);
        assert_eq!(sum.title, "");
    }
}
