// P2 Task 5：联系人双向确认与准入过滤集成测试。
// 覆盖：请求/同意/拒绝/屏蔽状态机、互发请求自动同意、AcceptOnce 非法、
// accept_peer 七条准入规则表。
// 注释中文，标识符英文。
// 两节点测试一律用 node_offline + link（手动互拨），规避发现与手动拨号并存的
// 双连接裁决竞态。

mod common;

use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, watch, RwLock};

use lanmsg_api::types::{
    ErrorCode, Event, MessageState, Request, RequestAction, RequestKind, Response,
};
use lanmsg_core::hlc::HlcClock;
use lanmsg_core::identity::Identity;
use lanmsg_core::keystore::FileKeyStore;
use lanmsg_core::net::{ConnManager, HelloInfo, PeerInfo};
use lanmsg_core::store::{
    ContactRow, ContactState, ConvKindRow, ConvRow, DeviceRow, Gate, RequestKindRow, Store,
};
use lanmsg_core::trust::{GateDecision, Trust};

use common::{befriend, link, node_offline, TestNode};
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{BlobHash, ConvId, DeviceId, MsgId, UserId};
use lanmsg_proto::sign::{sign_with_zeroed_sig, DeviceKey, UserKey};
use lanmsg_proto::types::{
    DeviceCert, DeviceType, Envelope, Hlc, MsgBody, PathClass, Revocation, Signature64, Status,
    SyncPush,
};

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

/// 规则表/门控测试用的固定基准时间。
const NOW: i64 = 1_700_000_000_000;

/// 构造测试用 Envelope（gate_inbound 不验签，签名可置零）。
fn env_for(author: DeviceId, conv: ConvId, seq: u64, body: MsgBody) -> Envelope {
    Envelope {
        conv,
        id: MsgId { author, seq },
        conv_seq: seq,
        hlc: Hlc {
            millis: NOW + seq as i64,
            counter: 0,
        },
        body,
        enc: None,
        sig: Signature64([0u8; 64]),
    }
}

fn text_body(t: &str) -> MsgBody {
    MsgBody::Text { text: t.into() }
}

/// 构造设备行（作者入库用）。
fn dev_row(user: &UserKey, device: DeviceId) -> DeviceRow {
    let cert = cert_for(user, device);
    DeviceRow {
        device,
        user: user.user_id(),
        cert: cert.clone(),
        name: cert.device_name.clone(),
        device_type: DeviceType::Desktop,
        last_addr: None,
        last_path: None,
        last_seen: None,
        revoked: false,
    }
}

/// 陌生人场景：互连后 A 打开与 B 的 DM，返回会话 ID。
async fn stranger_conv(a: &TestNode, b: &TestNode) -> String {
    link(a, b).await;
    match a.call(Request::OpenDm { user: b.user_id() }).await {
        Response::ConvId { value } => value,
        other => panic!("期望 ConvId，实际 {other:?}"),
    }
}

/// 原始节点（不经 Service 组装；testutil 仅在单元测试可用，集成测试自建）。
struct RawNode {
    store: Arc<Store>,
    identity: Arc<RwLock<Identity>>,
    conns: ConnManager,
    msg: lanmsg_core::msg::Messaging,
    user: UserId,
    device: DeviceId,
    _dir: tempfile::TempDir,
}

/// 构造单个原始节点：内存 Store、已初始化身份、ConnManager（准入全放行）。
async fn raw_node() -> RawNode {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let ks = FileKeyStore::new(dir.path().join("keys"));
    let mut identity = Identity::load_or_create(&ks, &store).unwrap();
    identity
        .init_first_device(&ks, &store, "原始节点", DeviceType::Desktop, NOW)
        .unwrap();
    let user = identity.user_id().unwrap();
    let device = identity.device_id();
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
    if let Some(cert) = identity.read().await.cert.clone() {
        let _ = hello_tx.send_replace(HelloInfo {
            display_name: cert.device_name.clone(),
            cert: Some(cert),
            listen_port: 0,
        });
    }
    drop(hello_tx);
    let (events_tx, _events_rx) = broadcast::channel(16);
    let msg = lanmsg_core::msg::Messaging::new(
        store.clone(),
        identity.clone(),
        clock,
        conns.clone(),
        events_tx,
        lanmsg_core::PlatformHooks::default(),
    );
    RawNode {
        store,
        identity,
        conns,
        msg,
        user,
        device,
        _dir: dir,
    }
}

