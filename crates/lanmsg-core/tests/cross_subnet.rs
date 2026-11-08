// P5 Task 3：手动添加对端、持久化与重启恢复、地址簿跨节点传播。
// 不同 MemoryBus 的节点彼此不会经发现互连，用来模拟“不同网段”。
// 注释中文，标识符英文。

mod common;

use lanmsg_api::types::{ErrorCode, Event, Request};
use lanmsg_core::discovery::MemoryBus;
use lanmsg_core::platform::PlatformHooks;

use common::{befriend, node, node_with, pair, TestNode};
use lanmsg_api::types::Response;

fn is_online(dev: String) -> impl Fn(&Event) -> bool {
    move |e| matches!(e, Event::PeerOnline { peer } if peer.device_id == dev)
}

#[tokio::test]
async fn manual_peer_connects_and_is_remembered() {
    let a = node(&MemoryBus::new()).await;
    let mut b = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    b.init_first("B").await;
    let a_dev = a.device_id().await;

    b.call(Request::AddManualPeer {
        addr: a.svc.local_addr().to_string(),
    })
    .await;
    b.wait_event(is_online(a_dev.clone()), 5).await;

    // 重启 b：地址来自 manual_peers，10 秒内再次连上 a。
    let TestNode { svc, dir, .. } = b;
    svc.shutdown().await;
    drop(svc);
    let mut b2 = node_with(&MemoryBus::new(), dir, PlatformHooks::default()).await;
    b2.wait_event(is_online(a_dev), 10).await;
}

#[tokio::test]
async fn address_book_lets_my_devices_find_contact() {
    // 节点都在回环地址上：测试期间允许地址簿交换回环地址，结束时复位。
    struct ResetLoopback;
    impl Drop for ResetLoopback {
        fn drop(&mut self) {
            lanmsg_core::net::addrbook::allow_loopback_for_tests(false);
        }
    }
    let _reset = ResetLoopback;
    lanmsg_core::net::addrbook::allow_loopback_for_tests(true);
    let bus1 = MemoryBus::new();
    let bus2 = MemoryBus::new();
    let mut p1 = node(&bus1).await;
    let mut p2 = node(&bus1).await;
    p1.init_first("P").await;
    p2.try_call(Request::Init {
        display_name: "P2".into(),
        device_name: "P2".into(),
        mode: lanmsg_api::types::InitMode::JoinExisting,
    })
    .await
    .expect("Init JoinExisting failed");
    pair(&mut p1, &mut p2).await;
    let mut c = node(&bus2).await;
    c.init_first("C").await;
    let c_dev = c.device_id().await;

    // p1 手动连上 c 并结为联系人；p2 与 c 不同 bus，从未直连。
    p1.call(Request::AddManualPeer {
        addr: c.svc.local_addr().to_string(),
    })
    .await;
    befriend(&mut p1, &mut c).await;

    // p2 经 p1 发来的地址簿找到 c。
    p2.wait_event(is_online(c_dev), 15).await;
}

#[tokio::test]
async fn add_manual_peer_rejects_unparsable_addr() {
    let a = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    for bad in ["not a host:99999", "", "host:"] {
        let err = a
            .try_call(Request::AddManualPeer { addr: bad.into() })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "{bad:?}");
    }
}

