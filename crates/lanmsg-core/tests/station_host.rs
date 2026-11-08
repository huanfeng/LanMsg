// P5 Task 5：中转站主机端经请求入口的集成测试（创建、放入、错误码、事件）。
// 注释中文，标识符英文。

mod common;

use lanmsg_api::types::{ErrorCode, Event, Request, Response, StationPutItem, StoreMode};

use common::node_offline;

#[tokio::test]
async fn host_creates_station_and_accepts_items() {
    let mut n = node_offline().await;
    n.init_first("alice").await;

    let station = match n.call(Request::StationCreate { name: "box".into() }).await {
        Response::ConvId { value } => value,
        other => panic!("expected ConvId, got {other:?}"),
    };

    let put = |item| Request::StationPut {
        station: station.clone(),
        item,
        mode: StoreMode::Reference,
        expires_at: None,
    };
    n.call(put(StationPutItem::Text { text: "hi".into() }))
        .await;
    n.wait_event(
        |e| matches!(e, Event::StationUpdated { station: s } if *s == station),
        2,
    )
    .await;

    let file = n.dir.path().join("a.txt");
    std::fs::write(&file, b"hello").unwrap();
    n.call(put(StationPutItem::Paths {
        paths: vec![file.to_string_lossy().into_owned()],
    }))
    .await;

    // 不存在的站点、非法 ID、不存在的条目都给出对应错误码。
    let err = n
        .try_call(Request::StationPut {
            station: "00".repeat(32),
            item: StationPutItem::Text { text: "x".into() },
            mode: StoreMode::Copy,
            expires_at: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    let err = n
        .try_call(Request::StationRemove {
            station: station.clone(),
            item: "zz".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    let err = n
        .try_call(Request::StationSetPinned {
            station,
            item: "00".repeat(16),
            pinned: true,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn station_name_is_validated() {
    let n = node_offline().await;
    n.init_first("alice").await;
    let err = n
        .try_call(Request::StationCreate { name: " ".into() })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}