/// 构造一对已连接的原始节点（A 拨 B），并把对方设备写入库（无路由任务，
/// 手动 upsert）。
async fn raw_pair() -> (RawNode, RawNode) {
    let a = raw_node().await;
    let b = raw_node().await;
    let a_dev = a.device;
    let b_dev = b.device;
    let _ = a.conns.connect(b.conns.local_addr(), false).await;
    let start = std::time::Instant::now();
    loop {
        if a.conns.is_connected(&b_dev) && b.conns.is_connected(&a_dev) {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(5) {
            panic!("双方未在 5 秒内建立连接");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    for (src, dst) in [(&a, &b), (&b, &a)] {
        let cert = src.identity.read().await.cert.clone().unwrap();
        dst.store
            .upsert_device(&DeviceRow {
                device: src.device,
                user: src.user,
                cert,
                name: "对端设备".into(),
                device_type: DeviceType::Desktop,
                last_addr: None,
                last_path: None,
                last_seen: None,
                revoked: false,
            })
            .unwrap();
    }
    (a, b)
}

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
        // ContactAdded 经同步到达（响应先于消息，ContactsChanged 可能先到），
        // 轮询等待可见消息出现。
        common::wait_until(
            || async {
                matches!(
                    node.try_call(Request::ListMessages {
                        conv: conv.clone(),
                        before: None,
                        limit: 50,
                    })
                    .await,
                    Ok(Response::Messages { ref value }) if value.len() == 1
                )
            },
            5,
        )
        .await;
        let resp = node
            .call(Request::ListMessages {
                conv: conv.clone(),
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

/// 陌生人连发两条文字：B 只收到 1 次 Message 类请求，preview 为第二条前 50 字；
/// B 的会话列表不含该会话。
#[tokio::test]
async fn stranger_text_creates_single_request_and_hidden_conv() {
    let a = node_offline().await;
    let mut b = node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    let conv = stranger_conv(&a, &b).await;

    let long = "测".repeat(60);
    a.call(Request::SendText {
        conv: conv.clone(),
        text: "第一条你好".into(),
    })
    .await;
    a.call(Request::SendText {
        conv: conv.clone(),
        text: long,
    })
    .await;

    // B 收到 Message 类请求。
    b.wait_event(
        |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, RequestKind::Message)),
        5,
    )
    .await;
    // 折叠后仅 1 条 open 请求，preview 为第二条前 50 字。
    common::wait_until(
        || async {
            matches!(
                b.try_call(Request::ListRequests).await,
                Ok(Response::Requests { ref value })
                    if value.len() == 1 && value[0].preview == "测".repeat(50)
            )
        },
        5,
    )
    .await;
    // 不再出现第二次请求事件。
    b.no_event(|e| matches!(e, Event::IncomingRequest { .. }), 500)
        .await;

    // B 的会话列表不含该会话（门控隐藏）。
    let resp = b.call(Request::ListConversations).await;
    let Response::Conversations { value } = resp else {
        panic!("期望 Conversations");
    };
    assert!(
        !value.iter().any(|c| c.conv == conv),
        "隐藏会话不应出现在列表: {value:?}"
    );
}

/// 接收一次：揭示消息并进入 30 分钟窗口；窗口过期后再次 Pending。
#[tokio::test]
async fn accept_once_reveals_messages_and_window_expires() {
    let a = node_offline().await;
    let mut b = node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    let conv = stranger_conv(&a, &b).await;

    a.call(Request::SendText {
        conv: conv.clone(),
        text: "第一条".into(),
    })
    .await;
    a.call(Request::SendText {
        conv: conv.clone(),
        text: "第二条".into(),
    })
    .await;
    let req_id = match b
        .wait_event(
            |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, RequestKind::Message)),
            5,
        )
        .await
    {
        Event::IncomingRequest { request } => request.request,
        other => panic!("期望 IncomingRequest，实际 {other:?}"),
    };

    // B 接收一次：会话出现，2 条消息可见。
    b.call(Request::RespondRequest {
        request: req_id,
        action: RequestAction::AcceptOnce,
    })
    .await;
    common::wait_until(
        || async {
            matches!(
                b.try_call(Request::ListMessages {
                    conv: conv.clone(),
                    before: None,
                    limit: 10
                })
                .await,
                Ok(Response::Messages { ref value }) if value.len() == 2
            )
        },
        5,
    )
    .await;
    let resp = b.call(Request::ListConversations).await;
    let Response::Conversations { value } = resp else {
        panic!("期望 Conversations");
    };
    assert!(value.iter().any(|c| c.conv == conv), "接收一次后会话应可见");

    // 窗口过期（now 前移 31 分钟）：gate_inbound 再次判定 Pending。
    let (trust, store, self_cert, _self_device) = rules_env().await;
    let self_user = self_cert.user;
    let stranger = UserKey::generate();
    let stranger_dev = DeviceId([9u8; 32]);
    store
        .upsert_device(&dev_row(&stranger, stranger_dev))
        .unwrap();
    let gconv = ConvId::dm(&self_user, &stranger.user_id());
    store
        .ensure_conversation(&ConvRow {
            conv: gconv,
            kind: ConvKindRow::Dm,
            peer_user: Some(stranger.user_id()),
            title: None,
            created_at: 1,
        })
        .unwrap();
    store.set_gate(&gconv, Gate::Open, Some(NOW)).unwrap();
    let env = env_for(stranger_dev, gconv, 1, text_body("窗口内"));
    let lan = peer_info(
        Some(cert_for(&stranger, stranger_dev)),
        PathClass::Lan,
        false,
    );
    assert!(
        matches!(
            trust.gate_inbound(&lan, &env, NOW - 1_000).unwrap(),
            GateDecision::Accept
        ),
        "窗口内 → Accept"
    );
    assert!(
        matches!(
            trust
                .gate_inbound(&lan, &env, NOW + 31 * 60 * 1000)
                .unwrap(),
            GateDecision::Pending { .. }
        ),
        "31 分钟后 → Pending"
    );
}

/// 拒绝：消息被删除，之后该用户的新消息重新生成请求。
#[tokio::test]
async fn reject_deletes_and_next_message_requests_again() {
    let a = node_offline().await;
    let mut b = node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    let conv = stranger_conv(&a, &b).await;

    a.call(Request::SendText {
        conv: conv.clone(),
        text: "第一次".into(),
    })
    .await;
    let req1 = match b
        .wait_event(
            |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, RequestKind::Message)),
            5,
        )
        .await
    {
        Event::IncomingRequest { request } => request.request,
        other => panic!("期望 IncomingRequest，实际 {other:?}"),
    };

    // B 拒绝：消息删除、请求关闭。
    b.call(Request::RespondRequest {
        request: req1.clone(),
        action: RequestAction::Reject,
    })
    .await;
    let resp = b
        .call(Request::ListMessages {
            conv: conv.clone(),
            before: None,
            limit: 10,
        })
        .await;
    let Response::Messages { value } = resp else {
        panic!("期望 Messages");
    };
    assert!(value.is_empty(), "拒绝后消息应被删除: {value:?}");
    let resp = b.call(Request::ListRequests).await;
    let Response::Requests { value } = resp else {
        panic!("期望 Requests");
    };
    assert!(value.is_empty(), "拒绝后不应有 open 请求: {value:?}");

    // A 再发：重新生成请求（新 id）。
    a.call(Request::SendText {
        conv: conv.clone(),
        text: "第二次".into(),
    })
    .await;
    let req2 = match b
        .wait_event(
            |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, RequestKind::Message)),
            5,
        )
        .await
    {
        Event::IncomingRequest { request } => request.request,
        other => panic!("期望第二次 IncomingRequest，实际 {other:?}"),
    };
    assert_ne!(req2, req1, "拒绝后应生成新请求");
}

