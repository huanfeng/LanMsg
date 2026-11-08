// P1 核心骨架：Service 集成测试（Task 11）。
// 两个 Service 经 MemoryBus 发现、聊天；静态对端直连；隐身与重启语义。
// 注释中文，标识符英文。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use lanmsg_api::types::{
    ConvKind, ErrorCode, Event, InitMode, Request, RequestAction, RequestKind, Response, StatusDto,
};
use lanmsg_core::discovery::{DiscoveryMode, MemoryBus};
use lanmsg_core::keystore::FileKeyStore;
use lanmsg_core::platform::PlatformHooks;
use lanmsg_core::{Service, ServiceConfig};

/// 在临时目录中启动一个 Service。
async fn make_service(bus: Option<MemoryBus>, dir: &tempfile::TempDir) -> Service {
    Service::start(ServiceConfig {
        data_dir: dir.path().to_path_buf(),
        listen_port: 0,
        discovery: match bus {
            Some(b) => DiscoveryMode::Memory(b),
            None => DiscoveryMode::Disabled,
        },
        keystore: Arc::new(FileKeyStore::new(dir.path().join("keys"))),
        static_peers: Vec::new(),
        platform: PlatformHooks::default(),
        localsend_port: 0,
    })
    .await
    .expect("Service 启动失败")
}

fn init_req(name: &str) -> Request {
    Request::Init {
        display_name: name.into(),
        device_name: format!("{name}-设备"),
        mode: InitMode::FirstDevice,
    }
}

/// 取下一条匹配事件，5 秒超时 panic。
async fn next_event(rx: &mut broadcast::Receiver<Event>, pred: impl Fn(&Event) -> bool) -> Event {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Ok(event)) => {
                if pred(&event) {
                    return event;
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) => panic!("事件通道已关闭"),
            Err(_) => panic!("5 秒内未收到匹配事件"),
        }
    }
}

