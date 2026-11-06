// P3 Task 7：SetUiFocus 只存内存，Service::ui_focus() 读取。
// 注释中文，标识符英文。

mod common;

use lanmsg_api::types::{ErrorCode, Request, Response};
use lanmsg_proto::ids::ConvId;

#[tokio::test]
async fn ui_focus_roundtrip() {
    let n = common::node_offline().await;
    assert_eq!(n.svc.ui_focus(), None);

    let conv = ConvId([7u8; 32]);
    let resp = n
        .svc
        .handle(Request::SetUiFocus {
            conv: Some(conv.to_hex()),
        })
        .await
        .unwrap();
    assert!(matches!(resp, Response::Ok));
    assert_eq!(n.svc.ui_focus(), Some(conv));

    n.svc
        .handle(Request::SetUiFocus { conv: None })
        .await
        .unwrap();
    assert_eq!(n.svc.ui_focus(), None);
}

#[tokio::test]
async fn ui_focus_rejects_invalid_hex_and_keeps_old_value() {
    let n = common::node_offline().await;
    let conv = ConvId([9u8; 32]);
    n.svc
        .handle(Request::SetUiFocus {
            conv: Some(conv.to_hex()),
        })
        .await
        .unwrap();

    let err = n
        .svc
        .handle(Request::SetUiFocus {
            conv: Some("zz-not-hex".into()),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert_eq!(n.svc.ui_focus(), Some(conv));
}
