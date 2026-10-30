// P2 Task 11：Blob 发送集成测试。
// 覆盖：send_files 的 Preparing → 完成流转、数据服务端 GetMeta/GetRange 的
// 权限与源校验、文件夹遍历上限、重启恢复 preparing、BlobQuery 持有者上报。
// 注释中文，标识符英文。

mod common;

use common::{befriend, link, node, node_offline, node_offline_with, wait_until};
use lanmsg_api::types::{BlobState, Event, MessageBodyView, Request, Response};
use lanmsg_core::blob::walk_folder;
use lanmsg_core::platform::PlatformHooks;
use lanmsg_proto::data::{read_data_frame, write_data_frame, DataFrame};
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{BlobHash, ConvId, DeviceId, UserId};
use lanmsg_proto::types::BlobQuery;

/// 从事件中取出文件消息的第一项（无则 None）。
fn first_file_item(e: &Event) -> Option<lanmsg_api::types::FileItemView> {
    let msg = match e {
        Event::MessageAdded { msg } | Event::MessageUpdated { msg } => msg,
        _ => return None,
    };
    match &msg.body {
        MessageBodyView::Files { items } => items.first().cloned(),
        MessageBodyView::Folder { items, .. } => items.first().cloned(),
        _ => None,
    }
}

/// 等待本端收到首个文件项满足条件的 MessageAdded/MessageUpdated，返回该项。
async fn wait_file_item(
    node: &mut common::TestNode,
    f: impl Fn(&lanmsg_api::types::FileItemView) -> bool,
    secs: u64,
) -> lanmsg_api::types::FileItemView {
    loop {
        let ev = node
            .wait_event(
                |e| matches!(e, Event::MessageAdded { .. } | Event::MessageUpdated { .. }),
                secs,
            )
            .await;
        if let Some(item) = first_file_item(&ev) {
            if f(&item) {
                return item;
            }
        }
    }
}

/// 写一个 size 字节的内容固定文件，返回路径。
fn make_file(dir: &std::path::Path, name: &str, size: usize) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, vec![0xA5u8; size]).expect("写测试文件失败");
    path
}

/// A、X 结为联系人，返回 (a, x)。
async fn contact_pair() -> (common::TestNode, common::TestNode) {
    let bus = lanmsg_core::discovery::MemoryBus::new();
    let mut a = node(&bus).await;
    a.init_first("A").await;
    let mut x = node(&bus).await;
    x.init_first("X").await;
    befriend(&mut a, &mut x).await;
    (a, x)
}

/// A 向 X 发送一个文件并等双方完成流转，返回 (conv, hash 十六进制)。
async fn send_and_wait(
    a: &mut common::TestNode,
    x: &mut common::TestNode,
    path: &std::path::Path,
    size: usize,
) -> (String, String) {
    let conv = match a.call(Request::OpenDm { user: x.user_id() }).await {
        Response::ConvId { value } => value,
        other => panic!("期望 ConvId，实际 {other:?}"),
    };
    a.call(Request::SendFiles {
        conv: conv.clone(),
        paths: vec![path.to_string_lossy().into_owned()],
    })
    .await;

    // A：先看到 Preparing，再看到 hash 非空。
    let preparing = wait_file_item(a, |it| it.state == BlobState::Preparing, 5).await;
    assert!(preparing.hash.is_empty(), "Preparing 项 hash 应为空");
    let _ = wait_file_item(a, |it| !it.hash.is_empty(), 5).await;
    // X：收到最终消息，大小一致、state Remote。
    let item = wait_file_item(
        x,
        |it| !it.hash.is_empty() && it.size == size as u64 && it.state == BlobState::Remote,
        5,
    )
    .await;
    (conv, item.hash)
}

#[tokio::test]
async fn send_files_goes_preparing_then_sent() {
    let (mut a, mut x) = contact_pair().await;
    let dir = tempfile::tempdir().unwrap();
    let path = make_file(dir.path(), "报告.bin", 1024 * 1024);
    let (conv, hash) = send_and_wait(&mut a, &mut x, &path, 1024 * 1024).await;
    assert!(!conv.is_empty());
    assert!(!hash.is_empty());
}

#[tokio::test]
async fn serving_modified_source_returns_not_found() {
    let (mut a, mut x) = contact_pair().await;
    let dir = tempfile::tempdir().unwrap();
    let path = make_file(dir.path(), "原文件.bin", 64 * 1024);
    let (_, hash_hex) = send_and_wait(&mut a, &mut x, &path, 64 * 1024).await;

    // 改写源文件（长度变化确保 size/mtime 校验必然触发重哈希）。
    std::fs::write(&path, vec![0x5Au8; 100]).unwrap();

    // X 打开数据连接向 A 请求 GetMeta → NotFound。
    let a_dev = DeviceId::from_hex(&a.device_id().await).unwrap();
    let mut stream = x.svc.open_data(&a_dev).await.expect("open_data 失败");
    let hash = BlobHash::from_hex(&hash_hex).unwrap();
    write_data_frame(&mut stream, &DataFrame::GetMeta { hash })
        .await
        .unwrap();
    let resp = read_data_frame(&mut stream).await.unwrap();
    assert!(
        matches!(resp, DataFrame::NotFound { .. }),
        "改写后的源文件应返回 NotFound，实际 {resp:?}"
    );
}

