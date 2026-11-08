//! LocalSend 组播发现：收发 UDP 通告，并对 `announce: true` 的通告走 HTTP register 回复。

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::types::{LsAnnounce, LsInfo, LsPeer};
use crate::{LsError, LOCALSEND_MCAST, LOCALSEND_PORT};

/// 周期性通告间隔。
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30);

/// 发现事件。
#[derive(Debug, Clone)]
pub enum LsPeerEvent {
    Seen(LsPeer),
}

/// 发现任务句柄；丢弃时停止。
pub struct LsDiscovery {
    task: JoinHandle<()>,
}

impl Drop for LsDiscovery {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn bind_socket() -> Result<UdpSocket, LsError> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, LOCALSEND_PORT);
    sock.bind(&addr.into()).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            LsError::PortBusy
        } else {
            LsError::Io(e)
        }
    })?;
    sock.join_multicast_v4(&LOCALSEND_MCAST, &Ipv4Addr::UNSPECIFIED)?;
    sock.set_multicast_loop_v4(true)?;
    sock.set_nonblocking(true)?;
    Ok(UdpSocket::from_std(sock.into())?)
}

/// 解析一个数据报：非法 JSON 或自身指纹返回 `None`。
pub(crate) fn parse_datagram(
    buf: &[u8],
    src: IpAddr,
    my_fingerprint: &str,
) -> Option<(LsPeer, bool)> {
    let a: LsAnnounce = serde_json::from_slice(buf).ok()?;
    if a.info.fingerprint == my_fingerprint {
        return None;
    }
    Some((
        LsPeer {
            info: a.info,
            ip: src,
        },
        a.announce,
    ))
}

async fn send_announce(sock: &UdpSocket, me: &LsInfo, announce: bool) {
    let msg = LsAnnounce {
        info: me.clone(),
        announce,
    };
    let Ok(bytes) = serde_json::to_vec(&msg) else {
        return;
    };
    let dst = SocketAddr::V4(SocketAddrV4::new(LOCALSEND_MCAST, LOCALSEND_PORT));
    if let Err(e) = sock.send_to(&bytes, dst).await {
        tracing::debug!("localsend announce send failed: {e}");
    }
}

/// 启动发现。`announce_enabled == false` 时只收不发，也不调用 register。
pub fn spawn(
    me: LsInfo,
    announce_enabled: bool,
    events: mpsc::Sender<LsPeerEvent>,
) -> Result<LsDiscovery, LsError> {
    let sock = Arc::new(bind_socket()?);
    let task = tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("localsend http client build failed: {e}");
                return;
            }
        };
        let mut tick = tokio::time::interval(ANNOUNCE_INTERVAL);
        let mut buf = vec![0u8; 4096];
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if announce_enabled {
                        send_announce(&sock, &me, true).await;
                    }
                }
                r = sock.recv_from(&mut buf) => {
                    let (n, src) = match r {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::debug!("localsend recv failed: {e}");
                            continue;
                        }
                    };
                    let Some((peer, wants_reply)) = parse_datagram(&buf[..n], src.ip(), &me.fingerprint) else {
                        continue;
                    };
                    if events.send(LsPeerEvent::Seen(peer.clone())).await.is_err() {
                        return;
                    }
                    if wants_reply && announce_enabled {
                        let (client, me, sock, events) = (client.clone(), me.clone(), sock.clone(), events.clone());
                        tokio::spawn(async move {
                            match register_reply(&client, &me, &peer).await {
                                Ok(info) => {
                                    let _ = events.send(LsPeerEvent::Seen(LsPeer { info, ip: peer.ip })).await;
                                }
                                Err(e) => {
                                    tracing::debug!("localsend register failed, replying via multicast: {e}");
                                    send_announce(&sock, &me, false).await;
                                }
                            }
                        });
                    }
                }
            }
        }
    });
    Ok(LsDiscovery { task })
}

/// 向对端 `POST /api/localsend/v2/register`，返回对端信息。
pub async fn register_reply(
    client: &reqwest::Client,
    me: &LsInfo,
    peer: &LsPeer,
) -> Result<LsInfo, LsError> {
    let scheme = match peer.info.protocol {
        crate::LsProtocol::Http => "http",
        crate::LsProtocol::Https => "https",
    };
    let addr = SocketAddr::new(peer.ip, peer.info.port);
    let url = format!("{scheme}://{addr}/api/localsend/v2/register");
    let resp = client
        .post(url)
        .json(me)
        .send()
        .await
        .map_err(|e| LsError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(LsError::Status(resp.status().as_u16()));
    }
    resp.json::<LsInfo>()
        .await
        .map_err(|e| LsError::Json(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn announce_json(fp: &str, announce: bool) -> Vec<u8> {
        format!(
            r#"{{"alias":"a","version":"2.0","fingerprint":"{fp}","port":53317,"protocol":"http","announce":{announce}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn parse_datagram_ignores_own_fingerprint() {
        let ip: IpAddr = "192.168.1.5".parse().unwrap();
        assert!(parse_datagram(&announce_json("me", true), ip, "me").is_none());
    }

    #[test]
    fn parse_datagram_returns_peer_and_announce_flag() {
        let ip: IpAddr = "192.168.1.5".parse().unwrap();
        let (p, wants) = parse_datagram(&announce_json("other", true), ip, "me").unwrap();
        assert_eq!(p.ip, ip);
        assert!(wants);
        let (_, wants) = parse_datagram(&announce_json("other", false), ip, "me").unwrap();
        assert!(!wants);
    }

    #[test]
    fn parse_datagram_rejects_garbage() {
        let ip: IpAddr = "192.168.1.5".parse().unwrap();
        assert!(parse_datagram(b"not json", ip, "me").is_none());
    }
}
