// P1 核心骨架：发现模式——UDP 组播（C6）。
// socket2 建双栈组播套接字（SO_REUSEADDR，Unix 加 SO_REUSEPORT），
// 对每个非回环网卡加入 DISCOVERY_V4 / DISCOVERY_V6 组并按网卡轮流发送；
// 每 IFACE_POLL_INTERVAL 重新枚举网卡，有变化重新加入组并立即通告。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Arc;

use socket2::{Domain, Protocol, SockRef, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch, Notify};

use lanmsg_proto::consts::{DISCOVERY_PORT, DISCOVERY_V4, DISCOVERY_V6, IFACE_POLL_INTERVAL};
use lanmsg_proto::sign::to_cbor;
use lanmsg_proto::types::Announce;

use super::OutPkt;

/// 单包接收缓冲区上限。
const RECV_BUF_LEN: usize = 65_536;

/// 组播收发套接字组：IPv4 必备，IPv6 可选（创建失败仅记日志，不影响 IPv4）。
pub(crate) struct McSockets {
    v4: Arc<UdpSocket>,
    v4_ifaces: Vec<Ipv4Addr>,
    v6: Option<Arc<UdpSocket>>,
    v6_ifaces: Vec<u32>,
}

/// 建一个非阻塞 UDP 套接字：SO_REUSEADDR，Unix 上加 SO_REUSEPORT。
fn new_sock(domain: Domain, bind_addr: SocketAddr) -> std::io::Result<Arc<UdpSocket>> {
    let sock = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    sock.bind(&bind_addr.into())?;
    sock.set_nonblocking(true)?;
    let std_sock: std::net::UdpSocket = sock.into();
    Ok(Arc::new(UdpSocket::from_std(std_sock)?))
}

/// 枚举非回环网卡：返回（IPv4 地址列表，IPv6 网卡 index 列表）。
fn enum_ifaces() -> (Vec<Ipv4Addr>, Vec<u32>) {
    let mut v4: Vec<Ipv4Addr> = Vec::new();
    let mut v6: Vec<u32> = Vec::new();
    let ifaces = match if_addrs::get_if_addrs() {
        Ok(list) => list,
        Err(e) => {
            tracing::debug!(error = %e, "枚举网卡失败");
            return (v4, v6);
        }
    };
    for iface in ifaces {
        if iface.is_loopback() {
            continue;
        }
        match iface.ip() {
            IpAddr::V4(ip) => v4.push(ip),
            IpAddr::V6(_) => {
                if let Some(index) = iface.index {
                    v6.push(index);
                }
            }
        }
    }
    v4.sort();
    v4.dedup();
    v6.sort();
    v6.dedup();
    (v4, v6)
}

impl McSockets {
    /// 打开 IPv4（必须成功）与 IPv6（失败仅记日志）组播套接字并按当前网卡加入组播组。
    pub(crate) fn open() -> std::io::Result<Self> {
        let v4 = new_sock(
            Domain::IPV4,
            SocketAddr::from(([0, 0, 0, 0], DISCOVERY_PORT)),
        )?;
        let v4_ref = SockRef::from(v4.as_ref());
        let (v4_ips, v6_idx) = enum_ifaces();
        for ip in &v4_ips {
            if let Err(e) = v4_ref.join_multicast_v4(&DISCOVERY_V4, ip) {
                tracing::debug!(error = %e, ip = %ip, "加入 IPv4 组播组失败");
            }
        }
        let (v6, joined) = match new_sock(
            Domain::IPV6,
            SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::UNSPECIFIED,
                DISCOVERY_PORT,
                0,
                0,
            )),
        ) {
            Ok(arc_sock) => {
                // 仅 IPv6：通过 socket2 套接字（尚未转为 tokio 套接字）设置只能 IPv6。
                let v6_ref = SockRef::from(arc_sock.as_ref());
                let _ = v6_ref.set_only_v6(true);
                let mut joined = Vec::new();
                for idx in &v6_idx {
                    match v6_ref.join_multicast_v6(&DISCOVERY_V6, *idx) {
                        Ok(()) => joined.push(*idx),
                        Err(e) => {
                            tracing::debug!(error = %e, index = *idx, "加入 IPv6 组播组失败");
                        }
                    }
                }
                (Some(arc_sock), joined)
            }
            Err(e) => {
                tracing::debug!(error = %e, "IPv6 组播套接字创建失败，仅用 IPv4");
                (None, Vec::new())
            }
        };
        Ok(McSockets {
            v4,
            v4_ifaces: v4_ips,
            v6,
            v6_ifaces: joined,
        })
    }

    /// 对新增网卡重新加入组播组。
    pub(crate) fn rejoin(&self, v4_ips: &[Ipv4Addr], v6_idx: &[u32]) {
        let v4_ref = SockRef::from(self.v4.as_ref());
        for ip in v4_ips {
            if !self.v4_ifaces.contains(ip) {
                if let Err(e) = v4_ref.join_multicast_v4(&DISCOVERY_V4, ip) {
                    tracing::debug!(error = %e, ip = %ip, "加入新 IPv4 网卡组播组失败");
                }
            }
        }
        if let Some(v6) = &self.v6 {
            let v6_ref = SockRef::from(v6.as_ref());
            for idx in v6_idx {
                if !self.v6_ifaces.contains(idx) {
                    match v6_ref.join_multicast_v6(&DISCOVERY_V6, *idx) {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::debug!(error = %e, index = *idx, "加入新 IPv6 网卡组播组失败");
                        }
                    }
                }
            }
        }
    }

    /// 组播泛洪：对每个网卡 set_multicast_if_* 后向组播地址发送一份。
    pub(crate) fn send_all(&self, bytes: &[u8]) -> std::io::Result<()> {
        let v4_ref = SockRef::from(self.v4.as_ref());
        let v4_target = SocketAddr::new(IpAddr::V4(DISCOVERY_V4), DISCOVERY_PORT);
        for ip in &self.v4_ifaces {
            v4_ref.set_multicast_if_v4(ip)?;
            let _ = self.v4.send_to(bytes, v4_target);
        }
        if let Some(v6) = &self.v6 {
            let v6_ref = SockRef::from(v6.as_ref());
            let v6_target = SocketAddr::new(IpAddr::V6(DISCOVERY_V6), DISCOVERY_PORT);
            for idx in &self.v6_ifaces {
                v6_ref.set_multicast_if_v6(*idx)?;
                let _ = v6.send_to(bytes, v6_target);
            }
        }
        Ok(())
    }

    /// 单播到指定地址：IPv4 目标用 IPv4 套接字，IPv6 目标用 IPv6 套接字（存在时）。
    pub(crate) fn send_unicast(&self, bytes: &[u8], addr: SocketAddr) -> std::io::Result<()> {
        match addr {
            SocketAddr::V4(_) => {
                let _ = self.v4.send_to(bytes, addr);
            }
            SocketAddr::V6(_) => {
                if let Some(v6) = &self.v6 {
                    let _ = v6.send_to(bytes, addr);
                }
            }
        }
        Ok(())
    }
}

