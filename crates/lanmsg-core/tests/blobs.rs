// P2 Task 11：Blob 发送集成测试。
// 覆盖：send_files 的 Preparing → 完成流转、数据服务端 GetMeta/GetRange 的
// 权限与源校验、文件夹遍历上限、重启恢复 preparing、BlobQuery 持有者上报。
// 注释中文，标识符英文。

mod common;

use std::path::PathBuf;

use common::{befriend, link, node, node_offline, node_offline_with, pair, wait_until};
use lanmsg_api::types::{BlobState, Event, InitMode, MessageBodyView, Request, Response};
use lanmsg_core::blob::transfer::{sanitize_component, unique_path};
use lanmsg_core::blob::walk_folder;
use lanmsg_core::platform::PlatformHooks;
use lanmsg_proto::consts::BLOB_SEGMENT_LEN;
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

// ===== P2 Task 12：传输引擎 =====

/// 设置节点的下载目录。
async fn set_download_dir(node: &common::TestNode, dir: &std::path::Path) {
    let resp = node.call(Request::GetSettings).await;
    let Response::Settings { mut value } = resp else {
        panic!("期望 Settings，实际 {resp:?}")
    };
    value.download_dir = dir.to_string_lossy().into_owned();
    node.call(Request::SetSettings { settings: value }).await;
}

/// 等待 hash 的 TransferFinished，返回 (ok, error)。
async fn wait_finished(
    node: &mut common::TestNode,
    hash_hex: &str,
    secs: u64,
) -> (bool, Option<String>) {
    let ev = node
        .wait_event(
            |e| matches!(e, Event::TransferFinished { hash, .. } if hash == hash_hex),
            secs,
        )
        .await;
    match ev {
        Event::TransferFinished { ok, error, .. } => (ok, error),
        other => panic!("期望 TransferFinished，实际 {other:?}"),
    }
}

