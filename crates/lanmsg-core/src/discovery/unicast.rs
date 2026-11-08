// P5 单播通告目标：用户配置的跨网段设备地址（IP、IP:端口、IPv4 网段）。
// 规则与桌面端设置页的输入校验保持一致（见 lanmsg-desktop 的 parse_targets）。
// 注释中文，标识符英文。

use std::net::{IpAddr, SocketAddr};

use ipnet::{IpNet, Ipv4Net};
use lanmsg_proto::consts::DISCOVERY_PORT;

use crate::error::CoreError;

/// 单播通告网段的最小前缀长度（/24：最多 254 个主机地址）。
pub const UNICAST_MIN_PREFIX: u8 = 24;

/// 展开后目标总数上限。
pub const MAX_UNICAST_TARGETS: usize = 1024;

fn invalid(item: &str) -> CoreError {
    CoreError::InvalidArgument(format!("invalid unicast target: {item}"))
}

/// 解析并展开单播通告目标。
///
/// 每项可以是 `ip`、`ip:port`（IPv6 为 `[ip]:port`）或 IPv4 网段（前缀 ≥ /24）；
/// 无端口用 `DISCOVERY_PORT`；网段展开为全部主机地址（去掉网络号与广播地址）；
/// 任一项非法返回 `InvalidArgument`（消息含该项）。不接受主机名（需要 DNS 阻塞解析）。
pub fn parse_unicast_targets(items: &[String]) -> Result<Vec<SocketAddr>, CoreError> {
    let mut out: Vec<SocketAddr> = Vec::new();
    for raw in items {
        let item = raw.trim();
        if item.is_empty() {
            continue;
        }
        if let Ok(addr) = item.parse::<SocketAddr>() {
            if addr.port() == 0 {
                return Err(invalid(item));
            }
            out.push(addr);
        } else if let Ok(ip) = item.parse::<IpAddr>() {
            out.push(SocketAddr::new(ip, DISCOVERY_PORT));
        } else if let Ok(net) = item.parse::<IpNet>() {
            let IpNet::V4(net) = net else {
                return Err(invalid(item));
            };
            expand_v4(net, item, &mut out)?;
        } else {
            return Err(invalid(item));
        }
        // 先去重再比上限：重复项不占名额。
        out.sort();
        out.dedup();
        if out.len() > MAX_UNICAST_TARGETS {
            return Err(CoreError::InvalidArgument(
                "too many unicast targets".into(),
            ));
        }
    }
    // 过滤本机网卡地址：向自己单播没有意义。
    let local: std::collections::HashSet<IpAddr> = crate::net::path::cached_ifaces()
        .into_iter()
        .map(|i| i.addr)
        .collect();
    out.retain(|a| !local.contains(&a.ip()));
    Ok(out)
}

fn expand_v4(net: Ipv4Net, item: &str, out: &mut Vec<SocketAddr>) -> Result<(), CoreError> {
    if net.prefix_len() < UNICAST_MIN_PREFIX {
        return Err(invalid(item));
    }
    out.extend(
        net.hosts()
            .map(|ip| SocketAddr::new(IpAddr::V4(ip), DISCOVERY_PORT)),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parse_unicast_targets_expands_cidr_and_defaults_port() {
        let v = parse_unicast_targets(&s(&["10.1.2.3", "10.9.0.0/30"])).unwrap();
        assert_eq!(v.len(), 3); // 10.1.2.3 + 10.9.0.1、10.9.0.2
        assert!(v.iter().all(|a| a.port() == DISCOVERY_PORT));
        assert!(parse_unicast_targets(&s(&["10.0.0.0/16"])).is_err());
    }

    #[test]
    fn accepts_explicit_ports_ipv6_and_32() {
        let v = parse_unicast_targets(&s(&[
            "192.168.1.20:7000",
            "[fd00::1]:7001",
            "fd00::2",
            "10.0.0.5/32",
        ]))
        .unwrap();
        assert_eq!(v.len(), 4);
        assert!(v.contains(&"192.168.1.20:7000".parse().unwrap()));
        assert!(v.contains(&"[fd00::2]:53320".parse().unwrap()));
        assert!(v.contains(&"10.0.0.5:53320".parse().unwrap()));
    }

    #[test]
    fn rejects_bad_items() {
        for bad in [
            "example.com",
            "10.0.0.1:0",
            "10.0.0.1:99999",
            "fd00::/120",
            "10.0.0.0/23",
            "not-an-ip",
        ] {
            let err = parse_unicast_targets(&s(&[bad])).unwrap_err();
            assert!(matches!(err, CoreError::InvalidArgument(_)), "{bad}");
        }
        // 空项被忽略。
        assert!(parse_unicast_targets(&s(&["", "  "])).unwrap().is_empty());
    }

    #[test]
    fn duplicates_do_not_count_toward_the_cap() {
        let items: Vec<String> = (0..10).map(|_| "10.7.0.0/24".to_string()).collect();
        let v = parse_unicast_targets(&items).unwrap();
        assert_eq!(v.len(), 254);
    }

    #[test]
    fn total_expansion_is_capped() {
        let items: Vec<String> = (0..6).map(|i| format!("10.{i}.0.0/24")).collect();
        let err = parse_unicast_targets(&items).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)));
    }
}