/// 接收任务：循环读取套接字，将包发进收件箱。
async fn recv_loop(sock: Arc<UdpSocket>, inbox: mpsc::Sender<(Vec<u8>, SocketAddr)>) {
    let mut buf = [0u8; RECV_BUF_LEN];
    loop {
        match sock.recv_from(&mut buf).await {
            Ok((n, from)) => {
                if inbox.send((buf[..n].to_vec(), from)).await.is_err() {
                    break; // 收件箱关闭
                }
            }
            Err(e) => tracing::debug!(error = %e, "发现接收失败"),
        }
    }
}

/// 发送任务：循环读出要发的包，泛洪或单播。
async fn send_loop(sockets: Arc<McSockets>, mut outbox: mpsc::Receiver<OutPkt>) {
    while let Some(pkt) = outbox.recv().await {
        match pkt.to {
            None => {
                let _ = sockets.send_all(&pkt.bytes);
            }
            Some(addr) => {
                let _ = sockets.send_unicast(&pkt.bytes, addr);
            }
        }
    }
}

/// 发现主循环：接收远程包进收件箱；本地通告变化/周期/网卡变化时泛洪通告。
pub(crate) async fn run(
    mut announce_rx: watch::Receiver<Option<Announce>>,
    inbox: mpsc::Sender<(Vec<u8>, SocketAddr)>,
    outbox: mpsc::Receiver<OutPkt>,
    shutdown: Arc<Notify>,
) {
    let sockets = Arc::new(match McSockets::open() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "打开发现套接字失败");
            return;
        }
    });

    // 两个独立接收子任务（IPv4/IPv6），各自持有套接字 Arc 克隆。
    let inbox_clone = inbox.clone();
    tokio::spawn(recv_loop(Arc::clone(&sockets.v4), inbox));
    if let Some(v6) = &sockets.v6 {
        tokio::spawn(recv_loop(Arc::clone(v6), inbox_clone));
    }

    // 发送子任务。
    tokio::spawn(send_loop(Arc::clone(&sockets), outbox));

    let mut last_ifaces = enum_ifaces();
    let mut ticker = tokio::time::interval(IFACE_POLL_INTERVAL);
    loop {
        tokio::select! {
            _ = shutdown.notified() => break,
            changed = announce_rx.changed() => {
                if changed.is_err() { break; }
                if let Some(ann) = announce_rx.borrow().as_ref() {
                    let _ = sockets.send_all(&to_cbor(ann));
                }
            }
            _ = ticker.tick() => {
                let new_ifaces = enum_ifaces();
                if new_ifaces != last_ifaces {
                    sockets.rejoin(&new_ifaces.0, &new_ifaces.1);
                    last_ifaces = new_ifaces;
                }
                if let Some(ann) = announce_rx.borrow().as_ref() {
                    let _ = sockets.send_all(&to_cbor(ann));
                }
            }
        }
    }
}
