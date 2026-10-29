// P1 核心骨架：路径等级判定（Task 8）。
// 注释中文，标识符英文。

use std::net::IpAddr;

use if_addrs::{get_if_addrs, IfAddr};
use ipnet::{IpNet, Ipv4Net, Ipv6Net};

use lanmsg_proto::types::PathClass;

/// 本地网卡信息。
pub struct LocalIface {
    pub name: String,
    pub addr: IpAddr,
    pub prefix: u8,
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

    // peer 与任一网卡同网段 → 局域网。
    for iface in ifaces {
        if same_subnet(peer, iface.addr, iface.prefix) {
            return PathClass::Lan;
        }
    }

    // 私有 IPv4 / CGNAT / IPv6 ULA / 链路本地 → VPN。
    if is_private_or_local(peer) {
        return PathClass::Vpn;
    }

    // 其余 → 互联网。
    PathClass::Internet
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
}
