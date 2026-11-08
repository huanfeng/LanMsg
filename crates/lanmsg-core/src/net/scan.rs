// P5 网段扫描：对一个小 IPv4 网段逐个探测监听端口，连得上的做完整握手。
// 注释中文，标识符英文。

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use ipnet::Ipv4Net;
use lanmsg_proto::consts::SCAN_MIN_PREFIX;
use lanmsg_proto::ids::DeviceId;
use tokio::sync::Semaphore;

use crate::error::CoreError;
use crate::net::ConnManager;

/// 并发探测数。
const SCAN_CONCURRENCY: usize = 64;

/// 每个地址的 TCP 探测超时。
const PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// 扫描结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    /// 实际探测的地址数（不含本机地址）。
    pub probed: u32,
    /// 握手成功的不同设备数。
    pub found: u32,
}

/// 校验扫描网段：必须是 IPv4，前缀不得小于 `SCAN_MIN_PREFIX`（/22）。
/// 非法输入立即返回 `InvalidArgument`，不会发出任何探测。
pub fn validate_scan_cidr(cidr: &str) -> Result<Ipv4Net, CoreError> {
    let net: Ipv4Net = cidr
        .trim()
        .parse()
        .map_err(|_| CoreError::InvalidArgument("invalid subnet format".into()))?;
    if net.prefix_len() < SCAN_MIN_PREFIX {
        return Err(CoreError::InvalidArgument(
            "only IPv4 subnets of /22 or smaller are supported".into(),
        ));
    }
    Ok(net)
}

/// 扫描 `net` 内所有主机地址的 `port`：先 TCP 探测（800ms），连得上再走完整的
/// TLS + Hello 握手。并发 64，跳过本机网卡地址。
pub async fn scan(conns: ConnManager, net: Ipv4Net, port: u16) -> ScanReport {
    let local: HashSet<IpAddr> = crate::net::path::local_ifaces()
        .into_iter()
        .map(|i| i.addr)
        .collect();
    let sem = Arc::new(Semaphore::new(SCAN_CONCURRENCY));
    // JoinSet：外层 future 被丢弃时自动 abort 全部探测任务。
    let mut tasks = tokio::task::JoinSet::new();
    let mut probed = 0u32;
    for ip in net.hosts() {
        let ip = IpAddr::V4(ip);
        if local.contains(&ip) {
            continue;
        }
        probed += 1;
        let addr = SocketAddr::new(ip, port);
        let conns = conns.clone();
        let sem = sem.clone();
        tasks.spawn(async move {
            let _permit = sem.acquire_owned().await.ok()?;
            let reachable =
                tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr))
                    .await
                    .is_ok_and(|r| r.is_ok());
            if !reachable {
                return None;
            }
            conns.connect(addr, false).await.ok()
        });
    }
    let mut found: HashSet<DeviceId> = HashSet::new();
    while let Some(res) = tasks.join_next().await {
        if let Ok(Some(device)) = res {
            found.insert(device);
        }
    }
    ScanReport {
        probed,
        found: found.len() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_rejects_too_large_or_invalid_cidr() {
        assert!(validate_scan_cidr("10.0.0.0/8").is_err());
        assert!(validate_scan_cidr("10.0.0.0/21").is_err());
        assert!(validate_scan_cidr("abc").is_err());
        assert!(validate_scan_cidr("fd00::/120").is_err());
        assert!(validate_scan_cidr("").is_err());
        assert_eq!(
            validate_scan_cidr("192.168.4.0/22").unwrap().prefix_len(),
            22
        );
    }

    /// 回环网段内唯一监听的节点被找到；两个回环地址指向同一台设备，只算一台。
    #[tokio::test]
    async fn scan_finds_peer_on_loopback_range() {
        use crate::identity::Identity;
        use crate::net::{ConnEvent, HelloInfo, PeerInfo};
        use std::sync::RwLock;
        use tokio::sync::{mpsc, watch};

        async fn manager() -> (ConnManager, mpsc::Receiver<ConnEvent>, DeviceId) {
            let (device, user) = (
                lanmsg_proto::sign::DeviceKey::generate(),
                lanmsg_proto::sign::UserKey::generate(),
            );
            let cert = lanmsg_proto::types::DeviceCert::issue(
                &user,
                device.device_id(),
                "scan".into(),
                lanmsg_proto::types::DeviceType::Desktop,
                1,
            );
            let id = device.device_id();
            let identity = Arc::new(RwLock::new(Identity {
                device,
                user: None,
                cert: Some(cert.clone()),
            }));
            let (hello_tx, hello_rx) = watch::channel(HelloInfo {
                display_name: "scan".into(),
                cert: Some(cert),
                listen_port: 0,
            });
            std::mem::forget(hello_tx);
            let (tx, rx) = mpsc::channel(64);
            let filter = Arc::new(|_: &PeerInfo| true);
            let m = ConnManager::bind(identity, 0, hello_rx, filter, tx)
                .await
                .unwrap();
            (m, rx, id)
        }

        let (a, mut a_events, a_dev) = manager().await;
        let (b, mut b_events, _b_dev) = manager().await;
        tokio::spawn(async move { while a_events.recv().await.is_some() {} });
        tokio::spawn(async move { while b_events.recv().await.is_some() {} });
        let port = a.local_addr().port();
        let net: Ipv4Net = "127.0.0.0/30".parse().unwrap();
        let report = scan(b.clone(), net, port).await;
        assert_eq!(report.probed, 2, "hosts of a /30");
        assert_eq!(report.found, 1);
        assert!(b.is_connected(&a_dev));
    }
}
