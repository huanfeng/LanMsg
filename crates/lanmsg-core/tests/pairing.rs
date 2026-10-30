// P2 Task 7：「我的设备」配对集成测试。
// 覆盖：Qr 配对共享身份、Sas 双端确认、数字不一致双失败、错误口令零泄漏、
// 未加入模式初始化拒绝、窗口未开拒绝。
// 注释中文，标识符英文。

mod common;

use std::time::Duration;

use lanmsg_api::types::{
    ConvKind, ErrorCode, Event, InitMode, PairMethod, PairStateDto, PairTarget, Request, Response,
    StatusDto,
};
use lanmsg_core::discovery::MemoryBus;

use common::{node, node_offline, pair, wait_until, TestNode};

// ===== 辅助 =====

/// 匹配指定会话的 PairStatus{Done}。
fn is_done(session: &str) -> impl Fn(&Event) -> bool + '_ {
    move |e| {
        matches!(
            e,
            Event::PairStatus { session: s, state: PairStateDto::Done } if s.as_str() == session
        )
    }
}

/// 匹配指定会话的 PairStatus{Failed}。
fn is_failed(session: &str) -> impl Fn(&Event) -> bool + '_ {
    move |e| {
        matches!(
            e,
            Event::PairStatus { session: s, state: PairStateDto::Failed { .. } } if s.as_str() == session
        )
    }
}

/// 匹配指定会话的 PairStatus{ShowSas}，返回 code。
async fn wait_show_sas(node: &mut TestNode, session: &str) -> String {
    let ev = node
        .wait_event(
            |e| {
                matches!(
                    e,
                    Event::PairStatus {
                        session: s,
                        state: PairStateDto::ShowSas { .. }
                    } if s.as_str() == session
                )
            },
            5,
        )
        .await;
    match ev {
        Event::PairStatus {
            state: PairStateDto::ShowSas { code },
            ..
        } => code,
        other => panic!("期望 ShowSas，实际 {other:?}"),
    }
}

/// 触发 A 重新通告（B 后启动会错过值变化事件，靠 SetStatus 触发一次即时通告）。
async fn reannounce(a: &TestNode) {
    a.try_call(Request::SetStatus {
        status: StatusDto::Online,
    })
    .await
    .expect("SetStatus 失败");
}

/// 以 Device 目标发起 PairJoin，等待发现表收录 A 后成功（NotFound 视为尚未发现）。
/// 返回 B 的配对会话 ID（Sas/Device 目标下 B 本地生成，双方事件与确认均以它为准）。
async fn join_device(b: &TestNode, a_dev: &str) -> String {
    for _ in 0..50 {
        match b
            .try_call(Request::PairJoin {
                target: PairTarget::Device {
                    device: a_dev.to_string(),
                },
            })
            .await
        {
            Ok(Response::PairSession { value }) => return value.session,
            Err(e) if e.code == ErrorCode::NotFound => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => panic!("PairJoin 失败: {e:?}"),
            Ok(other) => panic!("期望 PairSession，实际 {other:?}"),
        }
    }
    panic!("5 秒内 PairJoin 未成功");
}

/// B 已加入模式初始化（Init JoinExisting）。
async fn init_join(b: &TestNode) {
    b.try_call(Request::Init {
        display_name: "乙机".into(),
        device_name: "乙机".into(),
        mode: InitMode::JoinExisting,
    })
    .await
    .expect("Init JoinExisting 失败");
}

/// B 以 Qr URI 加入 A 的配对窗口。
async fn join_qr(b: &TestNode, uri: String) {
    b.try_call(Request::PairJoin {
        target: PairTarget::QrUri { uri },
    })
    .await
    .expect("PairJoin 失败");
}

// ===== 测试 =====

/// 安装测试日志（RUST_LOG 控制，默认 warn）。
fn init_log() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(
            std::env::var("RUST_LOG")
                .ok()
                .and_then(|l| l.parse().ok())
                .unwrap_or(tracing::level_filters::LevelFilter::WARN),
        )
        .with_writer(std::io::stderr)
        .try_init();
}