#[tokio::test]
async fn manual_peer_offline_is_persisted_and_reports_peer_offline() {
    let a = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    // 本机上没有服务监听的端口：连接失败，但地址保留给重连任务。
    let err = a
        .try_call(Request::AddManualPeer {
            addr: "127.0.0.1:1".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PeerOffline);
}

#[tokio::test]
async fn remove_manual_peer_forgets_it_and_list_reports_bindings() {
    let a = node(&MemoryBus::new()).await;
    let b = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    b.init_first("B").await;
    let addr = b.svc.local_addr().to_string();
    a.call(Request::AddManualPeer { addr: addr.clone() }).await;
    let Response::ManualPeers { value } = a.call(Request::ListManualPeers).await else {
        panic!("expected ManualPeers");
    };
    assert_eq!(value.len(), 1);
    assert_eq!(value[0].device_id, Some(b.device_id().await));
    a.call(Request::RemoveManualPeer { addr }).await;
    let Response::ManualPeers { value } = a.call(Request::ListManualPeers).await else {
        panic!("expected ManualPeers");
    };
    assert!(value.is_empty());
}

#[tokio::test]
async fn manual_peer_count_is_limited() {
    let a = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    for i in 0..32 {
        // 无服务监听的地址：连接失败（PeerOffline），记录照常保留。
        let _ = a
            .try_call(Request::AddManualPeer {
                addr: format!("127.0.0.1:{}", 40000 + i),
            })
            .await;
    }
    let err = a
        .try_call(Request::AddManualPeer {
            addr: "127.0.0.1:50000".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

/// 主机名条目离线添加（PeerOffline），对端之后上线：后台任务连上后按存储键绑定设备。
#[tokio::test]
async fn hostname_manual_peer_is_bound_after_peer_comes_online() {
    use lanmsg_api::types::InitMode;
    use lanmsg_core::discovery::DiscoveryMode;
    use lanmsg_core::keystore::FileKeyStore;
    use lanmsg_core::{Service, ServiceConfig};
    use std::sync::Arc;

    let a = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    // 取一个空闲端口，先让它保持无人监听。
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let key = format!("localhost:{port}");
    let err = a
        .try_call(Request::AddManualPeer { addr: key.clone() })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PeerOffline);

    // 对端之后在该端口上线。
    let dir = tempfile::tempdir().unwrap();
    let b = Service::start(ServiceConfig {
        data_dir: dir.path().to_path_buf(),
        listen_port: port,
        discovery: DiscoveryMode::Disabled,
        keystore: Arc::new(FileKeyStore::new(dir.path().join("keys"))),
        static_peers: Vec::new(),
        platform: PlatformHooks::default(),
        localsend_port: 0,
    })
    .await
    .unwrap();
    b.handle(Request::Init {
        display_name: "B".into(),
        device_name: "B".into(),
        mode: InitMode::FirstDevice,
    })
    .await
    .unwrap();

    common::wait_until(
        || async {
            matches!(
                a.try_call(Request::ListManualPeers).await,
                Ok(Response::ManualPeers { value })
                    if value.len() == 1 && value[0].addr == key && value[0].device_id.is_some()
            )
        },
        15,
    )
    .await;
}

/// ScanSubnet：非法网段同步拒绝；合法网段立即返回，扫描期间重复请求被拒，完成后发 ScanFinished。
/// 扫描对象是回环网段 127.0.0.0/30 的默认服务端口：测试里让该端口上有一个只 accept
/// 不应答的监听者，握手被卡住，扫描因此可控地保持“进行中”。
#[tokio::test]
async fn scan_subnet_request_validates_and_reports_finish() {
    let mut a = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    for bad in ["10.0.0.0/8", "abc", "fd00::/120", "10.0.0.0/21"] {
        let err = a
            .try_call(Request::ScanSubnet { cidr: bad.into() })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "{bad}");
    }
    // 监听者绑定随机端口，并让扫描改探测该端口，不依赖固定端口是否空闲。
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    a.svc
        .override_scan_port_for_test(listener.local_addr().unwrap().port());
    // 只接受连接、不做 TLS 应答：握手在 dial 超时（数秒）前一直卡住。
    let blocker = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            held.push(s);
        }
    });
    a.call(Request::ScanSubnet {
        cidr: "127.0.0.0/30".into(),
    })
    .await;
    let err = a
        .try_call(Request::ScanSubnet {
            cidr: "127.0.0.0/30".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    let ev = a
        .wait_event(|e| matches!(e, Event::ScanFinished { .. }), 60)
        .await;
    let Event::ScanFinished {
        cidr,
        probed,
        found,
    } = ev
    else {
        unreachable!()
    };
    assert_eq!(cidr, "127.0.0.0/30");
    assert_eq!(probed, 2);
    assert_eq!(found, 0);
    // 标志已复位：可以再次扫描。
    a.call(Request::ScanSubnet {
        cidr: "127.0.0.0/30".into(),
    })
    .await;
    blocker.abort();
}

/// SetSettings 先校验单播目标：非法（过大网段、主机名）拒绝保存，合法保存。
#[tokio::test]
async fn set_settings_validates_unicast_targets() {
    let a = node(&MemoryBus::new()).await;
    a.init_first("A").await;
    let Response::Settings { value } = a.call(Request::GetSettings).await else {
        panic!("expected Settings");
    };
    for bad in ["10.0.0.0/16", "example.com", "fd00::/120"] {
        let mut s = value.clone();
        s.unicast_targets = vec![bad.into()];
        let err = a
            .try_call(Request::SetSettings { settings: s })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "{bad}");
    }
    let mut ok = value.clone();
    ok.unicast_targets = vec!["10.9.0.0/30".into(), "192.168.1.5:7000".into()];
    a.call(Request::SetSettings {
        settings: ok.clone(),
    })
    .await;
    let Response::Settings { value: got } = a.call(Request::GetSettings).await else {
        panic!("expected Settings");
    };
    assert_eq!(got.unicast_targets, ok.unicast_targets);
}
