// P5 Task 6：中转站同步、拉取端代理写入与按哈希下载的集成测试。
// h 是主机（先建站），p 是与它配对的另一台自己的设备。
// 注释中文，标识符英文。

mod common;

use lanmsg_api::types::{
    BlobState, Event, Request, Response, StationItemView, StationPutItem, StationView, StoreMode,
};
use lanmsg_core::discovery::MemoryBus;
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId};
use lanmsg_proto::station::{ItemId, StationContent, StationItem, StationMsg};

use common::{node, pair, wait_until, TestNode};

async fn setup() -> (TestNode, TestNode, String) {
    let bus = MemoryBus::new();
    let mut h = node(&bus).await;
    h.init_first("host").await;
    let mut p = node(&bus).await;
    p.call(Request::Init {
        display_name: "phone".into(),
        device_name: "phone".into(),
        mode: lanmsg_api::types::InitMode::JoinExisting,
    })
    .await;
    pair(&mut h, &mut p).await;
    let station = match h
        .call(Request::StationCreate {
            name: "shelf".into(),
        })
        .await
    {
        Response::ConvId { value } => value,
        other => panic!("expected ConvId, got {other:?}"),
    };
    (h, p, station)
}

async fn stations(n: &TestNode) -> Vec<StationView> {
    match n.call(Request::StationList).await {
        Response::Stations { value } => value,
        other => panic!("expected Stations, got {other:?}"),
    }
}

async fn items(n: &TestNode, station: &str) -> Vec<StationItemView> {
    match n
        .try_call(Request::StationItems {
            station: station.into(),
        })
        .await
    {
        Ok(Response::StationItems { value }) => value,
        Ok(other) => panic!("expected StationItems, got {other:?}"),
        Err(_) => Vec::new(),
    }
}

async fn put_text(n: &TestNode, station: &str, text: &str) {
    n.call(Request::StationPut {
        station: station.into(),
        item: StationPutItem::Text { text: text.into() },
        mode: StoreMode::Copy,
        expires_at: None,
    })
    .await;
}

async fn set_download_dir(n: &TestNode, dir: &std::path::Path) {
    let Response::Settings { mut value } = n.call(Request::GetSettings).await else {
        panic!("expected Settings")
    };
    value.download_dir = dir.to_string_lossy().into_owned();
    n.call(Request::SetSettings { settings: value }).await;
}

async fn item_count_is(n: &TestNode, station: &str, count: usize) -> bool {
    items(n, station).await.len() == count
}

#[tokio::test]
async fn phone_sees_station_items_after_connect() {
    let (h, p, station) = setup().await;
    put_text(&h, &station, "a").await;
    wait_until(
        || async { stations(&p).await.iter().any(|s| s.station == station) },
        10,
    )
    .await;
    wait_until(|| item_count_is(&p, &station, 1), 10).await;
    let view = &items(&p, &station).await[0];
    assert_eq!(view.text.as_deref(), Some("a"));
    let list = stations(&p).await;
    assert!(list[0].host_online);
    assert!(!list[0].is_local, "phone is not the host");
    assert_eq!(list[0].item_count, 1);
    assert!(stations(&h).await[0].is_local);
}

#[tokio::test]
async fn delta_pushes_new_items_live() {
    let (h, mut p, station) = setup().await;
    put_text(&h, &station, "a").await;
    wait_until(|| item_count_is(&p, &station, 1), 10).await;
    put_text(&h, &station, "b").await;
    p.wait_event(|e| matches!(e, Event::StationUpdated { .. }), 10)
        .await;
    wait_until(|| item_count_is(&p, &station, 2), 10).await;
}

#[tokio::test]
async fn phone_downloads_station_file_by_hash() {
    let (h, mut p, station) = setup().await;
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("big.bin");
    let content: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &content).unwrap();
    h.call(Request::StationPut {
        station: station.clone(),
        item: StationPutItem::Paths {
            paths: vec![path.to_string_lossy().into_owned()],
        },
        mode: StoreMode::Reference,
        expires_at: None,
    })
    .await;
    wait_until(|| item_count_is(&p, &station, 1), 10).await;
    let file = items(&p, &station).await[0].file.clone().unwrap();
    assert_eq!(file.state, BlobState::Remote);
    let dl = tempfile::tempdir().unwrap();
    set_download_dir(&p, dl.path()).await;
    p.call(Request::DownloadBlob {
        hash: file.hash.clone(),
    })
    .await;
    let ev = p
        .wait_event(
            |e| matches!(e, Event::TransferFinished { hash, .. } if *hash == file.hash),
            20,
        )
        .await;
    assert!(
        matches!(ev, Event::TransferFinished { ok: true, .. }),
        "{ev:?}"
    );
    let got = std::fs::read(dl.path().join("Station").join("shelf").join("big.bin")).unwrap();
    assert_eq!(got, content);
}