/// 屏蔽：断开连接，之后 A 再发的消息保持 Sending，B 无事件无入库。
#[tokio::test]
async fn block_drops_future_messages() {
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    a.init_first("甲").await;
    b.init_first("乙").await;
    let conv = stranger_conv(&a, &b).await;

    a.call(Request::SendText {
        conv: conv.clone(),
        text: "第一条".into(),
    })
    .await;
    let req_id = match b
        .wait_event(
            |e| matches!(e, Event::IncomingRequest { request } if matches!(request.kind, RequestKind::Message)),
            5,
        )
        .await
    {
        Event::IncomingRequest { request } => request.request,
        other => panic!("期望 IncomingRequest，实际 {other:?}"),
    };
    let b_device = b.device_id().await;
    b.call(Request::RespondRequest {
        request: req_id,
        action: RequestAction::Block,
    })
    .await;
    // 连接被断开。
    a.wait_event(
        |e| matches!(e, Event::PeerOffline { device } if *device == b_device),
        5,
    )
    .await;
    // 请求已关闭。
    let resp = b.call(Request::ListRequests).await;
    let Response::Requests { value } = resp else {
        panic!("期望 Requests");
    };
    assert!(value.is_empty(), "屏蔽后不应有 open 请求: {value:?}");

    // A 再发消息：无在线目标，2 秒后仍为 Sending；B 无事件。
    a.call(Request::SendText {
        conv: conv.clone(),
        text: "屏蔽后再发".into(),
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let resp = a
        .call(Request::ListMessages {
            conv: conv.clone(),
            before: None,
            limit: 10,
        })
        .await;
    let Response::Messages { value } = resp else {
        panic!("期望 Messages");
    };
    let last = value
        .iter()
        .find(|m| matches!(&m.body, lanmsg_api::types::MessageBodyView::Text { text } if text == "屏蔽后再发"))
        .expect("A 侧应有该消息");
    assert_eq!(
        last.state,
        MessageState::Sending,
        "屏蔽后消息应保持 Sending"
    );
    b.no_event(
        |e| {
            matches!(
                e,
                Event::MessageAdded { .. } | Event::IncomingRequest { .. }
            )
        },
        1_000,
    )
    .await;
}

/// 陌生人文件消息：请求类型为 File，preview 含文件名（SendFiles 未完成前
/// 用手工构造 Files envelope 经 on_frame 注入）。
#[tokio::test]
async fn stranger_file_request_kind_is_file() {
    let (a, b) = raw_pair().await;

    // B 侧构建 Trust 并注入门控回调（等价 Service 组装）。
    let clock = Arc::new(std::sync::Mutex::new(HlcClock::new()));
    let (tx, _rx) = broadcast::channel(16);
    let trust = Trust::new(
        b.store.clone(),
        b.identity.clone(),
        clock,
        b.conns.clone(),
        tx,
    );
    b.msg
        .set_gate_callback(Arc::new(move |p, e, n| trust.gate_inbound(p, e, n)));

    // A 手工构造 Files envelope（设备密钥签名）。
    let conv = ConvId::dm(&a.user, &b.user);
    let (seq, conv_seq) = a.store.next_seqs(&a.device, &conv).unwrap();
    let meta = lanmsg_proto::types::BlobMeta {
        hash: BlobHash([1u8; 32]),
        size: 2048,
        name: "机密.pdf".into(),
        mime: "application/pdf".into(),
        media: None,
        thumb: None,
    };
    let mut env = Envelope {
        conv,
        id: MsgId {
            author: a.device,
            seq,
        },
        conv_seq,
        hlc: Hlc {
            millis: NOW + 1,
            counter: 0,
        },
        body: MsgBody::Files { items: vec![meta] },
        enc: None,
        sig: Signature64([0u8; 64]),
    };
    let key_bytes = a.identity.read().await.device.to_bytes();
    let key = DeviceKey::from_bytes(&key_bytes);
    sign_with_zeroed_sig(&mut env, |b| key.sign(b));

    // 经 on_frame 注入 B。
    b.msg
        .on_frame(
            a.device,
            Frame::SyncPush(SyncPush {
                envelopes: vec![env.clone()],
            }),
        )
        .await
        .unwrap();

    // B：open 请求恰 1 条，kind File，preview 含文件名。
    let reqs = b.store.list_open_requests().unwrap();
    assert_eq!(reqs.len(), 1, "应恰有 1 条 open 请求: {reqs:?}");
    assert_eq!(reqs[0].kind, RequestKindRow::File);
    assert!(
        reqs[0].preview.contains("机密.pdf"),
        "preview 应含文件名: {}",
        reqs[0].preview
    );
    // 消息已入库但隐藏。
    assert!(b.store.get_message(&env.id).unwrap().is_some());
    assert!(
        b.store.list_messages(&conv, None, 10).unwrap().is_empty(),
        "隐藏消息不应出现在列表"
    );
}

/// gate_inbound 七条规则逐条断言。
#[tokio::test]
async fn gate_rules_table() {
    let (trust, store, self_cert, self_device) = rules_env().await;
    let self_user = self_cert.user;
    let own_from = peer_info(Some(self_cert.clone()), PathClass::Lan, false);

    // 规则 1：from 设备属于自己 → Accept（即使作者是陌生人）。
    let s1 = UserKey::generate();
    let s1_dev = DeviceId([21u8; 32]);
    store.upsert_device(&dev_row(&s1, s1_dev)).unwrap();
    let env1 = env_for(
        s1_dev,
        ConvId::dm(&self_user, &s1.user_id()),
        1,
        text_body("规则一"),
    );
    assert!(
        matches!(
            trust.gate_inbound(&own_from, &env1, NOW).unwrap(),
            GateDecision::Accept
        ),
        "规则1: 自己设备转交 → Accept"
    );

    // 规则 2：作者用户是自己 → Accept。
    let s2 = UserKey::generate();
    let s2_dev = DeviceId([22u8; 32]);
    let env2 = env_for(
        self_device,
        ConvId::dm(&self_user, &s2.user_id()),
        1,
        text_body("规则二"),
    );
    let s2_from = peer_info(Some(cert_for(&s2, s2_dev)), PathClass::Lan, false);
    assert!(
        matches!(
            trust.gate_inbound(&s2_from, &env2, NOW).unwrap(),
            GateDecision::Accept
        ),
        "规则2: 作者用户为自己 → Accept"
    );

    // 规则 3：作者用户 Blocked → Drop。
    let s3 = UserKey::generate();
    let s3_dev = DeviceId([23u8; 32]);
    store.upsert_device(&dev_row(&s3, s3_dev)).unwrap();
    seed_contact(&store, s3.user_id(), ContactState::Blocked);
    let env3 = env_for(
        s3_dev,
        ConvId::dm(&self_user, &s3.user_id()),
        1,
        text_body("规则三"),
    );
    let s3_from = peer_info(Some(cert_for(&s3, s3_dev)), PathClass::Lan, false);
    assert!(
        matches!(
            trust.gate_inbound(&s3_from, &env3, NOW).unwrap(),
            GateDecision::Drop
        ),
        "规则3: Blocked → Drop"
    );

    // 规则 4：作者用户 Accepted 联系人 → Accept。
    let s4 = UserKey::generate();
    let s4_dev = DeviceId([24u8; 32]);
    store.upsert_device(&dev_row(&s4, s4_dev)).unwrap();
    seed_contact(&store, s4.user_id(), ContactState::Accepted);
    let env4 = env_for(
        s4_dev,
        ConvId::dm(&self_user, &s4.user_id()),
        1,
        text_body("规则四"),
    );
    let s4_from = peer_info(Some(cert_for(&s4, s4_dev)), PathClass::Lan, false);
    assert!(
        matches!(
            trust.gate_inbound(&s4_from, &env4, NOW).unwrap(),
            GateDecision::Accept
        ),
        "规则4: Accepted → Accept"
    );

    // 规则 5：会话存在且 gate Open、窗口未失效 → Accept。
    let s5 = UserKey::generate();
    let s5_dev = DeviceId([25u8; 32]);
    store.upsert_device(&dev_row(&s5, s5_dev)).unwrap();
    let conv5 = ConvId::dm(&self_user, &s5.user_id());
    store
        .ensure_conversation(&ConvRow {
            conv: conv5,
            kind: ConvKindRow::Dm,
            peer_user: Some(s5.user_id()),
            title: None,
            created_at: 1,
        })
        .unwrap();
    let env5 = env_for(s5_dev, conv5, 1, text_body("规则五"));
    let s5_from = peer_info(Some(cert_for(&s5, s5_dev)), PathClass::Lan, false);
    store.set_gate(&conv5, Gate::Open, None).unwrap();
    assert!(
        matches!(
            trust.gate_inbound(&s5_from, &env5, NOW).unwrap(),
            GateDecision::Accept
        ),
        "规则5: gate Open 且窗口空 → Accept"
    );
    store
        .set_gate(&conv5, Gate::Open, Some(NOW + 1_000))
        .unwrap();
    assert!(
        matches!(
            trust.gate_inbound(&s5_from, &env5, NOW).unwrap(),
            GateDecision::Accept
        ),
        "规则5: gate Open 且窗口未到 → Accept"
    );
    store
        .set_gate(&conv5, Gate::Open, Some(NOW - 1_000))
        .unwrap();
    assert!(
        matches!(
            trust.gate_inbound(&s5_from, &env5, NOW).unwrap(),
            GateDecision::Pending { .. }
        ),
        "规则5: 窗口已过 → 落入规则 7 Pending"
    );

    // 规则 6：非 Lan 且非（Vpn 且出站）→ Drop（陌生作者、无会话）。
    let s6 = UserKey::generate();
    let s6_dev = DeviceId([26u8; 32]);
    store.upsert_device(&dev_row(&s6, s6_dev)).unwrap();
    let env6 = env_for(
        s6_dev,
        ConvId::dm(&self_user, &s6.user_id()),
        1,
        text_body("规则六"),
    );
    let internet = peer_info(Some(cert_for(&s6, s6_dev)), PathClass::Internet, false);
    let relay = peer_info(Some(cert_for(&s6, s6_dev)), PathClass::Relay, false);
    let vpn_in = peer_info(Some(cert_for(&s6, s6_dev)), PathClass::Vpn, false);
    assert!(
        matches!(
            trust.gate_inbound(&internet, &env6, NOW).unwrap(),
            GateDecision::Drop
        ),
        "规则6: Internet → Drop"
    );
    assert!(
        matches!(
            trust.gate_inbound(&relay, &env6, NOW).unwrap(),
            GateDecision::Drop
        ),
        "规则6: Relay → Drop"
    );
    assert!(
        matches!(
            trust.gate_inbound(&vpn_in, &env6, NOW).unwrap(),
            GateDecision::Drop
        ),
        "规则6: VPN 入站 → Drop"
    );

    // 规则 7：其余（Lan 陌生人）→ Pending{request_id}，会话转 Pending、
    // open 请求登记；Vpn 出站同样 Pending。
    let s7 = UserKey::generate();
    let s7_dev = DeviceId([27u8; 32]);
    store.upsert_device(&dev_row(&s7, s7_dev)).unwrap();
    let conv7 = ConvId::dm(&self_user, &s7.user_id());
    let env7 = env_for(s7_dev, conv7, 1, text_body("规则七"));
    let lan7 = peer_info(Some(cert_for(&s7, s7_dev)), PathClass::Lan, false);
    let vpn_out = peer_info(Some(cert_for(&s7, s7_dev)), PathClass::Vpn, true);
    let decision = trust.gate_inbound(&lan7, &env7, NOW).unwrap();
    let GateDecision::Pending { request_id } = &decision else {
        panic!("规则7: Lan 陌生人 → Pending，实际 {decision:?}");
    };
    assert!(!request_id.is_empty());
    assert_eq!(
        store.get_gate(&conv7).unwrap(),
        (Gate::Pending, None),
        "规则7: 会话 gate 置 Pending"
    );
    // 规则 5 过期分支也会为 s5 登记请求，这里只统计 s7 的 open 请求。
    let reqs: Vec<_> = store
        .list_open_requests()
        .unwrap()
        .into_iter()
        .filter(|r| r.from_user == s7.user_id())
        .collect();
    assert_eq!(reqs.len(), 1, "规则7: 应登记 open 请求");
    assert_eq!(reqs[0].request_id, *request_id);
    assert_eq!(reqs[0].kind, RequestKindRow::Message);
    assert_eq!(reqs[0].conv, Some(conv7));
    // 同作者第二条（Vpn 出站）折叠进同一请求（只更新 preview）。
    let env7b = env_for(s7_dev, conv7, 2, text_body("规则七第二条"));
    let decision2 = trust.gate_inbound(&vpn_out, &env7b, NOW).unwrap();
    let GateDecision::Pending { request_id: id2 } = decision2 else {
        panic!("规则7: Vpn 出站陌生人 → Pending");
    };
    assert_eq!(id2, *request_id, "同作者同类型应折叠为同一请求");
    let reqs: Vec<_> = store
        .list_open_requests()
        .unwrap()
        .into_iter()
        .filter(|r| r.from_user == s7.user_id())
        .collect();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].preview, "规则七第二条", "折叠只更新 preview");
}

