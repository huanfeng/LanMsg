// P1 核心骨架：路径等级判定（Task 8）。
// 注释中文，标识符英文。

use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use if_addrs::{get_if_addrs, IfAddr};
use ipnet::{IpNet, Ipv4Net, Ipv6Net};

use lanmsg_proto::types::PathClass;

/// 隧道（虚拟专用网）类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelKind {
    Tailscale,
    ZeroTier,
    WireGuard,
    Tun,
}

impl TunnelKind {
    /// 对外展示用的稳定字符串。
    pub fn as_str(&self) -> &'static str {
        match self {
            TunnelKind::Tailscale => "tailscale",
            TunnelKind::ZeroTier => "zerotier",
            TunnelKind::WireGuard => "wireguard",
            TunnelKind::Tun => "tun",
        }
    }
}

/// 本地网卡信息。
#[derive(Debug, Clone)]
pub struct LocalIface {
    pub name: String,
    pub addr: IpAddr,
    pub prefix: u8,
    /// 该网卡是否为隧道网卡。
    pub tunnel: Option<TunnelKind>,
}

/// 按网卡名与地址识别隧道类型，规则按顺序匹配。
pub fn detect_tunnel(name: &str, addr: IpAddr) -> Option<TunnelKind> {
    let n = name.to_lowercase();
    if n.contains("tailscale") {
        return Some(TunnelKind::Tailscale);
    }
    if (n.starts_with("zt") && n.len() > 2) || n.contains("zerotier") {
        return Some(TunnelKind::ZeroTier);
    }
    if n.starts_with("wg") || n.contains("wireguard") {
        return Some(TunnelKind::WireGuard);
    }
    if ["utun", "tun", "ppp", "tap"]
        .iter()
        .any(|p| n.starts_with(p))
    {
        // 100.64.0.0/10 是 Tailscale 使用的 CGNAT 段。
        if let IpAddr::V4(v4) = addr {
            let o = v4.octets();
            if o[0] == 100 && (64..=127).contains(&o[1]) {
                return Some(TunnelKind::Tailscale);
            }
        }
        return Some(TunnelKind::Tun);
    }
    None
}

/// 列出所有非回环本地网卡及其地址与前缀长度。
pub fn local_ifaces() -> Vec<LocalIface> {
    let mut out = Vec::new();
    if let Ok(ifaces) = get_if_addrs() {
        for iface in ifaces {
            match &iface.addr {
                IfAddr::V4(v4) => {
                    let addr = IpAddr::V4(v4.ip);
                    // 排除回环地址。
                    if addr.is_loopback() {
                        continue;
                    }
                    out.push(LocalIface {
                        name: iface.name.clone(),
                        addr,
                        prefix: v4.prefixlen,
                        tunnel: detect_tunnel(&iface.name, addr),
                    });
                }
                IfAddr::V6(v6) => {
                    let addr = IpAddr::V6(v6.ip);
                    // 排除回环地址。
                    if addr.is_loopback() {
                        continue;
                    }
                    out.push(LocalIface {
                        name: iface.name.clone(),
                        addr,
                        prefix: v6.prefixlen,
                        tunnel: detect_tunnel(&iface.name, addr),
                    });
                }
            }
        }
    }
    out
}

/// 判定路径等级。
pub fn classify(peer: IpAddr, ifaces: &[LocalIface], via_multicast: bool) -> PathClass {
    // 组播 → 局域网。
    if via_multicast {
        return PathClass::Lan;
    }

    // 回环 → 局域网。
    if peer.is_loopback() {
        return PathClass::Lan;
    }

    // 取与 peer 同网段且前缀最长的网卡（前缀相同时普通网卡优先）：
    // 该网卡是隧道 → VPN，否则 → 局域网。
    let best = ifaces
        .iter()
        .filter(|i| same_subnet(peer, i.addr, i.prefix))
        .max_by_key(|i| (i.prefix, i.tunnel.is_none()));
    if let Some(iface) = best {
        return if iface.tunnel.is_some() {
            PathClass::Vpn
        } else {
            PathClass::Lan
        };
    }

    // 枚举不到网卡（例如 Android 11+ 限制 netlink）时，私网地址按局域网处理，
    // 否则陌生人配对会全部被拒。
    if ifaces.is_empty() && is_private_lan_addr(peer) {
        return PathClass::Lan;
    }

    // 私有 IPv4 / CGNAT / IPv6 ULA / 链路本地 → VPN。
    if is_private_or_local(peer) {
        return PathClass::Vpn;
    }

    // 其余 → 互联网。
    PathClass::Internet
}