#[tokio::test]
async fn phone_put_file_is_pulled_by_host() {
    let (h, p, station) = setup().await;
    wait_until(
        || async { stations(&p).await.iter().any(|s| s.station == station) },
        10,
    )
    .await;
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("note.txt");
    std::fs::write(&path, b"from the phone").unwrap();
    p.call(Request::StationPut {
        station: station.clone(),
        item: StationPutItem::Paths {
            paths: vec![path.to_string_lossy().into_owned()],
        },
        mode: StoreMode::Copy,
        expires_at: None,
    })
    .await;
    // 主机条目拉取完成后可用，且副本文件落在数据目录的 Station 下。
    wait_until(
        || async {
            let v = items(&h, &station).await;
            v.len() == 1 && v[0].unavailable.is_none()
        },
        30,
    )
    .await;
    let base = h.dir.path().join("Station").join(&station);
    let found = walk(&base)
        .into_iter()
        .any(|f| std::fs::read(&f).is_ok_and(|b| b == b"from the phone"));
    assert!(found, "no copied file under {base:?}");
    // 放入者的原文件不受影响。
    assert_eq!(std::fs::read(&path).unwrap(), b"from the phone");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[tokio::test]
async fn phone_pins_item_remotely() {
    let (h, p, station) = setup().await;
    for t in ["one", "two", "three"] {
        put_text(&h, &station, t).await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    wait_until(|| item_count_is(&p, &station, 3), 10).await;
    // 列表按添加时间降序，最后一条是最早的。
    let oldest = items(&p, &station).await.last().unwrap().item.clone();
    p.call(Request::StationSetPinned {
        station: station.clone(),
        item: oldest.clone(),
        pinned: true,
    })
    .await;
    wait_until(
        || async {
            let v = items(&p, &station).await;
            v.first().is_some_and(|i| i.item == oldest && i.pinned)
        },
        10,
    )
    .await;
}

#[tokio::test]
async fn phone_removes_item_remotely_and_late_delivery_does_not_revive() {
    let (h, p, station) = setup().await;
    put_text(&h, &station, "gone").await;
    wait_until(|| item_count_is(&p, &station, 1), 10).await;
    let id = items(&p, &station).await[0].item.clone();
    p.call(Request::StationRemove {
        station: station.clone(),
        item: id.clone(),
    })
    .await;
    wait_until(|| item_count_is(&h, &station, 0), 10).await;
    wait_until(|| item_count_is(&p, &station, 0), 10).await;
    // 迟到的旧投递（同一条目 id）经 Put 到达主机：墓碑优先，不会复活。
    let me = DeviceId([0u8; 32]);
    let item = StationItem {
        item: ItemId::from_hex(&id).unwrap(),
        added_by: me,
        added_at: 1,
        expires_at: None,
        content: StationContent::Text {
            text: "gone".into(),
        },
        pinned: false,
        removed: false,
        available: true,
        updated_at: 1,
    };
    let h_dev = h.device_id().await;
    p.svc
        .send_frame_for_test(
            &DeviceId::from_hex(&h_dev).unwrap(),
            Frame::Station(StationMsg::Put {
                station: ConvId::from_hex(&station).unwrap(),
                req: 9_999,
                item,
                mode_copy: false,
            }),
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(item_count_is(&h, &station, 0).await);
}

#[tokio::test]
async fn phone_puts_known_blob_without_upload() {
    let (mut h, p, station) = setup().await;
    // h 先在「我的设备」会话里发一个文件，p 同步到消息但不下载。
    let Response::Conversations { value: convs } = h.call(Request::ListConversations).await else {
        panic!("expected Conversations")
    };
    let self_conv = convs
        .iter()
        .find(|c| c.kind == lanmsg_api::types::ConvKind::SelfDevices)
        .unwrap()
        .conv
        .clone();
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("known.bin");
    std::fs::write(&path, vec![7u8; 1_000_000]).unwrap();
    h.call(Request::SendFiles {
        conv: self_conv,
        paths: vec![path.to_string_lossy().into_owned()],
    })
    .await;
    // 等 p 看到该文件。
    let hash = loop_find_hash(&p).await;
    wait_until(
        || async { stations(&p).await.iter().any(|s| s.station == station) },
        10,
    )
    .await;
    p.call(Request::StationPut {
        station: station.clone(),
        item: StationPutItem::Blobs {
            hashes: vec![hash.clone()],
        },
        mode: StoreMode::Reference,
        expires_at: None,
    })
    .await;
    wait_until(|| item_count_is(&h, &station, 1), 10).await;
    let view = &items(&h, &station).await[0];
    assert!(view.unavailable.is_none());
    assert_eq!(view.file.as_ref().unwrap().hash, hash);
    // 主机没有为此回传任何数据。
    h.no_event(|e| matches!(e, Event::TransferProgress { .. }), 500)
        .await;
}

/// 轮询 p 的文件视图，直到出现 known.bin，返回其哈希。
async fn loop_find_hash(p: &TestNode) -> String {
    for _ in 0..100 {
        if let Response::Files { value } = p
            .call(Request::QueryFiles {
                filter: lanmsg_api::types::FileFilter {
                    kind: lanmsg_api::types::FileKindFilter::All,
                    conv: None,
                    source_device: None,
                    state: None,
                    name_query: None,
                    before_millis: None,
                    limit: 10,
                },
            })
            .await
        {
            if let Some(f) = value.iter().find(|f| f.item.name == "known.bin") {
                return f.item.hash.clone();
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("known.bin never synced");
}

#[tokio::test]
async fn host_offline_marks_items_source_offline() {
    let (h, p, station) = setup().await;
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("f.bin");
    std::fs::write(&path, b"data").unwrap();
    h.call(Request::StationPut {
        station: station.clone(),
        item: StationPutItem::Paths {
            paths: vec![path.to_string_lossy().into_owned()],
        },
        mode: StoreMode::Reference,
        expires_at: None,
    })
    .await;
    wait_until(|| item_count_is(&p, &station, 1), 10).await;
    h.call(Request::Shutdown).await;
    wait_until(
        || async {
            let v = items(&p, &station).await;
            let offline = v
                .first()
                .and_then(|i| i.file.as_ref())
                .is_some_and(|f| f.state == BlobState::SourceOffline);
            offline && stations(&p).await.iter().all(|s| !s.host_online)
        },
        10,
    )
    .await;
}

#[tokio::test]
async fn contact_cannot_write_to_station() {
    // 另一个用户：与 h 互为联系人后，直接向 h 发 Put，必须被拒绝。
    let bus2 = MemoryBus::new();
    let mut h2 = node(&bus2).await;
    h2.init_first("host2").await;
    let mut c = node(&bus2).await;
    c.init_first("contact").await;
    common::befriend(&mut h2, &mut c).await;
    let st2 = match h2.call(Request::StationCreate { name: "s2".into() }).await {
        Response::ConvId { value } => value,
        other => panic!("expected ConvId, got {other:?}"),
    };
    let item = StationItem {
        item: ItemId::random(),
        added_by: DeviceId([0u8; 32]),
        added_at: 1,
        expires_at: None,
        content: StationContent::Text { text: "x".into() },
        pinned: false,
        removed: false,
        available: true,
        updated_at: 1,
    };
    let h2_dev = DeviceId::from_hex(&h2.device_id().await).unwrap();
    c.svc
        .send_frame_for_test(
            &h2_dev,
            Frame::Station(StationMsg::Put {
                station: ConvId::from_hex(&st2).unwrap(),
                req: 1,
                item,
                mode_copy: false,
            }),
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(item_count_is(&h2, &st2, 0).await);
    // 联系人也看不到该站点。
    assert!(stations(&c).await.is_empty());
}

#[tokio::test]
async fn remote_rejection_message_is_not_double_wrapped() {
    let (h, p, station) = setup().await;
    put_text(&h, &station, "a").await;
    wait_until(|| item_count_is(&p, &station, 1), 10).await;
    // 主机端报 NotFound(条目 id)，拉取端看到 Rejected 且文本恰为内层文本。
    let missing = "00".repeat(16);
    let err = p
        .try_call(Request::StationRemove {
            station: station.clone(),
            item: missing.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, lanmsg_api::types::ErrorCode::Rejected);
    assert_eq!(err.message, missing);
}

#[tokio::test]
async fn contact_cannot_remove_or_pin_station_items() {
    let bus = MemoryBus::new();
    let mut h = node(&bus).await;
    h.init_first("host").await;
    let mut c = node(&bus).await;
    c.init_first("contact").await;
    common::befriend(&mut h, &mut c).await;
    let st = match h.call(Request::StationCreate { name: "s".into() }).await {
        Response::ConvId { value } => value,
        other => panic!("expected ConvId, got {other:?}"),
    };
    put_text(&h, &st, "keep").await;
    let item = items(&h, &st).await[0].item.clone();
    let h_dev = DeviceId::from_hex(&h.device_id().await).unwrap();
    let station = ConvId::from_hex(&st).unwrap();
    let id = ItemId::from_hex(&item).unwrap();
    for msg in [
        StationMsg::Remove {
            station,
            req: 1,
            item: id,
        },
        StationMsg::SetPinned {
            station,
            req: 2,
            item: id,
            pinned: true,
        },
        StationMsg::Subscribe { station, since: 0 },
    ] {
        c.svc
            .send_frame_for_test(&h_dev, Frame::Station(msg))
            .await
            .unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let v = items(&h, &st).await;
    assert_eq!(v.len(), 1);
    assert!(!v[0].pinned);
    // 联系人没有收到快照，看不到该站点。
    assert!(stations(&c).await.is_empty());
}
