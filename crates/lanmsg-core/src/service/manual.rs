// 手动添加的对端：地址解析、规范化与后台重连任务。
// 条目记录原始输入（IP 或主机名，规范化为 host:port），启动恢复时重新解析 DNS。
// 注释中文，标识符英文。

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use lanmsg_proto::consts::{RETRY_MAX, RETRY_MIN, SERVICE_PORT};

use super::Inner;
use crate::error::CoreError;

/// 解析后的手动对端目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManualTarget {
    Ip(SocketAddr),
    Host(String, u16),
}

impl ManualTarget {
    /// 存储键：IP 用 `SocketAddr` 显示形式，主机名用小写 `host:port`。
    pub(crate) fn key(&self) -> String {
        match self {
            ManualTarget::Ip(a) => a.to_string(),
            ManualTarget::Host(h, p) => format!("{h}:{p}"),
        }
    }
}

fn invalid(input: &str) -> CoreError {
    CoreError::InvalidArgument(format!("cannot parse address: {input}"))
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

fn parse_port(s: &str) -> Option<u16> {
    s.parse::<u16>().ok().filter(|p| *p != 0)
}

/// 严格解析 `ip`、`ip:port`、`[ipv6]`、`[ipv6]:port`、`host`、`host:port`。
/// 方括号必须成对；不带方括号的 IPv6 只接受不带端口的纯地址。
pub(crate) fn parse_manual_target(input: &str) -> Result<ManualTarget, CoreError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(invalid(input));
    }
    if let Some(rest) = input.strip_prefix('[') {
        let (inner, tail) = rest.split_once(']').ok_or_else(|| invalid(input))?;
        let ip: std::net::Ipv6Addr = inner.parse().map_err(|_| invalid(input))?;
        let port = match tail {
            "" => SERVICE_PORT,
            t => parse_port(t.strip_prefix(':').ok_or_else(|| invalid(input))?)
                .ok_or_else(|| invalid(input))?,
        };
        return Ok(ManualTarget::Ip(SocketAddr::new(IpAddr::V6(ip), port)));
    }
    if input.contains(']') {
        return Err(invalid(input));
    }
    if let Ok(ip) = input.parse::<IpAddr>() {
        return Ok(ManualTarget::Ip(SocketAddr::new(ip, SERVICE_PORT)));
    }
    let (host, port) = match input.split_once(':') {
        Some((h, p)) => (h, parse_port(p).ok_or_else(|| invalid(input))?),
        None => (input, SERVICE_PORT),
    };
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ManualTarget::Ip(SocketAddr::new(ip, port)));
    }
    if !valid_host(host) {
        return Err(invalid(input));
    }
    Ok(ManualTarget::Host(host.to_ascii_lowercase(), port))
}

/// 把存储键解析成可拨号的地址（主机名走 DNS）。
pub(crate) async fn resolve(key: &str) -> Result<SocketAddr, CoreError> {
    if let Ok(a) = key.parse::<SocketAddr>() {
        return Ok(a);
    }
    tokio::net::lookup_host(key)
        .await
        .map_err(|_| CoreError::InvalidArgument(format!("cannot resolve address: {key}")))?
        .next()
        .ok_or_else(|| CoreError::InvalidArgument(format!("cannot resolve address: {key}")))
}

/// 启动某个手动对端的后台任务：先（重复）解析，再按退避持续重连；同一键只启动一次。
/// `connected`：调用方刚连上，先等一个周期再拨。
pub(crate) fn start(inner: &Arc<Inner>, key: String, connected: bool) {
    let mut tasks = inner.manual_tasks.lock().unwrap();
    if tasks.contains_key(&key) {
        return;
    }
    let task_inner = inner.clone();
    let task_key = key.clone();
    let handle = tokio::spawn(async move {
        let mut delay = RETRY_MIN;
        let addr = loop {
            match resolve(&task_key).await {
                Ok(a) => break a,
                Err(_) => {
                    if super::sleep_or_shutdown(&task_inner, delay).await {
                        return;
                    }
                    delay = (delay * 2).min(RETRY_MAX);
                }
            }
        };
        super::static_peer_task(task_inner, addr, connected).await;
    });
    tasks.insert(key, handle.abort_handle());
}

/// 停止某个手动对端的后台任务。
pub(crate) fn stop(inner: &Inner, key: &str) {
    if let Some(h) = inner.manual_tasks.lock().unwrap().remove(key) {
        h.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> String {
        parse_manual_target(s).unwrap().key()
    }

    #[test]
    fn parses_valid_forms() {
        assert_eq!(key("10.0.0.5"), "10.0.0.5:53320");
        assert_eq!(key("10.0.0.5:7000"), "10.0.0.5:7000");
        assert_eq!(key("::1"), "[::1]:53320");
        assert_eq!(key("[fd00::1]"), "[fd00::1]:53320");
        assert_eq!(key("[fd00::1]:7000"), "[fd00::1]:7000");
        assert_eq!(key("MyHost.Example"), "myhost.example:53320");
        assert_eq!(key(" nas.local:9000 "), "nas.local:9000");
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "not a host:99999",
            "host:",
            "host:0",
            "host:70000",
            "[::1",
            "::1]",
            "[::1]x",
            "[::1]:",
            "[::1]:0",
            "[nothex]:1",
            "fd00::zz",
            "a b",
            "host:1:2",
            "[10.0.0.1]:80",
        ] {
            assert!(parse_manual_target(bad).is_err(), "{bad:?}");
        }
    }
}
