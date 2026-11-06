// P3 Task 15：文件视图查询、消息文字搜索、文件转发。
// 注释中文，标识符英文。

use std::collections::HashMap;

use lanmsg_api::types::{FileFilter, FileView, MessageView};
use lanmsg_proto::ids::{BlobHash, ConvId, DeviceId};
use lanmsg_proto::types::{BlobMeta, MsgBody};

use crate::error::CoreError;
use crate::service::views::{conv_summary, file_item_view, message_view};
use crate::service::Inner;
use crate::store::file_index::kind_str;
use crate::store::FileQuery;

/// 文件页单次查询条数上限。
const MAX_FILE_LIMIT: u32 = 500;
/// 带状态筛选时最多追加查询的轮数。
const MAX_FILTER_ROUNDS: u32 = 5;

/// 文件视图查询：按筛选条件分页，状态筛选在内存中做（状态是动态计算的，不入库）。
pub(crate) fn query_files(svc: &Inner, filter: &FileFilter) -> Result<Vec<FileView>, CoreError> {
    let conv = filter.conv.as_deref().map(parse_conv).transpose()?;
    let author = filter
        .source_device
        .as_deref()
        .map(|s| {
            DeviceId::from_hex(s)
                .map_err(|_| CoreError::InvalidArgument(format!("非法设备 ID: {s}")))
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

    let peers = svc.conns.peers();
    let mut titles: HashMap<ConvId, String> = HashMap::new();
    let mut out: Vec<FileView> = Vec::new();
    let mut before = base.before_millis;
    // 无状态筛选时一轮即可；有状态筛选时不足 limit 就以末行时间继续查。
    let rounds = if filter.state.is_some() {
        MAX_FILTER_ROUNDS
    } else {
        1
    };
    for _ in 0..rounds {
        let rows = svc.store.query_file_rows(&FileQuery {
            before_millis: before,
            ..base.clone()
        })?;
        let Some(last) = rows.last() else { break };
        before = Some(last.hlc_millis);
        let full = rows.len() as u32 >= limit;
        for row in &rows {
            let Some(stored) = svc.store.get_message(&row.msg)? else {
                continue;
            };
            let blobs = svc.store.message_blobs_of(&row.msg)?;
            let Some(mb) = blobs.into_iter().find(|b| b.idx == row.item_idx) else {
                continue;
            };
            let author_display = author_display_name(svc, &row.msg.author);
            let item = file_item_view(
                svc.store.as_ref(),
                &peers,
                &row.msg.author,
                &mb.meta,
                &author_display,
            );
            if filter.state.as_ref().is_some_and(|s| *s != item.state) {
                continue;
            }
            let title = match titles.get(&row.conv) {
                Some(t) => t.clone(),
                None => {
                    let t = conv_title(svc, &row.conv);
                    titles.insert(row.conv, t.clone());
                    t
                }
            };
            out.push(FileView {
                item,
                conv: row.conv.to_hex(),
                conv_title: title,
                msg: stored.env.id.to_string(),
                hlc_millis: row.hlc_millis,
            });
        }
        if out.len() as u32 >= limit || !full {
            break;
        }
    }
    Ok(out)
}

/// 消息文字搜索：先查索引拿 id，再逐条还原视图（取不到的跳过）。
pub(crate) fn search_messages(
    svc: &Inner,
    query: &str,
    conv: Option<&ConvId>,
    limit: u32,
) -> Result<Vec<MessageView>, CoreError> {
    let ids = svc.store.search_messages(query, conv, limit)?;
    let self_device = svc.identity.read().unwrap().device_id();
    let peers = svc.conns.peers();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(m) = svc.store.get_message(&id)? {
            out.push(message_view(svc.store.as_ref(), &self_device, &peers, &m));
        }
    }
    Ok(out)
}

/// 把已有文件转发到会话：复用库中已有的 BlobMeta（名字、大小、mime、媒体信息、
/// 缩略图），不重新哈希，也不要求本机持有原件；接收方按 P2 机制向持有者拉取。
pub(crate) async fn forward_files(
    svc: &Inner,
    conv: &ConvId,
    hashes: &[BlobHash],
) -> Result<lanmsg_proto::ids::MsgId, CoreError> {
    if hashes.is_empty() {
        return Err(CoreError::InvalidArgument("没有要转发的文件".into()));
    }
    if svc.store.get_conversation(conv)?.is_none() {
        return Err(CoreError::NotFound(format!("会话 {}", conv.to_hex())));
    }
    let mut items: Vec<BlobMeta> = Vec::with_capacity(hashes.len());
    for h in hashes {
        let Some(row) = svc.store.message_blobs_by_hash(h)?.into_iter().next() else {
            return Err(CoreError::NotFound(format!("文件 {}", h.to_hex())));
        };
        items.push(row.meta);
    }
    // 锁已在上面各步骤中释放，这里才 await。
    svc.msg.send(conv, MsgBody::Files { items }).await
}

fn parse_conv(s: &str) -> Result<ConvId, CoreError> {
    ConvId::from_hex(s).map_err(|_| CoreError::InvalidArgument(format!("非法会话 ID: {s}")))
}

/// 作者显示名：users 表显示名优先，无则设备名（与 message_view 一致）。
fn author_display_name(svc: &Inner, author: &DeviceId) -> String {
    let row = svc.store.get_device(author).ok().flatten();
    match row {
        Some(r) => svc
            .store
            .get_user(&r.user)
            .ok()
            .flatten()
            .map(|u| u.display_name)
            .unwrap_or(r.name),
        None => author.short(),
    }
}

/// 会话标题（规则同会话列表）；会话不存在时回退为空串。
fn conv_title(svc: &Inner, conv: &ConvId) -> String {
    match svc.store.get_conversation(conv) {
        Ok(Some(row)) => {
            let self_user = svc.state.read().unwrap().self_user;
            conv_summary(svc.store.as_ref(), self_user.as_ref(), &row, false).title
        }
        _ => String::new(),
    }
}
