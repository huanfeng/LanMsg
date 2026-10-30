// P2 Task 5：联系人双向确认与准入过滤集成测试。
// 覆盖：请求/同意/拒绝/屏蔽状态机、互发请求自动同意、AcceptOnce 非法、
// accept_peer 七条准入规则表。
// 注释中文，标识符英文。
// 两节点测试一律用 node_offline + link（手动互拨），规避发现与手动拨号并存的
// 双连接裁决竞态。

mod common;

use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, watch, RwLock};

use lanmsg_api::types::{ErrorCode, Event, Request, RequestAction, RequestKind, Response};
use lanmsg_core::hlc::HlcClock;
use lanmsg_core::identity::Identity;
use lanmsg_core::keystore::FileKeyStore;
use lanmsg_core::net::{ConnManager, HelloInfo, PeerInfo};
use lanmsg_core::store::{ContactRow, ContactState, Store};
use lanmsg_core::trust::Trust;

use common::{befriend, link, node_offline};
use lanmsg_proto::ids::{DeviceId, UserId};
use lanmsg_proto::sign::UserKey;
use lanmsg_proto::types::{DeviceCert, DeviceType, PathClass, Status};

// ===== 辅助 =====

/// 构造测试用设备证书（指定用户密钥与设备 ID）。
fn cert_for(user: &UserKey, device: DeviceId) -> DeviceCert {
    DeviceCert::issue(
        user,
        device,
        "对端设备".into(),
        DeviceType::Desktop,
        1_700_000_000_000,
    )
}

/// 构造 PeerInfo（地址全零占位）。
fn peer_info(cert: Option<DeviceCert>, path: PathClass, outbound: bool) -> PeerInfo {
    let device = cert
        .as_ref()
        .map(|c| c.device)
        .unwrap_or(DeviceId([0u8; 32]));
    PeerInfo {
        device,
        cert,
        display_name: "对端".into(),
        addr: "127.0.0.1:1".parse().unwrap(),
        dial_addr: "127.0.0.1:1".parse().unwrap(),
        path,
        caps: vec![],
        outbound,
        tls_exporter: [0u8; 32],
    }
}

/// 写入一条联系人行（updated 取固定小值，仅规则表使用）。
fn seed_contact(store: &Store, user: UserId, state: ContactState) {
    store
        .upsert_contact_if_newer(&ContactRow {
            user,
            state,
            auto_accept: false,
            ask_above_bytes: None,
            updated: lanmsg_proto::types::Hlc {
                millis: 1,
                counter: 0,
            },
        })
        .unwrap();
}

/// 搭建规则表测试环境：内存 Store + 已初始化身份 + Trust（返回自身 UserKey 供签发证书）。
async fn rules_env() -> (Trust, Arc<Store>, DeviceCert, DeviceId) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let ks = FileKeyStore::new(dir.path().join("keys"));
    let mut identity = Identity::load_or_create(&ks, &store).unwrap();
    identity
        .init_first_device(&ks, &store, "自己", DeviceType::Desktop, 1_700_000_000_000)
        .unwrap();
    let self_cert = identity.cert.clone().unwrap();
    let self_device = identity.device_id();
    let identity = Arc::new(RwLock::new(identity));
    let clock = Arc::new(std::sync::Mutex::new(HlcClock::new()));
    let (hello_tx, hello_rx) = watch::channel(HelloInfo {
        display_name: String::new(),
        cert: None,
        listen_port: 0,
    });
    let (conn_tx, _conn_rx) = mpsc::channel(16);
    let conns = ConnManager::bind(
        identity.clone(),
        0,
        hello_rx,
        Arc::new(|_: &PeerInfo| true),
        conn_tx,
    )
    .await
    .unwrap();
    drop(hello_tx);
    let (events_tx, _events_rx) = broadcast::channel(16);
    let trust = Trust::new(store.clone(), identity, clock, conns, events_tx);
    (trust, store, self_cert, self_device)
}

// ===== 集成测试 =====

/// 请求→同意→双方成为联系人；双方 DM 中恰 1 条可见 System(ContactAdded)。
#[tokio::test]
async fn contact_request_accept_makes_both_contacts() {
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    link(&a, &b).await;

    befriend(&mut a, &mut b).await;

    // 双方 ListContacts 各含对方。
    for (node, other) in [(&a, b.user_id()), (&b, a.user_id())] {
        let resp = node.call(Request::ListContacts).await;
        let Response::Contacts { value } = resp else {
            panic!("期望 Contacts");
        };
        assert!(
            value.iter().any(|c| c.user_id == other),
            "联系人列表应含对方: {value:?}"
        );
    }

    // 双方 DM 会话中恰有 1 条可见 System 消息（ContactAdded，视图层为 Unsupported）。
    let conv_a = match a.call(Request::OpenDm { user: b.user_id() }).await {
        Response::ConvId { value } => value,
        other => panic!("期望 ConvId，实际 {other:?}"),
    };
    let conv_b = match b.call(Request::OpenDm { user: a.user_id() }).await {
        Response::ConvId { value } => value,
        other => panic!("期望 ConvId，实际 {other:?}"),
    };
    for (node, conv) in [(&a, conv_a), (&b, conv_b)] {
        let resp = node
            .call(Request::ListMessages {
                conv,
                before: None,
                limit: 50,
            })
            .await;
        let Response::Messages { value } = resp else {
            panic!("期望 Messages");
        };
        assert_eq!(value.len(), 1, "DM 应恰有 1 条可见消息: {value:?}");
        assert!(
            matches!(
                &value[0].body,
                lanmsg_api::types::MessageBodyView::Unsupported
            ),
            "应为 System(ContactAdded)（视图层 Unsupported）: {:?}",
            value[0].body
        );
    }
}

