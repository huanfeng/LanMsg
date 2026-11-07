// P3 Task 15：文件视图查询、消息文字搜索、文件转发的集成测试。
// 全部使用 127.0.0.1 + 内存发现（MemoryBus）。
// 注释中文，标识符英文。

mod common;

use std::collections::HashSet;
use std::time::Duration;

use common::{befriend, node, TestNode};
use lanmsg_api::types::{
    BlobState, ErrorCode, Event, FileFilter, FileKindFilter, FileView, LastMessageKind,
    MessageBodyView, Request, Response,
};
use lanmsg_core::discovery::MemoryBus;

fn filter(kind: FileKindFilter, limit: u32) -> FileFilter {
    FileFilter {
        kind,
        conv: None,
        source_device: None,
        state: None,
        name_query: None,
        before_millis: None,
        limit,
    }
}

async fn query(node: &TestNode, f: FileFilter) -> Vec<FileView> {
    match node.call(Request::QueryFiles { filter: f }).await {
        Response::Files { value } => value,
        other => panic!("期望 Files，实际 {other:?}"),
    }
}

async fn open_dm(from: &TestNode, to: &TestNode) -> String {
    match from.call(Request::OpenDm { user: to.user_id() }).await {
        Response::ConvId { value } => value,
        other => panic!("期望 ConvId，实际 {other:?}"),
    }
}

fn write(dir: &std::path::Path, name: &str, content: &[u8]) -> String {
    let p = dir.join(name);
    std::fs::write(&p, content).expect("写测试文件失败");
    p.to_string_lossy().into_owned()
}

/// 轮询直到条件成立（最多 15 秒）。
async fn eventually<F, Fut>(f: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    common::wait_until(f, 15).await;
}

