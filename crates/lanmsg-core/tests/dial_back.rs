// P5 Task 3B：NAT 下数据通道反向建立。
// 用测试钩子把下载方看到的“源端 dial_addr”指向已关闭的端口（模拟源端在 NAT 后、直拨不通），
// 验证下载方经控制连接请求源端拨回，文件仍能完整下载。
// 注释中文，标识符英文。

mod common;

use std::sync::Arc;

use common::{befriend, node_with, TestNode};
use lanmsg_api::types::{Event, MessageBodyView, Request, Response};
use lanmsg_core::discovery::MemoryBus;
use lanmsg_core::platform::{NetworkState, PlatformHooks};
use lanmsg_proto::ids::DeviceId;

/// 非 wifi 钩子：默认策略下不自动下载，便于手动触发下载。
fn not_wifi_hooks() -> PlatformHooks {
    PlatformHooks {
        network_state: Some(Arc::new(|| NetworkState {
            is_metered: false,
            is_wifi_or_wired: false,
        })),
        thumbnailer: None,
    }
}

/// 已初始化并结为联系人的两个节点；返回 (a, x)，a 为拨号方。
async fn contact_pair() -> (TestNode, TestNode) {
    let bus = MemoryBus::new();
    let mut a = node_with(&bus, tempfile::tempdir().unwrap(), not_wifi_hooks()).await;
    a.init_first("A").await;
    let mut x = node_with(&bus, tempfile::tempdir().unwrap(), not_wifi_hooks()).await;
    x.init_first("X").await;
    befriend(&mut a, &mut x).await;
    (a, x)
}

/// sender 向 receiver 发一个文件；receiver 把 sender 的 dial_addr 改成已关闭端口后下载，
/// 断言下载成功且内容一致。
async fn transfer_with_unreachable_sender(sender: &mut TestNode, receiver: &mut TestNode) {
    let size = 300 * 1024;
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("nat.bin");
    let content: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &content).unwrap();

    let conv = match sender
        .call(Request::OpenDm {
            user: receiver.user_id(),
        })
        .await
    {
        Response::ConvId { value } => value,
        other => panic!("expected ConvId, got {other:?}"),
    };
    sender
        .call(Request::SendFiles {
            conv,
            paths: vec![path.to_string_lossy().into_owned()],
        })
        .await;
    // 接收方等到带哈希的文件消息。
    let hash = loop {
        let ev = receiver
            .wait_event(
                |e| matches!(e, Event::MessageAdded { .. } | Event::MessageUpdated { .. }),
                10,
            )
            .await;
        let (Event::MessageAdded { msg } | Event::MessageUpdated { msg }) = ev else {
            continue;
        };
        if let MessageBodyView::Files { items } = &msg.body {
            if let Some(it) = items.first() {
                if !it.hash.is_empty() && it.size == size as u64 {
                    break it.hash.clone();
                }
            }
        }
    };

    // 模拟发送方在 NAT 后：接收方直拨它的地址会被拒绝（端口已关闭）。
    let sender_dev = DeviceId::from_hex(&sender.device_id().await).unwrap();
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    receiver.svc.override_dial_addr_for_test(sender_dev, closed);

    let dl = tempfile::tempdir().unwrap();
    let Response::Settings { mut value } = receiver.call(Request::GetSettings).await else {
        panic!("expected Settings");
    };
    value.download_dir = dl.path().to_string_lossy().into_owned();
    receiver
        .call(Request::SetSettings { settings: value })
        .await;
    receiver
        .call(Request::DownloadBlob { hash: hash.clone() })
        .await;
    let ev = receiver
        .wait_event(
            |e| matches!(e, Event::TransferFinished { hash: h, .. } if h == &hash),
            30,
        )
        .await;
    let Event::TransferFinished { ok, error, .. } = ev else {
        unreachable!()
    };
    assert!(
        ok,
        "download should succeed via dial-back, error: {error:?}"
    );
    let got = std::fs::read_dir(dl.path())
        .unwrap()
        .flatten()
        .flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten())
        .find(|f| f.file_name() == "nat.bin")
        .expect("downloaded file not found");
    assert_eq!(std::fs::read(got.path()).unwrap(), content);
}

/// 发送方是拨号方（接收方是入站方）。
#[tokio::test]
async fn data_channel_falls_back_to_dial_back() {
    let (mut a, mut x) = contact_pair().await;
    transfer_with_unreachable_sender(&mut a, &mut x).await;
}

/// 入站方传文件：发送方是被拨入的一方，接收方是拨号方。
#[tokio::test]
async fn inbound_side_can_send_file_via_dial_back() {
    let (mut a, mut x) = contact_pair().await;
    transfer_with_unreachable_sender(&mut x, &mut a).await;
}