/// 轮询直到传输行有 n 个完成段，返回该行。
async fn wait_done_segments(
    store: &std::sync::Arc<lanmsg_core::store::Store>,
    id: &str,
    n: usize,
) -> lanmsg_core::store::TransferRow {
    let start = std::time::Instant::now();
    loop {
        if let Some(row) = store.get_transfer(id).unwrap() {
            if row.done_segments.iter().filter(|&&b| b).count() >= n {
                return row;
            }
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            panic!("10 秒内未完成 {n} 个分段");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// 在目录树下查找名为 name 的文件（递归，取第一个匹配）。
fn find_file(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()? {
        let p = entry.ok()?.path();
        if p.is_dir() {
            if let Some(found) = find_file(&p, name) {
                return Some(found);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}

/// 我的两台设备：a 已初始化，b 以 JoinExisting 加入并配对。
async fn self_device_pair(
    bus: &lanmsg_core::discovery::MemoryBus,
) -> (common::TestNode, common::TestNode) {
    let mut a = node(bus).await;
    a.init_first("A").await;
    let mut b = node(bus).await;
    b.call(Request::Init {
        display_name: "B".into(),
        device_name: "B".into(),
        mode: InitMode::JoinExisting,
    })
    .await;
    pair(&mut a, &mut b).await;
    (a, b)
}

#[tokio::test]
async fn download_large_file_in_parallel_segments() {
    let (mut a, mut x) = contact_pair().await;
    let size = 3 * BLOB_SEGMENT_LEN as usize + 123;
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "大文件.bin", size);
    let (_, hash_hex) = send_and_wait(&mut a, &mut x, &path, size).await;

    let dl = tempfile::tempdir().unwrap();
    set_download_dir(&x, dl.path()).await;
    let id = match x
        .call(Request::DownloadBlob {
            hash: hash_hex.clone(),
        })
        .await
    {
        Response::TransferId { value } => value,
        other => panic!("期望 TransferId，实际 {other:?}"),
    };
    assert!(!id.is_empty());
    let (ok, error) = wait_finished(&mut x, &hash_hex, 30).await;
    assert!(ok, "下载应成功，实际错误 {error:?}");
    // 文件位于 download_dir/<会话标题>/<名>，内容逐字节相同。
    let got = dl.path().join("A").join("大文件.bin");
    assert_eq!(std::fs::read(&got).unwrap(), std::fs::read(&path).unwrap());
    // .part 已消失，transfer 行 Done 且分段全完成。
    assert!(!dl.path().join("A").join("大文件.bin.part").exists());
    let row = x.svc.store_for_test().get_transfer(&id).unwrap().unwrap();
    assert_eq!(row.state, lanmsg_core::store::TransferState::Done);
    assert!(row.done_segments.iter().all(|&b| b));
}

#[tokio::test]
async fn download_from_my_other_device_when_sender_offline() {
    let bus = lanmsg_core::discovery::MemoryBus::new();
    let mut x = node(&bus).await;
    x.init_first("X").await;
    let (mut a, mut b) = self_device_pair(&bus).await;
    befriend(&mut x, &mut a).await;

    let size = 2 * BLOB_SEGMENT_LEN as usize;
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "转传.bin", size);
    let (_, hash_hex) = send_and_wait(&mut x, &mut a, &path, size).await;
    // 消息同步到 B。
    let _ = wait_file_item(
        &mut b,
        |it| !it.hash.is_empty() && it.size == size as u64,
        5,
    )
    .await;

    // A 先下载完成（并向 B 发 BlobHave）。
    let dl_a = tempfile::tempdir().unwrap();
    set_download_dir(&a, dl_a.path()).await;
    a.call(Request::DownloadBlob {
        hash: hash_hex.clone(),
    })
    .await;
    let (ok, error) = wait_finished(&mut a, &hash_hex, 30).await;
    assert!(ok, "A 下载应成功，实际错误 {error:?}");

    // X 下线，B 只能从 A 拉取。
    x.svc.shutdown().await;
    let dl_b = tempfile::tempdir().unwrap();
    set_download_dir(&b, dl_b.path()).await;
    b.call(Request::DownloadBlob {
        hash: hash_hex.clone(),
    })
    .await;
    let (ok, error) = wait_finished(&mut b, &hash_hex, 30).await;
    assert!(ok, "B 应从 A 下载成功，实际错误 {error:?}");
    let got = find_file(dl_b.path(), "转传.bin").expect("B 侧应有下载文件");
    assert_eq!(std::fs::read(&got).unwrap(), std::fs::read(&path).unwrap());
}

#[tokio::test]
async fn download_uses_two_holders_in_parallel() {
    let bus = lanmsg_core::discovery::MemoryBus::new();
    let mut x = node(&bus).await;
    x.init_first("X").await;
    let (mut a, mut b) = self_device_pair(&bus).await;
    befriend(&mut x, &mut a).await;

    let size = 2 * BLOB_SEGMENT_LEN as usize;
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "多源.bin", size);
    let (_, hash_hex) = send_and_wait(&mut x, &mut a, &path, size).await;
    let _ = wait_file_item(
        &mut b,
        |it| !it.hash.is_empty() && it.size == size as u64,
        5,
    )
    .await;

    // A 下载完成，成为第二持有者；X 与 A 都在线。
    let dl_a = tempfile::tempdir().unwrap();
    set_download_dir(&a, dl_a.path()).await;
    a.call(Request::DownloadBlob {
        hash: hash_hex.clone(),
    })
    .await;
    let (ok, error) = wait_finished(&mut a, &hash_hex, 30).await;
    assert!(ok, "A 下载应成功，实际错误 {error:?}");

    // B 下载：候选源为 {X, A} 两个持有者，多源并行完成。
    let dl_b = tempfile::tempdir().unwrap();
    set_download_dir(&b, dl_b.path()).await;
    b.call(Request::DownloadBlob {
        hash: hash_hex.clone(),
    })
    .await;
    let (ok, error) = wait_finished(&mut b, &hash_hex, 30).await;
    assert!(ok, "B 应从两个持有者下载成功，实际错误 {error:?}");
    let got = find_file(dl_b.path(), "多源.bin").expect("B 侧应有下载文件");
    assert_eq!(std::fs::read(&got).unwrap(), std::fs::read(&path).unwrap());
}

#[tokio::test]
async fn source_offline_fails_with_message() {
    let (mut a, mut x) = contact_pair().await;
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "孤源.bin", 1024);
    // X 发送、A 接收（A 无本地副本），X 下线后 A 无源可取。
    let (_, hash_hex) = send_and_wait(&mut x, &mut a, &path, 1024).await;

    x.svc.shutdown().await;
    let dl = tempfile::tempdir().unwrap();
    set_download_dir(&a, dl.path()).await;
    a.call(Request::DownloadBlob {
        hash: hash_hex.clone(),
    })
    .await;
    let (ok, error) = wait_finished(&mut a, &hash_hex, 15).await;
    assert!(!ok, "唯一持有者下线后下载应失败");
    assert_eq!(error.as_deref(), Some("源离线"));
    // transfer 行 Failed。
    let store = a.svc.store_for_test();
    let rows = store.active_transfers().unwrap();
    assert!(rows.is_empty(), "失败后不应有活动传输");
}

#[tokio::test]
async fn transfer_resumes_after_restart_with_verified_segments() {
    let (mut a, mut x) = contact_pair().await;
    let size = 24 * BLOB_SEGMENT_LEN as usize + 123;
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "续传.bin", size);
    let (_, hash_hex) = send_and_wait(&mut a, &mut x, &path, size).await;

    let dl = tempfile::tempdir().unwrap();
    set_download_dir(&x, dl.path()).await;
    let id = match x
        .call(Request::DownloadBlob {
            hash: hash_hex.clone(),
        })
        .await
    {
        Response::TransferId { value } => value,
        other => panic!("期望 TransferId，实际 {other:?}"),
    };
    // 收到首个进度（done > 0）即关停。
    x.wait_event(
        |e| {
            matches!(
                e,
                Event::TransferProgress { hash, done, .. } if hash == &hash_hex && *done > 0
            )
        },
        30,
    )
    .await;
    let common::TestNode {
        svc, dir: node_dir, ..
    } = x;
    svc.shutdown().await;

    // transfers 行有 ≥1 个 done 段；目标目录只有 .part 文件。
    let store = svc.store_for_test();
    let row = wait_done_segments(&store, &id, 1).await;
    assert!(row.done_segments.iter().any(|&b| b), "关停前应已有完成分段");
    assert_eq!(row.state, lanmsg_core::store::TransferState::Running);
    for entry in std::fs::read_dir(dl.path().join("A")).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            name.to_string_lossy().ends_with(".part"),
            "完成前不应出现无 .part 后缀的文件: {name:?}"
        );
    }
    drop(store);

    // 重启并重连 A → 续传完成。
    let mut x2 = node_offline_with(node_dir, PlatformHooks::default()).await;
    link(&x2, &a).await;
    let (ok, error) = wait_finished(&mut x2, &hash_hex, 60).await;
    assert!(ok, "续传应成功，实际错误 {error:?}");
    let got = find_file(dl.path(), "续传.bin").expect("续传后应有最终文件");
    assert_eq!(std::fs::read(&got).unwrap(), std::fs::read(&path).unwrap());
}