#[tokio::test]
async fn query_files_filters_by_kind_and_paginates() {
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    a.init_first("A").await;
    let mut b = node(&bus).await;
    b.init_first("B").await;
    befriend(&mut a, &mut b).await;
    let conv = open_dm(&a, &b).await;

    let src = tempfile::tempdir().unwrap();
    for i in 0..120 {
        // 内容不同，哈希各异；扩展名决定类型。
        let p = write(
            src.path(),
            &format!("pic{i:03}.png"),
            format!("img{i}").as_bytes(),
        );
        a.call(Request::SendFiles {
            conv: conv.clone(),
            paths: vec![p],
        })
        .await;
    }
    let doc = write(src.path(), "报告.pdf", b"doc");
    a.call(Request::SendFiles {
        conv: conv.clone(),
        paths: vec![doc],
    })
    .await;

    eventually(|| async { query(&b, filter(FileKindFilter::All, 500)).await.len() == 121 }).await;

    // 类型筛选：图片 120、文档 1。
    assert_eq!(
        query(&b, filter(FileKindFilter::Document, 100)).await.len(),
        1
    );
    // 分页：首页至少 100 条（同一毫秒的行会被一并带出，避免游标跳行），
    // 以末行时间为游标取剩余，合计恰好 120 条且不重复。
    let first = query(&b, filter(FileKindFilter::Image, 100)).await;
    assert!(first.len() >= 100);
    let mut f2 = filter(FileKindFilter::Image, 100);
    f2.before_millis = first.last().map(|v| v.hlc_millis);
    let rest = query(&b, f2).await;
    let all: HashSet<(String, String)> = first
        .iter()
        .chain(rest.iter())
        .map(|v| (v.msg.clone(), v.item.hash.clone()))
        .collect();
    assert_eq!(first.len() + rest.len(), 120);
    assert_eq!(all.len(), 120);
    // 新的在前。
    assert!(first.windows(2).all(|w| w[0].hlc_millis >= w[1].hlc_millis));
    // 会话标题是对端用户名。
    assert_eq!(first[0].conv_title, "A");

    // 名字：两个字（LIKE）与长词（FTS）。
    let mut by_short = filter(FileKindFilter::All, 100);
    by_short.name_query = Some("报告".into());
    assert_eq!(query(&b, by_short).await.len(), 1);
    let mut by_long = filter(FileKindFilter::All, 200);
    by_long.name_query = Some("pic07".into());
    assert_eq!(query(&b, by_long).await.len(), 10);

    // 来源设备与会话筛选。
    let mut by_dev = filter(FileKindFilter::All, 500);
    by_dev.source_device = Some(a.device_id().await);
    assert_eq!(query(&b, by_dev).await.len(), 121);
    let mut by_other = filter(FileKindFilter::All, 500);
    by_other.source_device = Some(b.device_id().await);
    assert!(query(&b, by_other).await.is_empty());
    let mut by_conv = filter(FileKindFilter::All, 500);
    by_conv.conv = Some(conv);
    assert_eq!(query(&b, by_conv).await.len(), 121);

    // 非法 ID → InvalidArgument。
    let mut bad = filter(FileKindFilter::All, 10);
    bad.conv = Some("zz".into());
    let err = b
        .try_call(Request::QueryFiles { filter: bad })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

#[tokio::test]
async fn query_files_filters_by_state_and_excludes_deleted() {
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    a.init_first("A").await;
    let mut b = node(&bus).await;
    b.init_first("B").await;
    befriend(&mut a, &mut b).await;
    let conv = open_dm(&a, &b).await;
    let src = tempfile::tempdir().unwrap();
    // A 发出的文件对 A 自己是 Local。
    for name in ["one.bin", "two.bin"] {
        a.call(Request::SendFiles {
            conv: conv.clone(),
            paths: vec![write(src.path(), name, name.as_bytes())],
        })
        .await;
    }
    eventually(|| async { query(&a, filter(FileKindFilter::All, 50)).await.len() == 2 }).await;
    let mut local = filter(FileKindFilter::All, 50);
    local.state = Some(BlobState::Local);
    let rows = query(&a, local.clone()).await;
    assert_eq!(rows.len(), 2);
    let mut remote = filter(FileKindFilter::All, 50);
    remote.state = Some(BlobState::SourceOffline);
    assert!(query(&a, remote).await.is_empty());

    // 本地删除后不再出现。
    a.call(Request::DeleteLocal {
        msgs: vec![rows[0].msg.clone()],
        also_my_devices: false,
    })
    .await;
    assert_eq!(query(&a, local).await.len(), 1);
}

#[tokio::test]
async fn search_messages_over_api() {
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    a.init_first("A").await;
    let mut b = node(&bus).await;
    b.init_first("B").await;
    befriend(&mut a, &mut b).await;
    let conv = open_dm(&a, &b).await;
    a.call(Request::SendText {
        conv: conv.clone(),
        text: "周五下午开会".into(),
    })
    .await;
    a.call(Request::SendText {
        conv: conv.clone(),
        text: "无关内容".into(),
    })
    .await;

    let search = |q: &'static str, conv: Option<String>| {
        let b = &b;
        async move {
            match b
                .call(Request::SearchMessages {
                    query: q.into(),
                    conv,
                    limit: 10,
                })
                .await
            {
                Response::Messages { value } => value,
                other => panic!("期望 Messages，实际 {other:?}"),
            }
        }
    };
    eventually(|| async { search("开会", None).await.len() == 1 }).await;
    let hit = search("开会", None).await;
    assert!(matches!(&hit[0].body, MessageBodyView::Text { text } if text == "周五下午开会"));
    assert_eq!(hit[0].conv, conv);
    assert_eq!(search("开会", Some(conv.clone())).await.len(), 1);
    assert!(search("开会", Some("00".repeat(32))).await.is_empty());
    assert!(search("", None).await.is_empty());
    // 查询词里的 SQL / FTS 语法字符只当字面量。
    assert!(search("\" OR 1=1 --", None).await.is_empty());
    // 非法会话 ID。
    let err = b
        .try_call(Request::SearchMessages {
            query: "开会".into(),
            conv: Some("zz".into()),
            limit: 10,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

#[tokio::test]
async fn forward_files_reuses_blob_meta_and_receiver_can_download() {
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    a.init_first("A").await;
    let mut b = node(&bus).await;
    b.init_first("B").await;
    let mut c = node(&bus).await;
    c.init_first("C").await;
    befriend(&mut a, &mut b).await;
    befriend(&mut b, &mut c).await;

    let dl_b = tempfile::tempdir().unwrap();
    let dl_c = tempfile::tempdir().unwrap();
    for (n, d) in [(&b, dl_b.path()), (&c, dl_c.path())] {
        let Response::Settings { mut value } = n.call(Request::GetSettings).await else {
            panic!("期望 Settings")
        };
        value.download_dir = d.to_string_lossy().into_owned();
        // 关闭自动下载：由测试显式触发。
        value.auto_file_max = 0;
        value.auto_image_max = 0;
        n.call(Request::SetSettings { settings: value }).await;
    }

    let src = tempfile::tempdir().unwrap();
    let content = vec![0x5Au8; 4096];
    let path = write(src.path(), "资料.bin", &content);
    let conv_ab = open_dm(&a, &b).await;
    a.call(Request::SendFiles {
        conv: conv_ab,
        paths: vec![path],
    })
    .await;
    eventually(|| async { query(&b, filter(FileKindFilter::All, 10)).await.len() == 1 }).await;
    let original = query(&b, filter(FileKindFilter::All, 10))
        .await
        .remove(0)
        .item;
    // B 先下载（成为持有者），A 与 C 不是联系人，C 只能向 B 拉取。
    b.call(Request::DownloadBlob {
        hash: original.hash.clone(),
    })
    .await;
    let h = original.hash.clone();
    b.wait_event(
        |e| matches!(e, Event::TransferFinished { hash, ok: true, .. } if *hash == h),
        15,
    )
    .await;

    let conv_bc = open_dm(&b, &c).await;
    let msg = match b
        .call(Request::ForwardFiles {
            conv: conv_bc,
            hashes: vec![original.hash.clone()],
        })
        .await
    {
        Response::MsgId { value } => value,
        other => panic!("期望 MsgId，实际 {other:?}"),
    };
    assert!(!msg.is_empty());

    eventually(|| async { query(&c, filter(FileKindFilter::All, 10)).await.len() == 1 }).await;
    let got = query(&c, filter(FileKindFilter::All, 10))
        .await
        .remove(0)
        .item;
    assert_eq!(got.hash, original.hash);
    assert_eq!(got.name, original.name);
    assert_eq!(got.size, original.size);

    c.call(Request::DownloadBlob {
        hash: got.hash.clone(),
    })
    .await;
    let h = got.hash.clone();
    c.wait_event(
        |e| matches!(e, Event::TransferFinished { hash, ok: true, .. } if *hash == h),
        15,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let done = query(&c, filter(FileKindFilter::All, 10))
        .await
        .remove(0)
        .item;
    assert_eq!(done.state, BlobState::Local);
    let local = done.local_path.expect("下载完成应有本地路径");
    assert_eq!(std::fs::read(local).unwrap(), content);
}

#[tokio::test]
async fn forward_unknown_hash_is_not_found() {
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    a.init_first("A").await;
    let mut b = node(&bus).await;
    b.init_first("B").await;
    befriend(&mut a, &mut b).await;
    let conv = open_dm(&a, &b).await;

    let err = a
        .try_call(Request::ForwardFiles {
            conv: conv.clone(),
            hashes: vec!["ab".repeat(32)],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    let err = a
        .try_call(Request::ForwardFiles {
            conv: conv.clone(),
            hashes: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    let err = a
        .try_call(Request::ForwardFiles {
            conv,
            hashes: vec!["not-hex".into()],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    // 会话不存在。
    let err = a
        .try_call(Request::ForwardFiles {
            conv: "11".repeat(32),
            hashes: vec!["ab".repeat(32)],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn forward_deleted_or_recalled_file_is_not_found() {
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    a.init_first("A").await;
    let mut b = node(&bus).await;
    b.init_first("B").await;
    befriend(&mut a, &mut b).await;
    let conv = open_dm(&a, &b).await;
    let src = tempfile::tempdir().unwrap();
    for name in ["del.bin", "rec.bin"] {
        a.call(Request::SendFiles {
            conv: conv.clone(),
            paths: vec![write(src.path(), name, name.as_bytes())],
        })
        .await;
    }
    eventually(|| async { query(&a, filter(FileKindFilter::All, 50)).await.len() == 2 }).await;
    let rows = query(&a, filter(FileKindFilter::All, 50)).await;
    let del = rows.iter().find(|r| r.item.name == "del.bin").unwrap();
    let rec = rows.iter().find(|r| r.item.name == "rec.bin").unwrap();

    a.call(Request::DeleteLocal {
        msgs: vec![del.msg.clone()],
        also_my_devices: false,
    })
    .await;
    a.call(Request::Recall {
        msg: rec.msg.clone(),
    })
    .await;
    for hash in [&del.item.hash, &rec.item.hash] {
        let err = a
            .try_call(Request::ForwardFiles {
                conv: conv.clone(),
                hashes: vec![hash.clone()],
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }
}

/// 回归：文件准备完成后（信封才入库）、对端收到文件、撤回后，
/// 两端都必须收到带最新 last_kind 的 ConversationUpdated，会话列表预览才会刷新。
#[tokio::test]
async fn conversation_summary_refreshes_after_file_and_recall() {
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    a.init_first("A").await;
    let mut b = node(&bus).await;
    b.init_first("B").await;
    befriend(&mut a, &mut b).await;
    let conv = open_dm(&a, &b).await;
    let src = tempfile::tempdir().unwrap();
    a.call(Request::SendFiles {
        conv: conv.clone(),
        paths: vec![write(src.path(), "doc.bin", b"hello")],
    })
    .await;

    let is_file = |e: &Event| {
        matches!(e, Event::ConversationUpdated { summary }
            if summary.conv == conv
                && matches!(&summary.last_kind, Some(LastMessageKind::File { name, .. }) if name == "doc.bin"))
    };
    b.wait_event(is_file, 10).await;

    let rows = query(&a, filter(FileKindFilter::All, 50)).await;
    a.call(Request::Recall {
        msg: rows[0].msg.clone(),
    })
    .await;
    let is_recalled = |e: &Event| {
        matches!(e, Event::ConversationUpdated { summary }
            if summary.conv == conv && summary.last_kind == Some(LastMessageKind::Recalled))
    };
    a.wait_event(is_recalled, 10).await;
    b.wait_event(is_recalled, 10).await;
}

/// 回归：单节点（自设备会话，无对端消息干扰）发文件，准备完成后必须发带
/// last_kind 的 ConversationUpdated——此前只发 MessageUpdated，会话列表预览不刷新。
#[tokio::test]
async fn conversation_summary_refreshes_when_file_preparation_finishes() {
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    a.init_first("A").await;
    let conv = match a.call(Request::ListConversations).await {
        Response::Conversations { value } => value[0].conv.clone(),
        other => panic!("期望 Conversations，实际 {other:?}"),
    };
    let src = tempfile::tempdir().unwrap();
    a.call(Request::SendFiles {
        conv: conv.clone(),
        paths: vec![write(src.path(), "solo.bin", b"hello")],
    })
    .await;
    a.wait_event(
        |e| {
            matches!(e, Event::ConversationUpdated { summary }
                if summary.conv == conv
                    && matches!(&summary.last_kind, Some(LastMessageKind::File { name, .. }) if name == "solo.bin"))
        },
        10,
    )
    .await;
}
