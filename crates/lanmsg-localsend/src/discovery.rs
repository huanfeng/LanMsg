//! LocalSend 组播发现：收发 UDP 通告，并对 `announce: true` 的通告走 HTTP register 回复。

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::types::{LsAnnounce, LsInfo, LsPeer};
use crate::{LsError, LOCALSEND_MCAST};

/// 周期性通告间隔。
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30);

/// 同一 (ip, 指纹) 的 register 去重窗口。
const REGISTER_DEDUP: Duration = Duration::from_secs(30);

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

fn bind_socket(port: u16) -> Result<UdpSocket, LsError> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
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

async fn send_announce(sock: &UdpSocket, mcast_port: u16, me: &LsInfo, announce: bool) {
    let msg = LsAnnounce {
        info: me.clone(),
        announce,
    };
    let Ok(bytes) = serde_json::to_vec(&msg) else {
        return;
    };
    let dst = SocketAddr::V4(SocketAddrV4::new(LOCALSEND_MCAST, mcast_port));
    if let Err(e) = sock.send_to(&bytes, dst).await {
        tracing::debug!("localsend announce send failed: {e}");
    }
}

/// 启动发现。`mcast_port` 是组播 UDP 端口（默认 `LOCALSEND_PORT`），与 `me.port`（HTTP 端口，
/// 对端据此回连）相互独立；官方协议未规定两者必须相同。
/// `announce_enabled == false` 时只收不发，也不调用 register。
/// `events` 用 `try_send` 投递：消费方必须及时取走，通道满时事件会被丢弃（发现是周期性的，丢失可自愈）。
pub fn spawn(
    me: LsInfo,
    mcast_port: u16,
    announce_enabled: bool,
    events: mpsc::Sender<LsPeerEvent>,
) -> Result<LsDiscovery, LsError> {
    let sock = Arc::new(bind_socket(mcast_port)?);
    let task = tokio::spawn(async move {
        let client = match lenient_client() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("localsend http client build failed: {e}");
                return;
            }
        };
        // (ip, fingerprint) -> 上次发起 register 的时间，30 秒内不重复
        let mut recent: HashMap<(IpAddr, String), Instant> = HashMap::new();
        let mut tick = tokio::time::interval(ANNOUNCE_INTERVAL);
        let mut buf = vec![0u8; 4096];
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if announce_enabled {
                        send_announce(&sock, mcast_port, &me, true).await;
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
                    match events.try_send(LsPeerEvent::Seen(peer.clone())) {
                        Err(mpsc::error::TrySendError::Closed(_)) => return,
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            tracing::debug!("localsend peer event dropped: channel full");
                        }
                        Ok(()) => {}
                    }
                    let fresh = if wants_reply && announce_enabled {
                        let now = Instant::now();
                        recent.retain(|_, t| now.duration_since(*t) < REGISTER_DEDUP);
                        recent
                            .insert((peer.ip, peer.info.fingerprint.clone()), now)
                            .is_none()
                    } else {
                        false
                    };
                    if fresh {
                        let (client, me, sock, events) = (client.clone(), me.clone(), sock.clone(), events.clone());
                        tokio::spawn(async move {
                            match register_reply(&client, &me, &peer).await {
                                Ok(info) => {
                                    // 接收端已关闭说明发现被停用，忽略发送失败；满则丢弃（见 spawn 文档）
                                    let _ = events.try_send(LsPeerEvent::Seen(LsPeer { info, ip: peer.ip }));
                                }
                                Err(e) => {
                                    tracing::debug!("localsend register failed, replying via multicast: {e}");
                                    send_announce(&sock, mcast_port, &me, false).await;
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

/// 构造访问 LocalSend 对端用的 HTTP 客户端。
///
/// LocalSend 的 HTTPS 使用各设备自签证书，身份靠指纹在首次使用时记住（TOFU），
/// 而不是 CA 链，所以这里不校验证书链。
pub fn lenient_client() -> Result<reqwest::Client, LsError> {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| LsError::Tls(e.to_string()))?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(
        rustls::crypto::aws_lc_rs::default_provider(),
    )))
    .with_no_client_auth();
    reqwest::Client::builder()
        .use_preconfigured_tls(config)
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| LsError::Http(e.to_string()))
}

/// 接受任意服务端证书（签名仍按握手校验）。
#[derive(Debug)]
struct AcceptAnyCert(rustls::crypto::CryptoProvider);

impl rustls::client::danger::ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// 对端 register 接口的 URL，scheme 跟随对端通告的协议。
pub(crate) fn register_url(peer: &LsPeer) -> String {
    let scheme = match peer.info.protocol {
        crate::LsProtocol::Http => "http",
        crate::LsProtocol::Https => "https",
    };
    let addr = SocketAddr::new(peer.ip, peer.info.port);
    format!("{scheme}://{addr}/api/localsend/v2/register")
}

/// 向对端 `POST /api/localsend/v2/register`，返回对端信息。
pub async fn register_reply(
    client: &reqwest::Client,
    me: &LsInfo,
    peer: &LsPeer,
) -> Result<LsInfo, LsError> {
    let resp = client
        .post(register_url(peer))
        .timeout(Duration::from_secs(5))
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

    fn info(protocol: crate::LsProtocol, port: u16) -> LsInfo {
        LsInfo {
            alias: "peer".into(),
            version: "2.0".into(),
            device_model: None,
            device_type: None,
            fingerprint: "fp".into(),
            port,
            protocol,
            download: false,
        }
    }

    #[test]
    fn register_url_follows_protocol_and_port() {
        let p = LsPeer {
            info: info(crate::LsProtocol::Https, 12345),
            ip: "192.168.1.9".parse().unwrap(),
        };
        assert_eq!(
            register_url(&p),
            "https://192.168.1.9:12345/api/localsend/v2/register"
        );
        let p = LsPeer {
            info: info(crate::LsProtocol::Http, 53317),
            ip: "::1".parse().unwrap(),
        };
        assert_eq!(
            register_url(&p),
            "http://[::1]:53317/api/localsend/v2/register"
        );
    }

    /// 起一个只回一次固定响应的假 HTTP 服务，返回端口与收到的请求文本。
    async fn fake_server(response: String) -> (u16, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let h = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                let n = s.read(&mut tmp).await.unwrap();
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(pos) = text.find("\r\n\r\n") {
                    let len = text[..pos]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if buf.len() >= pos + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            s.write_all(response.as_bytes()).await.unwrap();
            s.shutdown().await.ok();
            String::from_utf8_lossy(&buf).to_string()
        });
        (port, h)
    }

    #[tokio::test]
    async fn register_reply_returns_peer_info_on_200() {
        let body = r#"{"alias":"remote","version":"2.0","fingerprint":"rfp","port":53317,"protocol":"http"}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let (port, h) = fake_server(resp).await;
        let peer = LsPeer {
            info: info(crate::LsProtocol::Http, port),
            ip: "127.0.0.1".parse().unwrap(),
        };
        let got = register_reply(
            &lenient_client().unwrap(),
            &info(crate::LsProtocol::Http, 1),
            &peer,
        )
        .await
        .unwrap();
        assert_eq!(got.alias, "remote");
        let req = h.await.unwrap();
        assert!(req.starts_with("POST /api/localsend/v2/register"));
        assert!(req.contains("\"alias\":\"peer\""));
    }

    #[tokio::test]
    async fn register_reply_maps_4xx_to_status_error() {
        let resp =
            "HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string();
        let (port, _h) = fake_server(resp).await;
        let peer = LsPeer {
            info: info(crate::LsProtocol::Http, port),
            ip: "127.0.0.1".parse().unwrap(),
        };
        let err = register_reply(
            &lenient_client().unwrap(),
            &info(crate::LsProtocol::Http, 1),
            &peer,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LsError::Status(403)));
    }

    async fn free_udp_port() -> u16 {
        let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        s.local_addr().unwrap().port()
    }

    /// 向发现 socket 单播一条 announce:true 的通告，通告里的 HTTP 端口指向假服务。
    async fn announce_to(mcast_port: u16, http_port: u16) {
        let msg = format!(
            r#"{{"alias":"p","version":"2.0","fingerprint":"peerfp","port":{http_port},"protocol":"http","announce":true}}"#
        );
        let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        s.send_to(msg.as_bytes(), ("127.0.0.1", mcast_port))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn announce_disabled_receives_but_never_registers() {
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_port = http.local_addr().unwrap().port();
        let mcast_port = free_udp_port().await;
        let (tx, mut rx) = mpsc::channel(8);
        let _d = spawn(info(crate::LsProtocol::Http, 1), mcast_port, false, tx).unwrap();
        announce_to(mcast_port, http_port).await;
        let LsPeerEvent::Seen(p) = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(p.info.fingerprint, "peerfp");
        // 只收不发：不应有任何 register 连接
        assert!(
            tokio::time::timeout(Duration::from_millis(500), http.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn announce_enabled_registers_once_within_dedup_window() {
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_port = http.local_addr().unwrap().port();
        let mcast_port = free_udp_port().await;
        let (tx, _rx) = mpsc::channel(8);
        let _d = spawn(info(crate::LsProtocol::Http, 1), mcast_port, true, tx).unwrap();
        announce_to(mcast_port, http_port).await;
        announce_to(mcast_port, http_port).await;
        let (_conn, _) = tokio::time::timeout(Duration::from_secs(2), http.accept())
            .await
            .unwrap()
            .unwrap();
        // 第二条通告在去重窗口内，不应再发起连接
        assert!(
            tokio::time::timeout(Duration::from_millis(500), http.accept())
                .await
                .is_err()
        );
    }
}