/// Qr 配对：B 获得与 A 相同的用户身份，双方收到 Done 与 DevicesChanged，
/// A 的 SelfDevices 会话出现 DeviceAdded 系统消息。
#[tokio::test]
async fn qr_pairing_shares_identity() {
    init_log();
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    a.init_first("甲机").await;
    init_join(&b).await;

    let started = match a
        .try_call(Request::PairStart {
            method: PairMethod::Qr,
        })
        .await
        .expect("PairStart 失败")
    {
        Response::PairSession { value } => value,
        other => panic!("期望 PairSession，实际 {other:?}"),
    };
    let uri = started.qr_uri.expect("Qr 配对应返回 URI");
    let session = started.session;
    join_qr(&b, uri).await;

    a.wait_event(is_done(&session), 5).await;
    b.wait_event(is_done(&session), 5).await;
    a.wait_event(|e| matches!(e, Event::DevicesChanged), 5)
        .await;
    b.wait_event(|e| matches!(e, Event::DevicesChanged), 5)
        .await;

    // B 的用户身份 == A 的。
    let a_user = a.user_id();
    match b.try_call(Request::GetSelf).await.expect("GetSelf 失败") {
        Response::SelfInfo { value } => {
            assert!(value.initialized);
            assert_eq!(value.user_id.as_deref(), Some(a_user.as_str()));
        }
        other => panic!("期望 SelfInfo，实际 {other:?}"),
    }

    // A 的 SelfDevices 会话出现 DeviceAdded 系统消息（配对前该会话为空；
    // message_view 对 System 体暂渲染 Unsupported，故以「出现消息」断言）。
    wait_until_conv_has_message(&a).await;
}