#[tokio::test]
async fn final_hash_mismatch_fails_and_cleans_part() {
    let (mut a, mut x) = contact_pair().await;
    let size = 6 * BLOB_SEGMENT_LEN as usize + 123;
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "损坏.bin", size);
    let (_, hash_hex) = send_and_wait(&mut a, &mut x, &path, size).await;

    let dl = tempfile::tempdir().unwrap();
    set_download_dir(&x, dl.path()).await;
    let id = match x
        .call(Request::DownloadBlob {
            hash: hash_hex.clone(),
        })
        .await
    {
        Response::TransferId { value } => value,
        other => panic!("期望 TransferId，实际 {other:?}"),
    };
    // 等至少一段完成，破坏该段中间字节（该段不会再写，无竞态）。
    let store = x.svc.store_for_test();
    let row = wait_done_segments(&store, &id, 1).await;
    drop(store);
    let done_idx = row
        .done_segments
        .iter()
        .position(|&b| b)
        .expect("应有完成分段");
    let offset = done_idx as u64 * BLOB_SEGMENT_LEN + BLOB_SEGMENT_LEN / 2;
    let part = row.part_path.clone();
    {
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&part)
            .unwrap();
        f.seek(SeekFrom::Start(offset)).unwrap();
        let mut byte = [0u8; 1];
        f.read_exact(&mut byte).unwrap();
        byte[0] ^= 0xFF;
        f.seek(SeekFrom::Current(-1)).unwrap();
        f.write_all(&byte).unwrap();
    }

    // 整文件复核失败 → Failed("校验失败") 且 .part 被删除。
    let (ok, error) = wait_finished(&mut x, &hash_hex, 30).await;
    assert!(!ok, "整文件复核应失败");
    assert_eq!(error.as_deref(), Some("校验失败"));
    assert!(!part.exists(), "校验失败后 .part 应被删除");
}