/// 互发请求自动同意：双方 Accepted，无 open 请求残留。
#[tokio::test]
async fn mutual_requests_auto_accept() {
    let a = node_offline().await;
    let b = node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    link(&a, &b).await;

    a.call(Request::RequestContact { user: b.user_id() }).await;
    b.call(Request::RequestContact { user: a.user_id() }).await;

    // 双方等 ListContacts 各含对方。
    common::wait_until(
        || async {
            let a_sees = matches!(
                a.try_call(Request::ListContacts).await,
                Ok(Response::Contacts { ref value })
                    if value.iter().any(|c| c.user_id == b.user_id())
            );
            let b_sees = matches!(
                b.try_call(Request::ListContacts).await,
                Ok(Response::Contacts { ref value })
                    if value.iter().any(|c| c.user_id == a.user_id())
            );
            a_sees && b_sees
        },
        5,
    )
    .await;

    // 无 open 请求残留。
    for node in [&a, &b] {
        let resp = node.call(Request::ListRequests).await;
        let Response::Requests { value } = resp else {
            panic!("期望 Requests");
        };
        assert!(value.is_empty(), "不应有 open 请求残留: {value:?}");
    }
}

#[tokio::test]
async fn reject_then_requester_sees_rejected_and_can_retry() {
    let mut a = common::node_offline().await;
    let mut b = common::node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    common::link(&a, &b).await;

    // B 拒绝 A 的请求：B 上关闭请求，A 收到 accepted:false 后状态置 Rejected。
    a.call(Request::RequestContact { user: b.user_id() }).await;
    let req_id = match b
        .wait_event(
            |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, lanmsg_api::types::RequestKind::Contact)),
            5,
        )
        .await
    {
        Event::IncomingRequest { request } => request.request,
        other => panic!("期望 IncomingRequest，实际 {other:?}"),
    };
    b.call(Request::RespondRequest {
        request: req_id,
        action: RequestAction::Reject,
    })
    .await;
    b.wait_event(|e| matches!(e, Event::ContactsChanged), 5)
        .await;

    // A 等 ContactsChanged（Outgoing → Rejected），ListContacts 不含 B。
    a.wait_event(|e| matches!(e, Event::ContactsChanged), 5)
        .await;
    let resp = a.call(Request::ListContacts).await;
    let Response::Contacts { value } = resp else {
        panic!("期望 Contacts");
    };
    assert!(
        !value.iter().any(|c| c.user_id == b.user_id()),
        "A 的联系人列表不应含 B: {value:?}"
    );

    // A 重试：Rejected → Outgoing，B 再次收到请求。
    a.call(Request::RequestContact { user: b.user_id() }).await;
    b.wait_event(
        |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, lanmsg_api::types::RequestKind::Contact)),
        5,
    )
    .await;
}

/// AcceptOnce 对联系人请求非法 → InvalidArgument。
#[tokio::test]
async fn accept_once_on_contact_request_is_invalid() {
    let a = common::node_offline().await;
    let mut b = common::node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    common::link(&a, &b).await;

    a.call(Request::RequestContact { user: b.user_id() }).await;
    let req_id = match b
        .wait_event(
            |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, RequestKind::Contact)),
            5,
        )
        .await
    {
        Event::IncomingRequest { request } => request.request,
        other => panic!("期望 IncomingRequest，实际 {other:?}"),
    };

    let err = b
        .try_call(Request::RespondRequest {
            request: req_id,
            action: RequestAction::AcceptOnce,
        })
        .await
        .expect_err("AcceptOnce 应返回错误");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

/// 屏蔽断开对方所有连接并拒绝重连。
#[tokio::test]
async fn block_disconnects_and_refuses_reconnect() {
    let mut a = common::node_offline().await;
    let mut b = common::node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    let req_id = common::befriend(&mut a, &mut b).await;

    // B 屏蔽 A：A 的设备被断开 → A 收到 PeerOffline(B 设备)。
    let b_device = b.device_id().await;
    b.call(Request::RespondRequest {
        request: req_id,
        action: RequestAction::Block,
    })
    .await;
    a.wait_event(
        |e| matches!(e, Event::PeerOffline { device } if *device == b_device),
        5,
    )
    .await;

    // A 手动重拨 B：B 的准入过滤拒绝 → 3 秒内 A 无 PeerOnline(B 设备)。
    let addr = b.svc.local_addr().to_string();
    let _ = a.try_call(Request::AddManualPeer { addr }).await;
    a.no_event(
        |e| matches!(e, Event::PeerOnline { peer } if peer.device_id == b_device),
        3_000,
    )
    .await;
}

