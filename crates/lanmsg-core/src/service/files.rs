// P3 Task 15：文件视图查询、消息文字搜索、文件转发。
// 注释中文，标识符英文。

use std::collections::HashMap;

use lanmsg_api::types::{BlobState, ConvKind, FileFilter, FileView, MessageView};
use lanmsg_proto::ids::{BlobHash, ConvId, DeviceId, UserId};
use lanmsg_proto::types::{BlobMeta, MsgBody};

use crate::error::CoreError;
use crate::net::PeerInfo;
use crate::service::views::{conv_summary, file_item_view_with, message_view};
use crate::service::Inner;
use crate::store::file_index::kind_str;
use crate::store::{ConvKindRow, FileQuery, Store};

/// 文件页单次查询条数上限。
const MAX_FILE_LIMIT: u32 = 500;
/// 带状态筛选时单次请求最多扫描的行数，防止病态数据（几乎无命中）长时间占用线程。
const MAX_SCAN_ROWS: u32 = 20_000;

/// 文件视图查询：按筛选条件分页，状态筛选在内存中做（状态是动态计算的，不入库）。
/// 查询在 spawn_blocking 中执行，不占用 tokio worker。
pub(crate) async fn query_files(
    svc: &Inner,
    filter: &FileFilter,
) -> Result<Vec<FileView>, CoreError> {
    let conv = filter.conv.as_deref().map(parse_conv).transpose()?;
    let author = filter
        .source_device
        .as_deref()
        .map(|s| {
            DeviceId::from_hex(s)
                .map_err(|_| CoreError::InvalidArgument(format!("invalid device ID: {s}")))
        })
        .transpose()?;
    let limit = filter.limit.clamp(1, MAX_FILE_LIMIT);
    let base = FileQuery {
        kind: kind_str(&filter.kind).map(String::from),
        conv,
        author,
        name: filter.name_query.clone(),
        before_millis: filter.before_millis,
        limit,
    };
    let state = filter.state.clone();
    let store = svc.store.clone();
    let peers = svc.conns.peers();
    let self_user = svc.state.read().unwrap().self_user;
    tokio::task::spawn_blocking(move || scan_files(&store, &peers, self_user, base, state))
        .await
        .map_err(|e| {
            CoreError::Io(std::io::Error::other(format!(
                "file query task failed: {e}"
            )))
        })?
}

/// 本机路径位于 `under` 之下、且已“全部送达”的本地副本路径。
pub(crate) async fn delivered_local_paths(
    svc: &Inner,
    under: String,
) -> Result<Vec<String>, CoreError> {
    let store = svc.store.clone();
    let self_user = svc.state.read().unwrap().self_user;
    let self_device = svc.identity.read().unwrap().device_id();
    tokio::task::spawn_blocking(move || {
        delivered_paths(&store, self_user.as_ref(), &self_device, &under)
    })
    .await
    .map_err(|e| {
        CoreError::Io(std::io::Error::other(format!(
            "delivered paths task failed: {e}"
        )))
    })?
}