#[tokio::test]
async fn stranger_cannot_fetch_unshared_blob() {
    let (mut a, mut x) = contact_pair().await;
    // 陌生人 Y：仅与 A 互连，不是联系人。
    let bus = lanmsg_core::discovery::MemoryBus::new();
    let y = node(&bus).await;
    y.init_first("Y").await;
    link(&a, &y).await;

    let dir = tempfile::tempdir().unwrap();
    let path = make_file(dir.path(), "私密.bin", 32 * 1024);
    let (_, hash_hex) = send_and_wait(&mut a, &mut x, &path, 32 * 1024).await;
    let hash = BlobHash::from_hex(&hash_hex).unwrap();

    // Y 用 GetRange 向 A 索要 → NotFound（不区分无权与没有）。
    let a_dev = DeviceId::from_hex(&a.device_id().await).unwrap();
    let mut stream = y.svc.open_data(&a_dev).await.expect("open_data 失败");
    write_data_frame(
        &mut stream,
        &DataFrame::GetRange {
            hash,
            offset: 0,
            len: 16,
        },
    )
    .await
    .unwrap();
    let resp = read_data_frame(&mut stream).await.unwrap();
    assert!(
        matches!(resp, DataFrame::NotFound { .. }),
        "陌生人取未共享 Blob 应返回 NotFound，实际 {resp:?}"
    );
}

#[test]
fn send_folder_rejects_too_many_entries() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..3 {
        std::fs::write(dir.path().join(format!("文件{i}.txt")), b"x").unwrap();
    }
    // 上限 2 → InvalidArgument；上限 10 → 正常列出 3 项。
    match walk_folder(dir.path(), 2) {
        Err(lanmsg_core::error::CoreError::InvalidArgument(m)) => {
            assert!(m.contains("文件夹文件数超过"), "实际错误: {m}");
        }
        other => panic!("期望 InvalidArgument，实际 {other:?}"),
    }
    let ok = walk_folder(dir.path(), 10).unwrap();
    assert_eq!(ok.len(), 3);
}

#[tokio::test]
async fn preparing_resumes_after_restart() {
    let a = node_offline().await;
    a.init_first("A").await;
    // SelfDevices 会话（无对端，验证本地恢复链路）。
    let self_user = a.user_id();
    let conv = ConvId::self_devices(&UserId::from_hex(&self_user).unwrap()).to_hex();

    let dir = tempfile::tempdir().unwrap();
    let path = make_file(dir.path(), "大文件.bin", 8 * 1024 * 1024);

    a.call(Request::SendFiles {
        conv: conv.clone(),
        paths: vec![path.to_string_lossy().into_owned()],
    })
    .await;
    // 立刻关闭：后台准备任务被中止，preparing 行保留。
    let common::TestNode {
        svc, dir: node_dir, ..
    } = a;
    svc.shutdown().await;

    // 重启后 5 秒内收到 MessageUpdated（state Sending/Sent、hash 非空）。
    let mut a2 = node_offline_with(node_dir, PlatformHooks::default()).await;
    let item = wait_file_item(
        &mut a2,
        |it| !it.hash.is_empty() && it.size == 8 * 1024 * 1024,
        5,
    )
    .await;
    assert!(!item.hash.is_empty());
    assert_ne!(item.state, BlobState::Preparing);
}

#[tokio::test]
async fn blob_query_reports_holders() {
    let (mut a, mut x) = contact_pair().await;
    let dir = tempfile::tempdir().unwrap();
    let path = make_file(dir.path(), "持有者.bin", 16 * 1024);
    let (_, hash_hex) = send_and_wait(&mut a, &mut x, &path, 16 * 1024).await;
    let hash = BlobHash::from_hex(&hash_hex).unwrap();
    let a_dev = DeviceId::from_hex(&a.device_id().await).unwrap();

    // X 发 BlobQuery([hash]) 给 A → A 回 BlobHave → X 记录 A 为持有者。
    x.svc
        .send_frame_for_test(&a_dev, Frame::BlobQuery(BlobQuery { hashes: vec![hash] }))
        .await
        .expect("发送 BlobQuery 失败");
    wait_until(
        || async {
            x.svc
                .store_for_test()
                .holders(&hash)
                .unwrap_or_default()
                .contains(&a_dev)
        },
        5,
    )
    .await;
}
