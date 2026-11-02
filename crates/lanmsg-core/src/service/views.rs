// P1 核心骨架：Service 视图（Task 10）。
// message_view / conv_summary 将存储层记录转换为 API 层 DTO。
// 注释中文，标识符英文。

use std::path::PathBuf;

use lanmsg_api::types::{
    BlobState, ConvKind, ConversationSummary, FileItemView, MessageBodyView, MessageState,
    MessageView,
};
use lanmsg_proto::ids::{DeviceId, MsgId, UserId};
use lanmsg_proto::types::{BlobMeta, MsgBody};

use crate::settings::SETTING_DATA_DIR;
use crate::store::{ConvRow, MsgStateRow, PreparingKind, PreparingRow, Store, StoredMessage};

/// 由 Blob 元数据构造文件项视图（P2 Task 13 完整公式）。
///
/// - `state`：会话内有 preparing 行且条目名匹配 → `Preparing`；本地有 →
///   `Local`；该 hash 传输活动中（Queued/Running）→ `Downloading`；有已连接
///   的持有者 → `Remote`；否则 `SourceOffline`。
/// - `thumb_path`：meta.thumb 存在时写 `<data_dir>/cache/thumbs/<hash>.jpg`
///   （已存在则跳过写）并返回该路径。
/// - `source_label`：本地有 → `"来自 {作者} · 已下载"`；否则 `"来自 {作者} ·
///   存于 {首个在线持有者设备名 | 首个持有者设备名 | 作者设备名}"`。
///
/// `peers` 为当前已连接对端（在线持有者判定）；`author_display` 为已按
/// is_mine 解析的作者显示名（自己显示为"我"）。
pub(crate) fn file_item_view(
    store: &Store,
    peers: &[PeerInfo],
    author_device: &DeviceId,
    meta: &BlobMeta,
    author_display: &str,
) -> FileItemView {
    let local = store.get_local(&meta.hash).ok().flatten();
    let online_holder = store
        .holders(&meta.hash)
        .unwrap_or_default()
        .into_iter()
        .find(|d| peers.iter().any(|p| p.device == *d));
    // 会话内有同名条目的 preparing 行 → 正在准备（发送方重启场景兜底）。
    let in_preparing = store
        .list_preparing()
        .unwrap_or_default()
        .iter()
        .any(|row| {
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
    let source_label = if local.is_some() {
        format!("来自 {author_display} · 已下载")
    } else {
        let holder_name = online_holder
            .as_ref()
            .map(device_name_of)
            .unwrap_or_else(|| {
                holders
                    .first()
                    .map(device_name_of)
                    .unwrap_or_else(|| device_name_of(author_device))
            });
        format!("来自 {author_display} · 存于 {holder_name}")
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
        source_label,
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
                source_label: String::new(),
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

/// 将存储消息转换为消息视图。
///
/// - `author_device_name` 取 devices 表 name（无则 device short）。
/// - `is_mine` = 作者用户为自己（通过 devices 表的 user 字段比较）。
/// - `body`：Text → `Text`；Files/Folder 经 file_item_view（Task 13 完整
///   公式）；其余 → `Unsupported`。
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

    let (author_device_name, author_user_str) = match &author_row {
        Some(r) => (r.name.clone(), r.user.to_hex()),
        None => (author_device.short(), String::new()),
    };

    // 撤回提示与文件来源描述用作者显示名（users 表优先，无则设备名）。
    let author_display_name = author_user
        .and_then(|u| store.get_user(u).ok().flatten())
        .map(|ur| ur.display_name)
        .unwrap_or_else(|| author_device_name.clone());

    // 撤回的消息正文替换为系统提示（先于 Text 判定）；其余非 Text 体按
    // Files/Folder 展开，其他暂为 Unsupported。
    let body = match (&env.body, m.state) {
        (_, MsgStateRow::Recalled) => MessageBodyView::System {
            text: format!("{} 撤回了一条消息", author_display_name),
        },
        (MsgBody::Text { text }, _) => MessageBodyView::Text { text: text.clone() },
        (MsgBody::Files { items }, _) => MessageBodyView::Files {
            items: items
                .iter()
                .map(|b| file_item_view(store, peers, &author_device, b, &author_display_name))
                .collect(),
        },
        (MsgBody::Folder { name, entries }, _) => MessageBodyView::Folder {
            name: name.clone(),
            items: entries
                .iter()
                .map(|e| {
                    file_item_view(store, peers, &author_device, &e.blob, &author_display_name)
                })
                .collect(),
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

/// 将会话行转换为会话摘要。
///
/// - DM 标题 = 对端用户显示名（无则 user short）。
/// - SelfDevices 标题 = `"我的设备"`。
/// - `last_text`/`last_hlc_millis` 取最新一条消息。
/// - `unread` = 他人发出、晚于已读水位、未隐藏未删除未召回的条数。
pub fn conv_summary(
    store: &Store,
    self_user: Option<&UserId>,
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
