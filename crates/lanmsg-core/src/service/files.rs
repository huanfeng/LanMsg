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
use crate::store::{FileQuery, Store};

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
    use crate::store::{ConvKindRow, ConvRow};

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
}
