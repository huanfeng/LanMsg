// P1 核心骨架：控制连接（Task 9）。
// 单条 TCP + TLS 控制连接：出站/入站建立、Hello 握手与校验、帧读写、Ping/Pong 心跳。
// 注释中文，标识符英文。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt, TryFutureExt};
use tokio::io::split;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;
use tokio_util::codec::{FramedRead, FramedWrite};

use lanmsg_proto::codec::FrameCodec;
use lanmsg_proto::consts::PROTO_VERSION;
use lanmsg_proto::frame::{Frame, Hello};
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::sign::DeviceKey;
use lanmsg_proto::types::{DeviceCert, PathClass};
use rustls::pki_types::ServerName;

use crate::error::CoreError;
use crate::net::tls::{make_tls_configs, peer_device_id, TLS_SNI};

/// 对端连接信息。
#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub device: DeviceId,
    pub cert: Option<DeviceCert>,
    pub display_name: String,
    pub addr: SocketAddr,
    pub dial_addr: SocketAddr,
    pub path: PathClass,
    pub caps: Vec<String>,
    pub outbound: bool,
}

/// 连接事件。
#[derive(Debug)]
pub enum ConnEvent {
    Up(PeerInfo),
    Down(DeviceId),
    Frame(DeviceId, Frame),
}

/// Hello 握手参数（由 ConnManager 填入 listen_port）。
#[derive(Clone, Debug)]
pub struct HelloInfo {
    pub display_name: String,
    pub cert: Option<DeviceCert>,
}

/// 已建立并完成 Hello 校验的单条连接。
pub(crate) struct Connection {
    peer_info: PeerInfo,
    write_tx: mpsc::Sender<Frame>,
    up_sent: Arc<AtomicBool>,
    shutdown_tx: mpsc::Sender<()>,
}