/// “已全部送达”判定（保守）：路径在 `under` 之下的本地 Blob，其引用消息里至少有一条
/// 是本机用户发出的，且这些消息所在会话的**每个其他成员的每台未吊销设备**都已在
/// `blob_holders` 里。
/// - DM：对端用户的全部设备；自设备会话：本人用户除本机外的全部设备；其他类型会话
///   （访客、LocalSend、中转站）无法判定成员，一律不算送达。
/// - 同一哈希只要还被别人发出的消息引用就不算送达（本地只有一个副本）。
/// - 没有任何接收设备（如只有本机的自设备会话）不算送达：之后加入的设备还需要这份文件。
/// - 成员的某台设备从未下载（含从未上线）就会一直挡住，直到它下载或被吊销；
///   这是有意的保守取舍——绝不把对端尚未取走的文件判为可删。
fn delivered_paths(
    store: &Store,
    self_user: Option<&UserId>,
    self_device: &DeviceId,
    under: &str,
) -> Result<Vec<String>, CoreError> {
    let Some(self_user) = self_user else {
        return Ok(Vec::new());
    };
    let under_path = std::path::Path::new(under);
    let mut out = Vec::new();
    for (hash, path) in store.local_paths_with_prefix(under)? {
        if !std::path::Path::new(&path).starts_with(under_path) {
            continue;
        }
        if blob_delivered(store, self_user, self_device, &hash)? {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn blob_delivered(
    store: &Store,
    self_user: &UserId,
    self_device: &DeviceId,
    hash: &BlobHash,
) -> Result<bool, CoreError> {
    let holders: std::collections::HashSet<DeviceId> = store.holders(hash)?.into_iter().collect();
    let mut any_own = false;
    for mb in store.message_blobs_by_hash(hash)? {
        let own = store
            .get_device(&mb.msg.author)?
            .is_some_and(|d| d.user == *self_user);
        if !own {
            // 同一哈希也被别人的消息引用：本地只有这一个副本，删了对方消息里的
            // 同一个文件也打不开，保守地不算送达。
            return Ok(false);
        }
        any_own = true;
        let Some(conv) = store.get_conversation(&mb.conv)? else {
            return Ok(false);
        };
        let recipients: Vec<DeviceId> = match conv.kind {
            ConvKindRow::Dm => match conv.peer_user {
                Some(u) => store
                    .devices_of_user(&u)?
                    .into_iter()
                    .filter(|d| !d.revoked)
                    .map(|d| d.device)
                    .collect(),
                None => return Ok(false),
            },
            ConvKindRow::SelfDevices => store
                .devices_of_user(self_user)?
                .into_iter()
                .filter(|d| !d.revoked && d.device != *self_device)
                .map(|d| d.device)
                .collect(),
            _ => return Ok(false),
        };
        if recipients.is_empty() || !recipients.iter().all(|d| holders.contains(d)) {
            return Ok(false);
        }
    }
    Ok(any_own)
}

/// 阻塞扫描：无状态筛选时一轮；有状态筛选时持续翻页直到凑满 limit 或数据耗尽。
/// 扫描行数达到 `MAX_SCAN_ROWS` 时返回已得结果——此时客户端按“满页=还有更多”
/// 的约定会认为没有更多，这是对病态数据的有意取舍。
fn scan_files(
    store: &Store,
    peers: &[PeerInfo],
    self_user: Option<UserId>,
    base: FileQuery,
    state: Option<BlobState>,
) -> Result<Vec<FileView>, CoreError> {
    let limit = base.limit;
    let preparing = store.list_preparing()?;
    let mut titles: HashMap<ConvId, (String, ConvKind)> = HashMap::new();
    let mut out: Vec<FileView> = Vec::new();
    let mut before = base.before_millis;
    let mut scanned: u32 = 0;
    loop {
        let rows = store.query_file_rows(&FileQuery {
            before_millis: before,
            ..base.clone()
        })?;
        let Some(last) = rows.last() else { break };
        before = Some(last.hlc_millis);
        let full = rows.len() as u32 >= limit;
        scanned = scanned.saturating_add(rows.len() as u32);
        for row in &rows {
            let Some(stored) = store.get_message(&row.msg)? else {
                continue;
            };
            let blobs = store.message_blobs_of(&row.msg)?;
            let Some(mb) = blobs.into_iter().find(|b| b.idx == row.item_idx) else {
                continue;
            };
            let author_display = author_display_name(store, &row.msg.author);
            let author_is_me = is_own_device(store, self_user.as_ref(), &row.msg.author);
            let item = file_item_view_with(
                store,
                peers,
                &row.msg.author,
                &mb.meta,
                &author_display,
                author_is_me,
                &preparing,
            );
            if state.as_ref().is_some_and(|s| *s != item.state) {
                continue;
            }
            let (title, kind) = titles
                .entry(row.conv)
                .or_insert_with(|| conv_title_kind(store, self_user.as_ref(), &row.conv))
                .clone();
            out.push(FileView {
                item,
                conv: row.conv.to_hex(),
                conv_title: title,
                conv_kind: kind,
                msg: stored.env.id.to_string(),
                hlc_millis: row.hlc_millis,
            });
        }
        if state.is_none() || out.len() as u32 >= limit || !full || scanned >= MAX_SCAN_ROWS {
            break;
        }
    }
    Ok(out)
}

/// 消息文字搜索：先查索引拿 id，再逐条还原视图（取不到的跳过）。
/// 在 spawn_blocking 中执行。
pub(crate) async fn search_messages(
    svc: &Inner,
    query: &str,
    conv: Option<&ConvId>,
    limit: u32,
) -> Result<Vec<MessageView>, CoreError> {
    let store = svc.store.clone();
    let self_device = svc.identity.read().unwrap().device_id();
    let peers = svc.conns.peers();
    let query = query.to_string();
    let conv = conv.copied();
    tokio::task::spawn_blocking(move || -> Result<Vec<MessageView>, CoreError> {
        let ids = store.search_messages(&query, conv.as_ref(), limit)?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(m) = store.get_message(&id)? {
                out.push(message_view(&store, &self_device, &peers, &m));
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| CoreError::Io(std::io::Error::other(format!("search task failed: {e}"))))?
}

/// 把已有文件转发到会话：复用库中已有的 BlobMeta（名字、大小、mime、媒体信息、
/// 缩略图），不重新哈希，也不要求本机持有原件；接收方按 P2 机制向持有者拉取。
pub(crate) async fn forward_files(
    svc: &Inner,
    conv: &ConvId,
    hashes: &[BlobHash],
) -> Result<lanmsg_proto::ids::MsgId, CoreError> {
    if hashes.is_empty() {
        return Err(CoreError::InvalidArgument("no files to forward".into()));
    }
    if svc.store.get_conversation(conv)?.is_none() {
        return Err(CoreError::NotFound(format!(
            "conversation {}",
            conv.to_hex()
        )));
    }
    let mut items: Vec<BlobMeta> = Vec::with_capacity(hashes.len());
    for h in hashes {
        let Some(row) = svc.store.visible_blobs_by_hash(h)?.into_iter().next() else {
            return Err(CoreError::NotFound(format!("file {}", h.to_hex())));
        };
        items.push(row.meta);
    }
    // 锁已在上面各步骤中释放，这里才 await。
    svc.msg.send(conv, MsgBody::Files { items }).await
}

fn parse_conv(s: &str) -> Result<ConvId, CoreError> {
    ConvId::from_hex(s)
        .map_err(|_| CoreError::InvalidArgument(format!("invalid conversation ID: {s}")))
}

/// 作者显示名：users 表显示名优先，无则设备名（与 message_view 一致）。
fn author_display_name(store: &Store, author: &DeviceId) -> String {
    let row = store.get_device(author).ok().flatten();
    match row {
        Some(r) => store
            .get_user(&r.user)
            .ok()
            .flatten()
            .map(|u| u.display_name)
            .unwrap_or(r.name),
        None => author.short(),
    }
}

/// 会话标题与类型（规则同会话列表）；会话不存在时回退为空串与 Guest。
fn conv_title_kind(store: &Store, self_user: Option<&UserId>, conv: &ConvId) -> (String, ConvKind) {
    match store.get_conversation(conv) {
        Ok(Some(row)) => {
            let sum = conv_summary(store, self_user, &row, false);
            (sum.title, sum.kind)
        }
        _ => (String::new(), ConvKind::Guest),
    }
}

/// 作者设备是否属于本机用户。
fn is_own_device(store: &Store, self_user: Option<&UserId>, device: &DeviceId) -> bool {
    store
        .get_device(device)
        .ok()
        .flatten()
        .zip(self_user)
        .is_some_and(|(r, u)| r.user == *u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{BlobLocalKind, BlobLocalRow, ConvKindRow, ConvRow, DeviceRow, UserRow};
    use lanmsg_proto::ids::MsgId;
    use lanmsg_proto::sign::UserKey;
    use lanmsg_proto::types::{DeviceCert, DeviceType, Envelope, Hlc, Signature64};

    fn put_conv(s: &Store, kind: ConvKindRow, title: Option<&str>) -> ConvId {
        let conv = ConvId([5u8; 32]);
        s.ensure_conversation(&ConvRow {
            conv,
            kind,
            peer_user: None,
            title: title.map(String::from),
            created_at: 0,
        })
        .unwrap();
        conv
    }

    /// 自设备会话：标题为空串，界面按 kind 本地化。
    #[test]
    fn self_devices_conv_reports_empty_title_and_kind() {
        let s = Store::open_in_memory().unwrap();
        let conv = put_conv(&s, ConvKindRow::SelfDevices, Some("ignored"));
        assert_eq!(
            conv_title_kind(&s, None, &conv),
            (String::new(), ConvKind::SelfDevices)
        );
    }

    #[test]
    fn guest_conv_keeps_stored_title_and_kind() {
        let s = Store::open_in_memory().unwrap();
        let conv = put_conv(&s, ConvKindRow::Guest, Some("web guest"));
        assert_eq!(
            conv_title_kind(&s, None, &conv),
            ("web guest".to_string(), ConvKind::Guest)
        );
    }

    // ---- “已全部送达”判定 ----

    /// 一个用户及其若干设备（设备已入库）。
    struct Party {
        user: UserId,
        devices: Vec<DeviceId>,
    }

    fn party(s: &Store, tag: u8, n_devices: usize, is_self: bool) -> Party {
        let key = UserKey::generate();
        let user = key.user_id();
        s.upsert_user(&UserRow {
            user,
            display_name: format!("u{tag}"),
            is_self,
            updated_at: 0,
        })
        .unwrap();
        let mut devices = Vec::new();
        for i in 0..n_devices {
            let device = DeviceId([tag * 16 + i as u8 + 1; 32]);
            s.upsert_device(&DeviceRow {
                device,
                user,
                cert: DeviceCert::issue(&key, device, format!("d{tag}{i}"), DeviceType::Desktop, 1),
                name: format!("d{tag}{i}"),
                device_type: DeviceType::Desktop,
                last_addr: None,
                last_path: None,
                last_seen: None,
                revoked: false,
            })
            .unwrap();
            devices.push(device);
        }
        Party { user, devices }
    }

    fn revoke(s: &Store, p: &Party, i: usize) {
        let mut row = s.get_device(&p.devices[i]).unwrap().unwrap();
        row.revoked = true;
        s.upsert_device(&row).unwrap();
    }

    fn new_conv(s: &Store, tag: u8, kind: ConvKindRow, peer: Option<UserId>) -> ConvId {
        let conv = ConvId([tag; 32]);
        s.ensure_conversation(&ConvRow {
            conv,
            kind,
            peer_user: peer,
            title: None,
            created_at: 0,
        })
        .unwrap();
        conv
    }

    /// `author` 在 `conv` 发一条含单个文件的消息，本机该文件位于 `path`。
    fn send_file(
        s: &Store,
        author: DeviceId,
        seq: u64,
        conv: ConvId,
        tag: u8,
        path: &str,
    ) -> BlobHash {
        let hash = BlobHash([tag; 32]);
        let env = Envelope {
            conv,
            id: MsgId { author, seq },
            conv_seq: seq,
            hlc: Hlc {
                millis: 100 + seq as i64,
                counter: 0,
            },
            body: MsgBody::Files {
                items: vec![BlobMeta {
                    hash,
                    size: 1,
                    name: "f.bin".into(),
                    mime: "application/octet-stream".into(),
                    media: None,
                    thumb: None,
                }],
            },
            enc: None,
            sig: Signature64([0u8; 64]),
        };
        s.finish_preparing(
            &author,
            seq,
            &env,
            &[],
            &[BlobLocalRow {
                hash,
                path: path.into(),
                kind: BlobLocalKind::Source,
                size: 1,
                mtime: 0,
            }],
            1,
        )
        .unwrap();
        hash
    }

    fn delivered(s: &Store, me: &Party, under: &str) -> Vec<String> {
        delivered_paths(s, Some(&me.user), &me.devices[0], under).unwrap()
    }

    #[test]
    fn dm_delivered_once_every_peer_device_holds() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 1, true);
        let peer = party(&s, 2, 2, false);
        let conv = new_conv(&s, 9, ConvKindRow::Dm, Some(peer.user));
        let h = send_file(&s, me.devices[0], 1, conv, 7, "/out/a.bin");
        assert!(delivered(&s, &me, "/out").is_empty());
        s.add_holder(&h, &peer.devices[0], 1).unwrap();
        assert!(
            delivered(&s, &me, "/out").is_empty(),
            "对方还有一台设备没下载"
        );
        s.add_holder(&h, &peer.devices[1], 1).unwrap();
        assert_eq!(delivered(&s, &me, "/out"), vec!["/out/a.bin".to_string()]);
    }

    #[test]
    fn revoked_device_does_not_block_delivery() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 1, true);
        let peer = party(&s, 2, 2, false);
        let conv = new_conv(&s, 9, ConvKindRow::Dm, Some(peer.user));
        let h = send_file(&s, me.devices[0], 1, conv, 7, "/out/a.bin");
        s.add_holder(&h, &peer.devices[0], 1).unwrap();
        assert!(delivered(&s, &me, "/out").is_empty());
        revoke(&s, &peer, 1);
        assert_eq!(delivered(&s, &me, "/out").len(), 1);
    }

    /// 从未上线、从未下载的设备（未吊销）会一直挡住——有意的保守取舍。
    #[test]
    fn never_seen_device_keeps_blocking_until_revoked() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 1, true);
        let peer = party(&s, 2, 2, false);
        let conv = new_conv(&s, 9, ConvKindRow::Dm, Some(peer.user));
        let h = send_file(&s, me.devices[0], 1, conv, 7, "/out/a.bin");
        s.add_holder(&h, &peer.devices[0], 1).unwrap();
        for _ in 0..2 {
            assert!(delivered(&s, &me, "/out").is_empty());
        }
    }

    #[test]
    fn self_devices_conv_requires_all_own_other_devices() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 3, true);
        let conv = new_conv(&s, 9, ConvKindRow::SelfDevices, None);
        let h = send_file(&s, me.devices[0], 1, conv, 7, "/out/a.bin");
        s.add_holder(&h, &me.devices[0], 1).unwrap();
        assert!(delivered(&s, &me, "/out").is_empty());
        s.add_holder(&h, &me.devices[1], 1).unwrap();
        assert!(delivered(&s, &me, "/out").is_empty(), "第三台设备还没下载");
        s.add_holder(&h, &me.devices[2], 1).unwrap();
        assert_eq!(delivered(&s, &me, "/out").len(), 1);
    }

    #[test]
    fn self_devices_conv_with_only_this_device_is_not_delivered() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 1, true);
        let conv = new_conv(&s, 9, ConvKindRow::SelfDevices, None);
        send_file(&s, me.devices[0], 1, conv, 7, "/out/a.bin");
        assert!(delivered(&s, &me, "/out").is_empty());
    }

    #[test]
    fn paths_outside_prefix_are_excluded() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 1, true);
        let peer = party(&s, 2, 1, false);
        let conv = new_conv(&s, 9, ConvKindRow::Dm, Some(peer.user));
        for (seq, tag, path) in [
            (1, 7, "/out/a.bin"),
            (2, 8, "/outside/b.bin"),
            (3, 9, "/elsewhere/c.bin"),
        ] {
            let h = send_file(&s, me.devices[0], seq, conv, tag, path);
            s.add_holder(&h, &peer.devices[0], 1).unwrap();
        }
        assert_eq!(delivered(&s, &me, "/out"), vec!["/out/a.bin".to_string()]);
    }

    #[test]
    fn blob_only_referenced_by_peer_message_is_not_delivered() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 1, true);
        let peer = party(&s, 2, 1, false);
        let conv = new_conv(&s, 9, ConvKindRow::Dm, Some(peer.user));
        let h = send_file(&s, peer.devices[0], 1, conv, 7, "/out/a.bin");
        s.add_holder(&h, &peer.devices[0], 1).unwrap();
        assert!(delivered(&s, &me, "/out").is_empty());
    }

    /// 同一哈希也被对端自己的消息引用（本地只有一个副本）：不算送达。
    #[test]
    fn blob_also_referenced_by_peer_message_is_not_delivered() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 1, true);
        let peer = party(&s, 2, 1, false);
        let conv = new_conv(&s, 9, ConvKindRow::Dm, Some(peer.user));
        let h = send_file(&s, me.devices[0], 1, conv, 7, "/out/a.bin");
        s.add_holder(&h, &peer.devices[0], 1).unwrap();
        assert_eq!(delivered(&s, &me, "/out").len(), 1);
        // 对端后来在同一会话发了同一个文件（摄取时其设备也会被记为持有者）。
        send_file(&s, peer.devices[0], 1, conv, 7, "/out/a.bin");
        assert!(delivered(&s, &me, "/out").is_empty());
    }

    /// 同一哈希发给两个会话，一个满足、一个不满足：不送达。
    #[test]
    fn same_blob_in_two_convs_needs_both_satisfied() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 1, true);
        let p = party(&s, 2, 1, false);
        let q = party(&s, 3, 1, false);
        let conv_p = new_conv(&s, 9, ConvKindRow::Dm, Some(p.user));
        let conv_q = new_conv(&s, 10, ConvKindRow::Dm, Some(q.user));
        let h = send_file(&s, me.devices[0], 1, conv_p, 7, "/out/a.bin");
        send_file(&s, me.devices[0], 2, conv_q, 7, "/out/a.bin");
        // 只有 P 的设备持有：Q 会话不满足，且 P 持有不能替 Q 作数。
        s.add_holder(&h, &p.devices[0], 1).unwrap();
        assert!(delivered(&s, &me, "/out").is_empty());
        s.add_holder(&h, &q.devices[0], 1).unwrap();
        assert_eq!(delivered(&s, &me, "/out").len(), 1);
    }

    #[test]
    fn unsupported_conv_kinds_are_not_delivered() {
        let s = Store::open_in_memory().unwrap();
        let me = party(&s, 1, 1, true);
        let conv = new_conv(&s, 9, ConvKindRow::Guest, None);
        send_file(&s, me.devices[0], 1, conv, 7, "/out/a.bin");
        assert!(delivered(&s, &me, "/out").is_empty());
    }
}