/// 轮询异步请求直到条件成立，最多 5 秒。
async fn wait_until(mut poll: impl AsyncFnMut() -> Option<Response>) -> Response {
    let start = std::time::Instant::now();
    loop {
        if let Some(resp) = poll().await {
            return resp;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("5 秒内条件未成立");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn two_services_discover_and_chat_over_memory_bus() {
    let bus = MemoryBus::new();
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let a = make_service(Some(bus.clone()), &dir_a).await;
    let b = make_service(Some(bus), &dir_b).await;
    let mut ev_a = a.subscribe();
    let mut ev_b = b.subscribe();

    a.handle(init_req("甲")).await.unwrap();
    b.handle(init_req("乙")).await.unwrap();

    let b_self = b.handle(Request::GetSelf).await.unwrap();
    let Response::SelfInfo { value: b_info } = b_self else {
        panic!("期望 SelfInfo");
    };
    let b_user = b_info.user_id.clone().unwrap();
    let b_device = b_info.device_id.clone();

    // 5 秒内 A 收到 PeerOnline(B)。
    next_event(
        &mut ev_a,
        |e| matches!(e, Event::PeerOnline { peer } if peer.device_id == b_device),
    )
    .await;
    // ListPeers 含 B。
    let resp = wait_until(async || {
        let resp = a.handle(Request::ListPeers).await.ok()?;
        match resp {
            Response::Peers { value } => value
                .iter()
                .any(|p| p.device_id == b_device)
                .then_some(Response::Peers { value }),
            other => Some(other),
        }
    })
    .await;
    let Response::Peers { value } = resp else {
        panic!("期望 Peers");
    };
    assert!(value.iter().any(|p| p.device_id == b_device));

    // 先结为联系人：陌生人的消息会被门控拦截为 Pending（P2 Task 6）。
    a.handle(Request::RequestContact {
        user: b_user.clone(),
    })
    .await
    .unwrap();
    let ev = next_event(
        &mut ev_b,
        |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, RequestKind::Contact)),
    )
    .await;
    let Event::IncomingRequest { request } = ev else {
        panic!("期望 IncomingRequest");
    };
    b.handle(Request::RespondRequest {
        request: request.request,
        action: RequestAction::AddContact,
    })
    .await
    .unwrap();
    next_event(&mut ev_a, |e| matches!(e, Event::ContactsChanged)).await;
    next_event(&mut ev_b, |e| matches!(e, Event::ContactsChanged)).await;

    // A: OpenDm(B) → SendText("hello")。
    let resp = a.handle(Request::OpenDm { user: b_user }).await.unwrap();
    let Response::ConvId { value: conv_id } = resp else {
        panic!("期望 ConvId");
    };
    let resp = a
        .handle(Request::SendText {
            conv: conv_id.clone(),
            text: "hello".into(),
        })
        .await
        .unwrap();
    let Response::MsgId { value: msg_id } = resp else {
        panic!("期望 MsgId");
    };

    // B 收到 MessageAdded。
    next_event(
        &mut ev_b,
        |e| matches!(e, Event::MessageAdded { msg } if msg.id == msg_id),
    )
    .await;

    // B: ListMessages 返回 2 条（ContactAdded + hello）。
    let resp = b
        .handle(Request::ListMessages {
            conv: conv_id,
            before: None,
            limit: 10,
        })
        .await
        .unwrap();
    let Response::Messages { value } = resp else {
        panic!("期望 Messages");
    };
    assert_eq!(value.len(), 2);
    assert!(
        value
            .iter()
            .any(|m| matches!(&m.body, lanmsg_api::types::MessageBodyView::Text { text } if text == "hello")),
        "应含 hello 消息: {value:?}"
    );
}

#[tokio::test]
async fn static_peer_connects_without_discovery() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let a = make_service(None, &dir_a).await;
    a.handle(init_req("甲")).await.unwrap();

    // B 以 A 的地址为 static peer 启动（DiscoveryMode::Disabled）。
    let a_addr = a.local_addr();
    let b = Service::start(ServiceConfig {
        data_dir: dir_b.path().to_path_buf(),
        listen_port: 0,
        discovery: DiscoveryMode::Disabled,
        keystore: Arc::new(FileKeyStore::new(dir_b.path().join("keys"))),
        static_peers: vec![a_addr],
        platform: PlatformHooks::default(),
        localsend_port: 0,
    })
    .await
    .unwrap();
    b.handle(init_req("乙")).await.unwrap();
    let mut ev_a = a.subscribe();

    let b_self = b.handle(Request::GetSelf).await.unwrap();
    let Response::SelfInfo { value: b_info } = b_self else {
        panic!("期望 SelfInfo");
    };
    let b_device = b_info.device_id;

    // 5 秒内 A 收到 PeerOnline(B)。
    next_event(
        &mut ev_a,
        |e| matches!(e, Event::PeerOnline { peer } if peer.device_id == b_device),
    )
    .await;
}

#[tokio::test]
async fn uninitialized_service_rejects_send() {
    let dir = tempfile::tempdir().unwrap();
    let s = make_service(None, &dir).await;
    let err = s
        .handle(Request::SendText {
            conv: "00".repeat(32),
            text: "x".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInitialized);
}

/// 未实现的请求返回 Unimplemented 错误码，而不是 Internal。
#[tokio::test]
async fn unimplemented_request_returns_unimplemented_code() {
    let dir = tempfile::tempdir().unwrap();
    let s = make_service(None, &dir).await;
    let err = s.handle(Request::WebStart).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Unimplemented);
    assert!(
        err.message.starts_with("unimplemented: "),
        "message: {}",
        err.message
    );
}

#[tokio::test]
async fn list_interfaces_returns_ok() {
    let dir = tempfile::tempdir().unwrap();
    let s = make_service(None, &dir).await;
    let Response::Interfaces { value } = s.handle(Request::ListInterfaces).await.unwrap() else {
        panic!("expected Interfaces");
    };
    assert!(value
        .iter()
        .all(|i| i.addr != "127.0.0.1" && i.addr != "::1"));
}