/// P2 Task 8：联系人伪造的吊销记录被拒——验签失败返回 Err，设备未吊销。
#[tokio::test]
async fn revocation_from_non_owner_is_ignored() {
    let (trust, store, _self_cert, self_device) = rules_env().await;
    let self_user = store.get_device(&self_device).unwrap().unwrap().user;

    // 联系人 X 及其设备入库（Accepted）。
    let x = UserKey::generate();
    let x_device = DeviceKey::generate().device_id();
    let x_cert = cert_for(&x, x_device);
    seed_contact(&store, x.user_id(), ContactState::Accepted);
    store.upsert_device(&dev_row(&x, x_device)).unwrap();

    // X 用自己的用户密钥伪造「吊销 A 的设备」：user 字段填 A，签名却是 X 的。
    let mut r = Revocation {
        user: self_user,
        device: self_device,
        revoked_at: NOW,
        key_epoch: 0,
        sig: Signature64([0u8; 64]),
    };
    sign_with_zeroed_sig(&mut r, |b| x.sign(b));

    let from = peer_info(Some(x_cert), PathClass::Lan, false);
    assert!(
        trust.on_revocation(&from, r).await.is_err(),
        "伪造吊销应被拒绝"
    );

    // 设备未吊销。
    assert!(!store.is_revoked(&self_device).unwrap());
    assert!(!store.get_device(&self_device).unwrap().unwrap().revoked);
}