/// peer 是否为回环或与任一网卡（含隧道网卡）同网段。用于调节重连节奏，与路径等级无关。
pub fn on_local_subnet(peer: IpAddr, ifaces: &[LocalIface]) -> bool {
    peer.is_loopback() || ifaces.iter().any(|i| same_subnet(peer, i.addr, i.prefix))
}

/// 判断 peer 是否与 addr 在同一子网（prefix 长度内）。
fn same_subnet(peer: IpAddr, addr: IpAddr, prefix: u8) -> bool {
    match (peer, addr) {
        (IpAddr::V4(p), IpAddr::V4(a)) => {
            if prefix > 32 {
                return false;
            }
            let Ok(net) = Ipv4Net::new(a, prefix) else {
                return false;
            };
            let net: IpNet = net.into();
            net.contains(&IpAddr::V4(p))
        }
        (IpAddr::V6(p), IpAddr::V6(a)) => {
            if prefix > 128 {
                return false;
            }
            let Ok(net) = Ipv6Net::new(a, prefix) else {
                return false;
            };
            let net: IpNet = net.into();
            net.contains(&IpAddr::V6(p))
        }
        _ => false,
    }
}

/// 网卡枚举失败时用于回退为局域网的地址：RFC1918、ULA fd00::/8、回环、链路本地。
fn is_private_lan_addr(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xff00) == 0xfd00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// 网卡列表缓存的有效期；网络变化时由 `invalidate_ifaces_cache` 立即作废。
const IFACE_CACHE_TTL: Duration = Duration::from_secs(30);

static IFACE_CACHE: Mutex<Option<(Instant, Vec<LocalIface>)>> = Mutex::new(None);

/// 带短 TTL 缓存的网卡列表，供握手时判定路径，避免每次连接都枚举网卡。
pub fn cached_ifaces() -> Vec<LocalIface> {
    if let Ok(mut guard) = IFACE_CACHE.lock() {
        if let Some((at, list)) = guard.as_ref() {
            if at.elapsed() < IFACE_CACHE_TTL {
                return list.clone();
            }
        }
        let list = local_ifaces();
        *guard = Some((Instant::now(), list.clone()));
        return list;
    }
    local_ifaces()
}

/// 网络变化时作废缓存。
pub fn invalidate_ifaces_cache() {
    if let Ok(mut guard) = IFACE_CACHE.lock() {
        *guard = None;
    }
}