#[tokio::test]
async fn invisible_hides_from_other_users() {
    let bus = MemoryBus::new();
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let a = make_service(Some(bus.clone()), &dir_a).await;
    let b = make_service(Some(bus.clone()), &dir_b).await;

    a.handle(init_req("甲")).await.unwrap();
    b.handle(init_req("乙")).await.unwrap();
    let a_self = a.handle(Request::GetSelf).await.unwrap();
    let Response::SelfInfo { value: a_info } = a_self else {
        panic!("期望 SelfInfo");
    };
    let a_device = a_info.device_id;

    // A 切隐身（announce 置 None，不再主动外联其他用户）。
    a.handle(Request::SetStatus {
        status: StatusDto::Invisible,
    })
    .await
    .unwrap();

    // B 重启（新 Service，同一 data_dir；身份已初始化，不再 Init）。
    b.shutdown().await;
    drop(b);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let b2 = make_service(Some(bus), &dir_b).await;
    let mut ev_b2 = b2.subscribe();

    // 3 秒内 B 收不到 PeerOnline(A)。
    let probe = async {
        loop {
            let e = ev_b2.recv().await.unwrap();
            if matches!(&e, Event::PeerOnline { peer } if peer.device_id == a_device) {
                return e;
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(3), probe)
        .await
        .is_ok()
    {
        panic!("B 不应收到 PeerOnline(A)");
    }
}

#[tokio::test]
async fn init_first_device_creates_self_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let s = make_service(None, &dir).await;
    s.handle(init_req("测试者")).await.unwrap();

    let resp = s.handle(Request::ListConversations).await.unwrap();
    let Response::Conversations { value } = resp else {
        panic!("期望 Conversations");
    };
    assert!(
        value
            .iter()
            .any(|c| matches!(c.kind, ConvKind::SelfDevices) && c.title.is_empty()),
        "应包含标题为空的自设备会话，实际: {value:?}"
    );

    let resp = s.handle(Request::GetSelf).await.unwrap();
    let Response::SelfInfo { value } = resp else {
        panic!("期望 SelfInfo");
    };
    assert_eq!(value.display_name.as_deref(), Some("测试者"));
}

#[tokio::test]
async fn restart_keeps_identity_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let s = make_service(None, &dir).await;
    s.handle(init_req("持久")).await.unwrap();

    let resp = s.handle(Request::GetSelf).await.unwrap();
    let Response::SelfInfo { value: info1 } = resp else {
        panic!("期望 SelfInfo");
    };
    let user1 = info1.user_id.clone().unwrap();

    // 与一个伪造对端开 DM 并发送（无对端在线，消息保持 Sending 但入库）。
    let peer_hex = "ab".repeat(32);
    let resp = s.handle(Request::OpenDm { user: peer_hex }).await.unwrap();
    let Response::ConvId { value: conv_id } = resp else {
        panic!("期望 ConvId");
    };
    s.handle(Request::SendText {
        conv: conv_id.clone(),
        text: "历史消息".into(),
    })
    .await
    .unwrap();

    // 重启：同一 data_dir。
    s.shutdown().await;
    drop(s);
    let s2 = make_service(None, &dir).await;

    let resp = s2.handle(Request::GetSelf).await.unwrap();
    let Response::SelfInfo { value: info2 } = resp else {
        panic!("期望 SelfInfo");
    };
    assert_eq!(info2.user_id, Some(user1), "重启后身份不变");

    let resp = s2
        .handle(Request::ListMessages {
            conv: conv_id,
            before: None,
            limit: 10,
        })
        .await
        .unwrap();
    let Response::Messages { value } = resp else {
        panic!("期望 Messages");
    };
    assert_eq!(value.len(), 1, "重启后历史仍在");
    assert!(
        matches!(&value[0].body, lanmsg_api::types::MessageBodyView::Text { text } if text == "历史消息")
    );
}