/// 轮询 A 的 SelfDevices 会话，直到出现第一条消息。
async fn wait_until_conv_has_message(a: &TestNode) {
    let start = std::time::Instant::now();
    loop {
        let found = has_any_message(a).await;
        if found {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("5 秒内 SelfDevices 未出现 DeviceAdded 系统消息");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn has_any_message(a: &TestNode) -> bool {
    let convs = match a
        .try_call(Request::ListConversations)
        .await
        .expect("会话查询失败")
    {
        Response::Conversations { value } => value,
        other => panic!("期望 Conversations，实际 {other:?}"),
    };
    let Some(conv) = convs
        .iter()
        .find(|c| matches!(c.kind, ConvKind::SelfDevices))
    else {
        return false;
    };
    let msgs = match a
        .try_call(Request::ListMessages {
            conv: conv.conv.clone(),
            before: None,
            limit: 50,
        })
        .await
        .expect("消息查询失败")
    {
        Response::Messages { value } => value,
        other => panic!("期望 Messages，实际 {other:?}"),
    };
    !msgs.is_empty()
}

/// Sas 配对：双方 ShowSas code 相同；只有 A 确认时 2 秒内无 Done；B 确认后 Done。
#[tokio::test]
async fn sas_pairing_requires_both_confirmations() {
    init_log();
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    let mut b = node(&bus).await;
    a.init_first("甲机").await;
    init_join(&b).await;
    reannounce(&a).await;

    let started = match a
        .try_call(Request::PairStart {
            method: PairMethod::Sas,
        })
        .await
        .expect("PairStart 失败")
    {
        Response::PairSession { value } => value,
        other => panic!("期望 PairSession，实际 {other:?}"),
    };
    assert!(started.qr_uri.is_none(), "Sas 配对无二维码");

    let a_dev = a.device_id().await;
    // Sas/Device 目标下 B 本地生成会话 ID，双方事件与确认均以它为准。
    let session = join_device(&b, &a_dev).await;

    // 双方 ShowSas 且 code 相同。
    let code_a = wait_show_sas(&mut a, &session).await;
    let code_b = wait_show_sas(&mut b, &session).await;
    assert_eq!(code_a, code_b, "两端 SAS 应一致");
    assert_eq!(code_a.len(), 6);

    // 只有 A 确认：2 秒内双方无 Done。
    a.try_call(Request::PairConfirm {
        session: session.clone(),
        matches: true,
    })
    .await
    .expect("PairConfirm 失败");
    a.no_event(is_done(&session), 2000).await;
    b.no_event(is_done(&session), 2000).await;

    // B 也确认：双方 Done，B 已初始化且用户与 A 一致。
    b.try_call(Request::PairConfirm {
        session: session.clone(),
        matches: true,
    })
    .await
    .expect("PairConfirm 失败");
    a.wait_event(is_done(&session), 5).await;
    b.wait_event(is_done(&session), 5).await;
    match b.try_call(Request::GetSelf).await.expect("GetSelf 失败") {
        Response::SelfInfo { value } => {
            assert!(value.initialized);
            assert_eq!(value.user_id.as_deref(), Some(a.user_id().as_str()));
        }
        other => panic!("期望 SelfInfo，实际 {other:?}"),
    }
}

/// Sas 数字不一致：双方 Failed{"数字不一致"}，B 保持未初始化。
#[tokio::test]
async fn sas_mismatch_fails_both() {
    init_log();
    let bus = MemoryBus::new();
    let mut a = node(&bus).await;
    let mut b = node(&bus).await;
    a.init_first("甲机").await;
    init_join(&b).await;
    reannounce(&a).await;

    // A 开 Sas 配对窗口（窗口会话 ID 与 B 侧事件所用 ID 无关）。
    a.try_call(Request::PairStart {
        method: PairMethod::Sas,
    })
    .await
    .expect("PairStart 失败");
    let a_dev = a.device_id().await;
    let session = join_device(&b, &a_dev).await;
    let _ = wait_show_sas(&mut a, &session).await;
    let _ = wait_show_sas(&mut b, &session).await;

    // B 确认不一致：双方 Failed{"数字不一致"}。
    b.try_call(Request::PairConfirm {
        session: session.clone(),
        matches: false,
    })
    .await
    .expect("PairConfirm 失败");
    let fa = a.wait_event(is_failed(&session), 5).await;
    let fb = b.wait_event(is_failed(&session), 5).await;
    assert!(format!("{fa:?}").contains("数字不一致"));
    assert!(format!("{fb:?}").contains("数字不一致"));

    // B 保持未初始化。
    match b.try_call(Request::GetSelf).await.expect("GetSelf 失败") {
        Response::SelfInfo { value } => assert!(!value.initialized),
        other => panic!("期望 SelfInfo，实际 {other:?}"),
    }
}

/// Qr 口令错误：双方 Failed{"口令错误"}，B 未初始化且 keystore 无 user.key。
#[tokio::test]
async fn qr_pairing_with_wrong_secret_fails_and_leaks_nothing() {
    init_log();
    let mut a = node_offline().await;
    let mut b = node_offline().await;
    a.init_first("甲机").await;
    init_join(&b).await;

    let started = match a
        .try_call(Request::PairStart {
            method: PairMethod::Qr,
        })
        .await
        .expect("PairStart 失败")
    {
        Response::PairSession { value } => value,
        other => panic!("期望 PairSession，实际 {other:?}"),
    };
    let session = started.session;
    let uri = started.qr_uri.expect("Qr 配对应返回 URI");

    // 篡改 s 参数末字符（BASE32 大写字母，A/B 互换必然改变取值）。
    let pos = uri.rfind("&s=").expect("URI 应含 s 参数") + 3;
    let mut tampered = uri.clone();
    let last = tampered.as_bytes()[pos];
    let replacement = if last == b'A' { "B" } else { "A" };
    tampered.replace_range(pos..pos + 1, replacement);
    assert_ne!(uri, tampered);

    join_qr(&b, tampered).await;

    let fa = a.wait_event(is_failed(&session), 5).await;
    let fb = b.wait_event(is_failed(&session), 5).await;
    assert!(format!("{fa:?}").contains("口令错误"));
    assert!(format!("{fb:?}").contains("口令错误"));

    // B 保持未初始化，keystore 无 user.key。
    match b.try_call(Request::GetSelf).await.expect("GetSelf 失败") {
        Response::SelfInfo { value } => assert!(!value.initialized),
        other => panic!("期望 SelfInfo，实际 {other:?}"),
    }
    assert!(
        !b.dir.path().join("keys").join("user.key").exists(),
        "B 的 keystore 不应出现 user.key"
    );
}

/// 未以加入模式初始化直接 PairJoin → InvalidArgument。
#[tokio::test]
async fn join_requires_join_existing_init() {
    let a = node_offline().await;
    let b = node_offline().await;
    a.init_first("甲机").await;

    let started = match a
        .try_call(Request::PairStart {
            method: PairMethod::Qr,
        })
        .await
        .expect("PairStart 失败")
    {
        Response::PairSession { value } => value,
        other => panic!("期望 PairSession，实际 {other:?}"),
    };
    let uri = started.qr_uri.expect("Qr 配对应返回 URI");

    // B 未做 Init{JoinExisting}。
    let err = b
        .try_call(Request::PairJoin {
            target: PairTarget::QrUri { uri },
        })
        .await
        .expect_err("未初始化加入应被拒绝");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

/// 窗口未开启：B PairJoin Device 连接被拒或收到 Failed，B 保持未初始化。
#[tokio::test]
async fn pairing_window_closed_rejects_uninitialized_peer() {
    let bus = MemoryBus::new();
    let a = node(&bus).await;
    let mut b = node(&bus).await;
    a.init_first("甲机").await;
    init_join(&b).await;
    reannounce(&a).await;

    // A 未 PairStart：发现表就绪后，B 的连接应被 accept_filter 拒绝（join 报错）
    // 或 join 成功但随后收到 Failed。
    let a_dev = a.device_id().await;
    let mut rejected = false;
    for _ in 0..50 {
        match b
            .try_call(Request::PairJoin {
                target: PairTarget::Device {
                    device: a_dev.clone(),
                },
            })
            .await
        {
            Err(e) if e.code == ErrorCode::NotFound => {
                // 尚未发现 A，等通告。
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(_) => {
                rejected = true;
                break;
            }
            Ok(_) => {
                // join 返回 Ok 仍应很快收到 Failed（A 无窗口 → Abort）。
                let got_failed = tokio::time::timeout(Duration::from_secs(3), async {
                    loop {
                        match b.events.recv().await {
                            Ok(Event::PairStatus {
                                state: PairStateDto::Failed { .. },
                                ..
                            }) => return true,
                            Ok(_) => {}
                            Err(_) => return false,
                        }
                    }
                })
                .await
                .unwrap_or(false);
                rejected = got_failed;
                break;
            }
        }
    }
    assert!(rejected, "窗口未开时应被拒绝");
    match b.try_call(Request::GetSelf).await.expect("GetSelf 失败") {
        Response::SelfInfo { value } => assert!(!value.initialized),
        other => panic!("期望 SelfInfo，实际 {other:?}"),
    }
}

// ===== P2 Task 8：设备吊销 =====

/// A 移除 C：C 被断开且重连被拒；B 的设备列表不含 C；C 本机按未初始化处理。
#[tokio::test]
async fn removed_device_is_disconnected_and_refused() {
    init_log();
    let (mut a, mut b, mut c) = (
        node_offline().await,
        node_offline().await,
        node_offline().await,
    );
    a.init_first("甲机").await;
    init_join(&b).await;
    pair(&mut a, &mut b).await;
    init_join(&c).await;
    pair(&mut a, &mut c).await;
    let c_dev = c.device_id().await;

    // A 移除 C。
    a.call(Request::RemoveDevice {
        device: c_dev.clone(),
    })
    .await;

    // A 侧 C 被断开。
    a.wait_event(
        |e| matches!(e, Event::PeerOffline { device } if device == &c_dev),
        5,
    )
    .await;

    // B 的 ListMyDevices 不含 C。
    match b
        .try_call(Request::ListMyDevices)
        .await
        .expect("ListMyDevices 失败")
    {
        Response::Devices { value } => {
            assert!(
                !value.iter().any(|d| d.device_id == c_dev),
                "B 不应再列出 C"
            );
        }
        other => panic!("期望 Devices，实际 {other:?}"),
    }

    // C 重连 B 被拒：3 秒内 B 无 PeerOnline(C)。拨号因 accept_filter 拒绝
    // 而报错属预期，忽略。
    let b_addr = b.svc.local_addr().to_string();
    let _ = c.try_call(Request::AddManualPeer { addr: b_addr }).await;
    b.no_event(
        |e| matches!(e, Event::PeerOnline { peer } if peer.device_id == c_dev),
        3000,
    )
    .await;

    // C 本机按未初始化处理（GetSelf.initialized == false）。
    wait_until(
        || async {
            matches!(
                c.try_call(Request::GetSelf).await,
                Ok(Response::SelfInfo { value }) if !value.initialized
            )
        },
        5,
    )
    .await;
}

/// 不能移除本机（P2 Task 8）。
#[tokio::test]
async fn cannot_remove_self() {
    init_log();
    let a = node_offline().await;
    a.init_first("甲机").await;
    let a_dev = a.device_id().await;
    let err = a
        .try_call(Request::RemoveDevice { device: a_dev })
        .await
        .expect_err("移除本机应被拒绝");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}