/// accept_peer 七条准入规则逐条断言。
#[tokio::test]
async fn accept_peer_rules_table() {
    let (trust, store, self_cert, _self_device) = rules_env().await;

    // 规则 1：cert None —— 仅配对窗口开放时接受。
    let anon = peer_info(None, PathClass::Lan, false);
    assert!(
        !trust.accept_peer(&anon, Status::Online, false),
        "规则1: 无证书+窗口关 → 拒"
    );
    assert!(
        trust.accept_peer(&anon, Status::Online, true),
        "规则1: 无证书+窗口开 → 接受"
    );

    // 陌生人用户与设备（各规则复用）。
    let stranger = UserKey::generate();
    let stranger_user = stranger.user_id();
    let stranger_device = DeviceId([7u8; 32]);
    let stranger_cert = cert_for(&stranger, stranger_device);

    // 规则 2：设备已吊销 → 拒绝（否则 LAN 陌生人会被规则 7 放行）。
    store
        .insert_revocation(&lanmsg_proto::types::Revocation::issue(
            &stranger,
            stranger_device,
            1,
        ))
        .unwrap();
    let revoked_peer = peer_info(Some(stranger_cert.clone()), PathClass::Lan, false);
    assert!(
        !trust.accept_peer(&revoked_peer, Status::Online, false),
        "规则2: 吊销 → 拒"
    );

    // 规则 3：自己的设备 → 接受（含 Invisible 状态）。
    let own_peer = peer_info(Some(self_cert), PathClass::Lan, false);
    assert!(
        trust.accept_peer(&own_peer, Status::Online, false),
        "规则3: 自己 → 接受"
    );
    assert!(
        trust.accept_peer(&own_peer, Status::Invisible, false),
        "规则3: 自己 + Invisible → 接受"
    );

    // 规则 4：Blocked 联系人 → 拒绝（LAN 路径也不例外）。
    let blocked_user = UserKey::generate();
    seed_contact(&store, blocked_user.user_id(), ContactState::Blocked);
    let blocked_peer = peer_info(
        Some(cert_for(&blocked_user, DeviceId([4u8; 32]))),
        PathClass::Lan,
        false,
    );
    assert!(
        !trust.accept_peer(&blocked_peer, Status::Online, false),
        "规则4: Blocked → 拒"
    );

    // 规则 5：Invisible 时非自己设备一律拒绝（Accepted 联系人也不例外）。
    let acc_user = UserKey::generate();
    seed_contact(&store, acc_user.user_id(), ContactState::Accepted);
    let acc_peer = peer_info(
        Some(cert_for(&acc_user, DeviceId([5u8; 32]))),
        PathClass::Lan,
        false,
    );
    assert!(
        !trust.accept_peer(&acc_peer, Status::Invisible, false),
        "规则5: Invisible → 拒"
    );

    // 规则 6：Accepted 联系人任意路径接受（含 Internet）。
    assert!(
        trust.accept_peer(&acc_peer, Status::Online, false),
        "规则6: Accepted+Lan → 接受"
    );
    let acc_internet = peer_info(
        Some(cert_for(&acc_user, DeviceId([5u8; 32]))),
        PathClass::Internet,
        false,
    );
    assert!(
        trust.accept_peer(&acc_internet, Status::Online, false),
        "规则6: Accepted+Internet → 接受"
    );

    // 规则 7：陌生人按路径与方向（换未吊销的新设备，避免触发规则 2）。
    let fresh = UserKey::generate();
    let fresh_cert = cert_for(&fresh, DeviceId([8u8; 32]));
    let lan = peer_info(Some(fresh_cert.clone()), PathClass::Lan, false);
    let vpn_in = peer_info(Some(fresh_cert.clone()), PathClass::Vpn, false);
    let vpn_out = peer_info(Some(fresh_cert.clone()), PathClass::Vpn, true);
    let internet = peer_info(Some(fresh_cert.clone()), PathClass::Internet, false);
    let relay = peer_info(Some(fresh_cert.clone()), PathClass::Relay, false);
    assert!(
        trust.accept_peer(&lan, Status::Online, false),
        "规则7: 陌生人 LAN → 接受"
    );
    assert!(
        !trust.accept_peer(&vpn_in, Status::Online, false),
        "规则7: 陌生人 VPN 入站 → 拒"
    );
    assert!(
        trust.accept_peer(&vpn_out, Status::Online, false),
        "规则7: 陌生人 VPN 出站 → 接受"
    );
    assert!(
        !trust.accept_peer(&internet, Status::Online, false),
        "规则7: 陌生人 Internet → 拒"
    );
    assert!(
        !trust.accept_peer(&relay, Status::Online, false),
        "规则7: 陌生人 Relay → 拒"
    );
    let _ = stranger_user;
}