/// 判断地址是否属于私有或本地范围（应归为 VPN）。
fn is_private_or_local(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            // 10/8
            (v4.octets()[0] == 10)
            // 172.16/12
            || (v4.octets()[0] == 172 && v4.octets()[1] >= 16 && v4.octets()[1] <= 31)
            // 192.168/16
            || (v4.octets()[0] == 192 && v4.octets()[1] == 168)
            // CGNAT 100.64/10
            || (v4.octets()[0] == 100 && v4.octets()[1] >= 64 && v4.octets()[1] <= 127)
        }
        IpAddr::V6(v6) => {
            // IPv6 ULA fc00::/7
            (v6.segments()[0] & 0xfe00) == 0xfc00
            // IPv6 链路本地 fe80::/10
            || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_table() {
        let ifaces = vec![LocalIface {
            name: "eth0".into(),
            addr: "192.168.1.10".parse().unwrap(),
            prefix: 24,
            tunnel: None,
        }];

        assert_eq!(
            classify("192.168.1.20".parse().unwrap(), &ifaces, false),
            PathClass::Lan
        );
        assert_eq!(
            classify("10.8.0.2".parse().unwrap(), &ifaces, false),
            PathClass::Vpn
        );
        assert_eq!(
            classify("100.101.1.1".parse().unwrap(), &ifaces, false),
            PathClass::Vpn
        );
        assert_eq!(
            classify("8.8.8.8".parse().unwrap(), &ifaces, false),
            PathClass::Internet
        );
        assert_eq!(
            classify("8.8.8.8".parse().unwrap(), &ifaces, true),
            PathClass::Lan
        );
        assert_eq!(
            classify("127.0.0.1".parse().unwrap(), &ifaces, false),
            PathClass::Lan
        );
    }

    #[test]
    fn detect_tunnel_table() {
        let ip = |s: &str| -> IpAddr { s.parse().unwrap() };
        assert_eq!(
            detect_tunnel("tailscale0", ip("100.100.1.2")),
            Some(TunnelKind::Tailscale)
        );
        assert_eq!(
            detect_tunnel("utun4", ip("100.70.0.1")),
            Some(TunnelKind::Tailscale)
        );
        assert_eq!(
            detect_tunnel("utun4", ip("10.0.0.1")),
            Some(TunnelKind::Tun)
        );
        assert_eq!(
            detect_tunnel("ztabcdef12", ip("10.147.17.5")),
            Some(TunnelKind::ZeroTier)
        );
        assert_eq!(
            detect_tunnel("ZeroTier One [abc]", ip("10.147.17.5")),
            Some(TunnelKind::ZeroTier)
        );
        assert_eq!(
            detect_tunnel("wg0", ip("10.8.0.2")),
            Some(TunnelKind::WireGuard)
        );
        assert_eq!(detect_tunnel("eth0", ip("192.168.1.2")), None);
        assert_eq!(detect_tunnel("ppp0", ip("10.0.0.1")), Some(TunnelKind::Tun));
        assert_eq!(detect_tunnel("tap0", ip("10.0.0.1")), Some(TunnelKind::Tun));
        assert_eq!(
            detect_tunnel("Tailscale", ip("100.64.0.9")),
            Some(TunnelKind::Tailscale)
        );
        assert_eq!(detect_tunnel("zt", ip("10.0.0.1")), None);
    }

    #[test]
    fn same_subnet_over_tunnel_is_vpn() {
        let ifaces = vec![LocalIface {
            name: "tailscale0".into(),
            addr: "100.101.1.1".parse().unwrap(),
            prefix: 10,
            tunnel: Some(TunnelKind::Tailscale),
        }];
        assert_eq!(
            classify("100.90.2.2".parse().unwrap(), &ifaces, false),
            PathClass::Vpn
        );
        // 隧道网卡上的同网段地址仍被视为“本机网段”，用于重连节奏。
        assert!(on_local_subnet("100.90.2.2".parse().unwrap(), &ifaces));
        assert!(!on_local_subnet("8.8.8.8".parse().unwrap(), &ifaces));
    }

    #[test]
    fn plain_iface_wins_over_overlapping_tunnel() {
        let ifaces = vec![
            LocalIface {
                name: "wg0".into(),
                addr: "192.168.1.5".parse().unwrap(),
                prefix: 16,
                tunnel: Some(TunnelKind::WireGuard),
            },
            LocalIface {
                name: "eth0".into(),
                addr: "192.168.1.10".parse().unwrap(),
                prefix: 24,
                tunnel: None,
            },
        ];
        assert_eq!(
            classify("192.168.1.20".parse().unwrap(), &ifaces, false),
            PathClass::Lan
        );
    }

    #[test]
    fn longest_prefix_decides_between_overlapping_ifaces() {
        let ifaces = vec![
            LocalIface {
                name: "eth0".into(),
                addr: "192.168.1.10".parse().unwrap(),
                prefix: 16,
                tunnel: None,
            },
            LocalIface {
                name: "wg0".into(),
                addr: "192.168.1.5".parse().unwrap(),
                prefix: 24,
                tunnel: Some(TunnelKind::WireGuard),
            },
        ];
        assert_eq!(
            classify("192.168.1.20".parse().unwrap(), &ifaces, false),
            PathClass::Vpn
        );
        assert_eq!(
            classify("192.168.2.20".parse().unwrap(), &ifaces, false),
            PathClass::Lan
        );
    }

    #[test]
    fn empty_ifaces_private_addr_is_lan() {
        let none: Vec<LocalIface> = Vec::new();
        for a in [
            "10.1.2.3",
            "172.20.0.1",
            "192.168.5.5",
            "127.0.0.1",
            "fd12::1",
            "fe80::1",
        ] {
            assert_eq!(
                classify(a.parse().unwrap(), &none, false),
                PathClass::Lan,
                "{a}"
            );
        }
        // CGNAT 与公网在没有网卡信息时仍不是局域网。
        assert_eq!(
            classify("100.80.0.1".parse().unwrap(), &none, false),
            PathClass::Vpn
        );
        assert_eq!(
            classify("8.8.8.8".parse().unwrap(), &none, false),
            PathClass::Internet
        );
    }
}