#[tokio::test]
async fn download_zero_byte_file() {
    let (mut a, mut x) = contact_pair().await;
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "空.bin", 0);
    let (_, hash_hex) = send_and_wait(&mut a, &mut x, &path, 0).await;

    let dl = tempfile::tempdir().unwrap();
    set_download_dir(&x, dl.path()).await;
    x.call(Request::DownloadBlob {
        hash: hash_hex.clone(),
    })
    .await;
    let (ok, error) = wait_finished(&mut x, &hash_hex, 15).await;
    assert!(ok, "空文件下载应成功，实际错误 {error:?}");
    let got = dl.path().join("A").join("空.bin");
    assert!(got.exists());
    assert_eq!(std::fs::metadata(&got).unwrap().len(), 0);
}

#[tokio::test]
async fn cancel_removes_part_file() {
    let (mut a, mut x) = contact_pair().await;
    let size = 16 * BLOB_SEGMENT_LEN as usize + 123;
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "取消.bin", size);
    let (_, hash_hex) = send_and_wait(&mut a, &mut x, &path, size).await;

    let dl = tempfile::tempdir().unwrap();
    set_download_dir(&x, dl.path()).await;
    let id = match x
        .call(Request::DownloadBlob {
            hash: hash_hex.clone(),
        })
        .await
    {
        Response::TransferId { value } => value,
        other => panic!("期望 TransferId，实际 {other:?}"),
    };
    x.wait_event(
        |e| {
            matches!(
                e,
                Event::TransferProgress { hash, done, .. } if hash == &hash_hex && *done > 0
            )
        },
        30,
    )
    .await;
    x.call(Request::CancelTransfer {
        transfer: id.clone(),
    })
    .await;
    let (ok, error) = wait_finished(&mut x, &hash_hex, 15).await;
    assert!(!ok, "取消后传输应失败");
    assert_eq!(error.as_deref(), Some("已取消"));
    let row = x.svc.store_for_test().get_transfer(&id).unwrap().unwrap();
    assert_eq!(row.state, lanmsg_core::store::TransferState::Cancelled);
    assert!(!row.part_path.exists(), "取消后 .part 应被删除");
}

#[test]
fn unique_path_and_sanitize() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    std::fs::write(&a, b"x").unwrap();
    assert_eq!(unique_path(&a), dir.path().join("a (1).txt"));
    std::fs::write(dir.path().join("a (1).txt"), b"x").unwrap();
    assert_eq!(unique_path(&a), dir.path().join("a (2).txt"));
    // 不存在则原样返回。
    let c = dir.path().join("c.bin");
    assert_eq!(unique_path(&c), c);

    assert_eq!(sanitize_component("a/b:c"), "a_b_c");
    assert_eq!(sanitize_component(" .. "), "_");
    assert_eq!(sanitize_component("正常名字.txt"), "正常名字.txt");
    // 路径穿越成分被替换。
    assert_eq!(sanitize_component(".."), "_");
}