impl Connection {
    /// 出站连接：TCP → TLS → 交换并校验 Hello → 启动读写任务。
    pub(crate) async fn connect_outbound(
        addr: SocketAddr,
        device_key: &DeviceKey,
        self_device: DeviceId,
        hello: &HelloInfo,
        event_tx: mpsc::Sender<ConnEvent>,
    ) -> Result<(PeerInfo, Arc<Connection>), CoreError> {
        let stream = TcpStream::connect(addr).await.map_err(CoreError::Io)?;
        let peer_addr = stream.peer_addr().map_err(CoreError::Io)?;
        let (_server_config, client_config) = make_tls_configs(device_key)?;
        let domain = ServerName::try_from(TLS_SNI.as_bytes())
            .map_err(|e| CoreError::Tls(format!("非法 SNI: {e}")))?;
        let connector = TlsConnector::from(client_config);
        let tls_stream = connector
            .connect(domain, stream)
            .await
            .map_err(|e| CoreError::Tls(e.to_string()))?;
        let (_io, tls_conn) = tls_stream.get_ref();
        let peer_device = peer_device_id(tls_conn).map_err(|e| CoreError::Tls(e.to_string()))?;

        let (read_half, write_half) = split(tls_stream);
        let mut framed_read = FramedRead::new(read_half, FrameCodec);
        let mut framed_write = FramedWrite::new(write_half, FrameCodec);

        // 发送自己的 Hello。
        let own_hello = Hello {
            proto: PROTO_VERSION,
            caps: vec!["sync/1".into(), "pair/1".into()],
            cert: hello.cert.clone(),
            user_display_name: hello.display_name.clone(),
            listen_port: 0,
        };
        framed_write
            .send(Frame::Hello(own_hello.clone()))
            .map_err(|e| CoreError::Io(std::io::Error::other(e.to_string())))
            .await?;
        framed_write
            .flush()
            .await
            .map_err(|e| CoreError::Io(std::io::Error::other(e.to_string())))?;

        // 读取对端 Hello。
        let their_hello = match framed_read.next().await {
            Some(Ok(Frame::Hello(h))) => h,
            Some(Ok(Frame::Error { code, message })) => {
                return Err(CoreError::Rejected(format!(
                    "对端拒绝连接: code={code} message={message}"
                )));
            }
            Some(Ok(other)) => {
                return Err(CoreError::Proto(format!(
                    "期望 Hello 帧，收到类型 {}",
                    other.type_id()
                )));
            }
            Some(Err(e)) => return Err(CoreError::Io(std::io::Error::other(e.to_string()))),
            None => return Err(CoreError::Proto("对端在发送 Hello 前关闭连接".into())),
        };

        // 校验（按顺序）。
        if let Some(cert) = &their_hello.cert {
            if peer_device != cert.device {
                return Err(CoreError::Rejected(
                    "Hello 设备证书与 TLS 对端不一致".into(),
                ));
            }
            cert.verify()
                .map_err(|e| CoreError::Rejected(e.to_string()))?;
        }
        if their_hello.proto != PROTO_VERSION {
            return Err(CoreError::Rejected("协议版本不符".into()));
        }
        let their_device = their_hello
            .cert
            .as_ref()
            .map(|c| c.device)
            .unwrap_or(peer_device);
        if their_device == self_device {
            return Err(CoreError::Rejected("对端即自己".into()));
        }
        // 校验通过后，等待对端确认（对端若拒绝会发 Error 帧）。
        let confirm = tokio::time::timeout(Duration::from_secs(5), framed_read.next())
            .await
            .map_err(|_| CoreError::Proto("等待对端确认超时".into()))?;
        match confirm {
            Some(Ok(Frame::Error { code, message })) => {
                return Err(CoreError::Rejected(format!(
                    "对端拒绝连接: code={code} message={message}"
                )));
            }
            Some(Err(e)) => return Err(CoreError::Io(std::io::Error::other(e.to_string()))),
            None => return Err(CoreError::Proto("对端在确认前关闭连接".into())),
            Some(Ok(_)) => {} // 对端接受，继续
        }

        let peer_info = PeerInfo {
            device: peer_device,
            cert: their_hello.cert.clone(),
            display_name: their_hello.user_display_name,
            addr: peer_addr,
            dial_addr: SocketAddr::new(peer_addr.ip(), their_hello.listen_port),
            path: PathClass::Lan,
            caps: their_hello.caps,
            outbound: true,
        };

        let (write_tx, write_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let connection = Arc::new(Connection {
            peer_info: peer_info.clone(),
            write_tx,
            up_sent: Arc::new(AtomicBool::new(false)),
            shutdown_tx,
        });

        let event_tx_for_read = event_tx.clone();
        let write_tx_for_pong = connection.write_tx.clone();
        let device_for_events = peer_info.device;

        // 读循环：解码帧，Ping 自动回 Pong，其余上报；连接断开或 shutdown 发 Down。
        tokio::spawn(async move {
            let mut reader = framed_read;
            let mut shutdown_rx = shutdown_rx;
            loop {
                tokio::select! {
                    result = reader.next() => {
                        match result {
                            Some(Ok(Frame::Ping(_n))) => {
                                let _ = write_tx_for_pong.send(Frame::Pong(0)).await;
                            }
                            Some(Ok(frame)) => {
                                let _ = event_tx_for_read.send(ConnEvent::Frame(device_for_events, frame)).await;
                            }
                            Some(Err(_)) | None => break,
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }
            }
            let _ = event_tx.send(ConnEvent::Down(device_for_events)).await;
        });

        // 写循环：发送队列中的帧。
        tokio::spawn(async move {
            let mut writer = framed_write;
            let mut write_rx = write_rx;
            while let Some(frame) = write_rx.recv().await {
                if writer.send(frame).await.is_err() {
                    break;
                }
            }
            let _ = writer.flush().await;
        });

        Ok((peer_info, connection))
    }

    /// 入站连接：TLS 接受后交换并校验 Hello → 启动读写任务。
    pub(crate) async fn accept_inbound(
        stream: TcpStream,
        device_key: &DeviceKey,
        self_device: DeviceId,
        hello: &HelloInfo,
        accept_filter: &(dyn Fn(&PeerInfo) -> bool + Send + Sync),
        event_tx: mpsc::Sender<ConnEvent>,
    ) -> Result<(PeerInfo, Arc<Connection>), CoreError> {
        let peer_addr = stream.peer_addr().map_err(CoreError::Io)?;
        let (server_config, _client_config) = make_tls_configs(device_key)?;
        let acceptor = tokio_rustls::TlsAcceptor::from(server_config);
        let tls_stream = acceptor
            .accept(stream)
            .await
            .map_err(|e| CoreError::Tls(e.to_string()))?;
        let (_tcp_stream, tls_conn) = tls_stream.get_ref();
        let peer_device = peer_device_id(tls_conn).map_err(|e| CoreError::Tls(e.to_string()))?;

        let (read_half, write_half) = split(tls_stream);
        let mut framed_read = FramedRead::new(read_half, FrameCodec);
        let mut framed_write = FramedWrite::new(write_half, FrameCodec);

        // 发送自己的 Hello。
        let own_hello = Hello {
            proto: PROTO_VERSION,
            caps: vec!["sync/1".into(), "pair/1".into()],
            cert: hello.cert.clone(),
            user_display_name: hello.display_name.clone(),
            listen_port: 0,
        };
        framed_write
            .send(Frame::Hello(own_hello.clone()))
            .map_err(|e| CoreError::Io(std::io::Error::other(e.to_string())))
            .await?;
        framed_write
            .flush()
            .await
            .map_err(|e| CoreError::Io(std::io::Error::other(e.to_string())))?;

        // 读取对端 Hello。
        let their_hello = match framed_read.next().await {
            Some(Ok(Frame::Hello(h))) => h,
            Some(Ok(Frame::Error { code, message })) => {
                return Err(CoreError::Rejected(format!(
                    "对端拒绝连接: code={code} message={message}"
                )));
            }
            Some(Ok(other)) => {
                return Err(CoreError::Proto(format!(
                    "期望 Hello 帧，收到类型 {}",
                    other.type_id()
                )));
            }
            Some(Err(e)) => return Err(CoreError::Io(std::io::Error::other(e.to_string()))),
            None => return Err(CoreError::Proto("对端在发送 Hello 前关闭连接".into())),
        };

        if let Some(cert) = &their_hello.cert {
            if peer_device != cert.device {
                return Err(CoreError::Rejected(
                    "Hello 设备证书与 TLS 对端不一致".into(),
                ));
            }
            cert.verify()
                .map_err(|e| CoreError::Rejected(e.to_string()))?;
        }
        if their_hello.proto != PROTO_VERSION {
            return Err(CoreError::Rejected("协议版本不符".into()));
        }
        let their_device = their_hello
            .cert
            .as_ref()
            .map(|c| c.device)
            .unwrap_or(peer_device);
        if their_device == self_device {
            return Err(CoreError::Rejected("对端即自己".into()));
        }
        let peer_info = PeerInfo {
            device: peer_device,
            cert: their_hello.cert.clone(),
            display_name: their_hello.user_display_name,
            addr: peer_addr,
            dial_addr: SocketAddr::new(peer_addr.ip(), their_hello.listen_port),
            path: PathClass::Lan,
            caps: their_hello.caps,
            outbound: false,
        };
        if !accept_filter(&peer_info) {
            // 向对端发送 Error 帧并关闭连接。
            let _ = framed_write
                .send(Frame::Error {
                    code: 99,
                    message: "accept_filter 拒绝".into(),
                })
                .await;
            let _ = framed_write.flush().await;
            return Err(CoreError::Rejected("accept_filter 拒绝".into()));
        }

        // 向出站方发送确认帧（Ping），出站方收到非 Error 帧即视为接受。
        let _ = framed_write.send(Frame::Ping(0)).await;
        let _ = framed_write.flush().await;

        let (write_tx, write_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let connection = Arc::new(Connection {
            peer_info: peer_info.clone(),
            write_tx,
            up_sent: Arc::new(AtomicBool::new(false)),
            shutdown_tx,
        });

        let write_tx_for_pong = connection.write_tx.clone();
        let device_for_events = peer_info.device;
        let event_tx_for_read = event_tx.clone();
        tokio::spawn(async move {
            let mut reader = framed_read;
            let mut shutdown_rx = shutdown_rx;
            loop {
                tokio::select! {
                    result = reader.next() => {
                        match result {
                            Some(Ok(Frame::Ping(_n))) => {
                                let _ = write_tx_for_pong.send(Frame::Pong(0)).await;
                            }
                            Some(Ok(frame)) => {
                                let _ = event_tx_for_read.send(ConnEvent::Frame(device_for_events, frame)).await;
                            }
                            Some(Err(_)) | None => break,
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }
            }
            let _ = event_tx.send(ConnEvent::Down(device_for_events)).await;
        });
        tokio::spawn(async move {
            let mut writer = framed_write;
            let mut write_rx = write_rx;
            while let Some(frame) = write_rx.recv().await {
                if writer.send(frame).await.is_err() {
                    break;
                }
            }
            let _ = writer.flush().await;
        });

        Ok((peer_info, connection))
    }

    /// 发送一帧到对端。
    pub(crate) async fn send(&self, _device: &DeviceId, frame: Frame) -> Result<(), CoreError> {
        self.write_tx
            .send(frame)
            .await
            .map_err(|_| CoreError::PeerOffline)
    }

    pub(crate) fn peer_info(&self) -> &PeerInfo {
        &self.peer_info
    }

    pub(crate) fn set_up_sent(&self) {
        self.up_sent.store(true, Ordering::SeqCst);
    }

    /// 关闭连接：通知读循环退出并发送 Down 事件。
    pub(crate) async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_info_fields() {
        let device = DeviceId([1u8; 32]);
        let info = PeerInfo {
            device,
            cert: None,
            display_name: "测试".into(),
            addr: "127.0.0.1:53320".parse().unwrap(),
            dial_addr: "127.0.0.1:53320".parse().unwrap(),
            path: lanmsg_proto::types::PathClass::Lan,
            caps: vec!["sync/1".into()],
            outbound: true,
        };
        assert_eq!(info.device, device);
        assert!(info.outbound);
        assert_eq!(info.caps, vec!["sync/1"]);
    }
}
