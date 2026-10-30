// P1 核心骨架：控制连接（Task 9）。
// 单条 TCP + TLS 控制连接：出站/入站建立、Hello 握手与校验、帧读写、Ping/Pong 心跳。
// 注释中文，标识符英文。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt, TryFutureExt};
use tokio::io::{split, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;
use tokio_util::codec::{FramedRead, FramedWrite};

use lanmsg_proto::codec::FrameCodec;
use lanmsg_proto::consts::{CHANNEL_CONTROL, CHANNEL_DATA, PAIR_TLS_EXPORTER_LABEL, PROTO_VERSION};
use lanmsg_proto::data::{read_data_frame, DataFrame};
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
    /// TLS 导出值（双方一致当且仅当无中间人）。
    pub tls_exporter: [u8; 32],
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
    /// 本机控制监听端口（ConnManager::bind 后覆盖），对端据此拨 dial_addr。
    pub listen_port: u16,
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
        let (_io, session) = tls_stream.get_ref();
        let peer_device = peer_device_id(session).map_err(|e| CoreError::Tls(e.to_string()))?;
        // TLS 导出值：双方一致当且仅当无中间人。
        let mut tls_exporter = [0u8; 32];
        session
            .export_keying_material(&mut tls_exporter, PAIR_TLS_EXPORTER_LABEL, None)
            .map_err(|e| CoreError::Tls(format!("导出密钥材料失败: {e}")))?;
        // 拨号方声明通道：控制通道。
        let mut tls_stream = tls_stream;
        tls_stream
            .write_all(&[CHANNEL_CONTROL])
            .await
            .map_err(CoreError::Io)?;

        let (read_half, write_half) = split(tls_stream);
        let mut framed_read = FramedRead::new(read_half, FrameCodec);
        let mut framed_write = FramedWrite::new(write_half, FrameCodec);

        // 发送自己的 Hello。
        let own_hello = Hello {
            proto: PROTO_VERSION,
            caps: vec!["sync/1".into(), "pair/1".into()],
            cert: hello.cert.clone(),
            user_display_name: hello.display_name.clone(),
            listen_port: hello.listen_port,
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
            tls_exporter,
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

    /// 入站连接分流：先读拨号方的通道字节，控制通道走 P1 Hello 流程，数据通道
    /// 校验 DataHello 后交给注册的处理器。
    pub(crate) async fn accept_channel(
        stream: TcpStream,
        device_key: &DeviceKey,
        self_device: DeviceId,
        hello: &HelloInfo,
        accept_filter: &(dyn Fn(&PeerInfo) -> bool + Send + Sync),
        event_tx: mpsc::Sender<ConnEvent>,
        data_handler: Option<DataHandler>,
    ) -> Result<Accepted, CoreError> {
        let peer_addr = stream.peer_addr().map_err(CoreError::Io)?;
        let (server_config, _client_config) = make_tls_configs(device_key)?;
        let acceptor = tokio_rustls::TlsAcceptor::from(server_config);
        let tls_stream = acceptor
            .accept(stream)
            .await
            .map_err(|e| CoreError::Tls(e.to_string()))?;
        // 先在 server::TlsStream 上取会话（可 deref 到 ConnectionCommon 提取导出值），
        // 再转换为统一的 TlsStream 供分流。
        let (_tcp, session) = tls_stream.get_ref();
        let peer_device = peer_device_id(session).map_err(|e| CoreError::Tls(e.to_string()))?;
        let mut tls_exporter = [0u8; 32];
        session
            .export_keying_material(&mut tls_exporter, PAIR_TLS_EXPORTER_LABEL, None)
            .map_err(|e| CoreError::Tls(format!("导出密钥材料失败: {e}")))?;
        let mut tls_stream = tokio_rustls::TlsStream::from(tls_stream);

        // 读取拨号方的通道字节并分流。
        let mut channel = [0u8; 1];
        tls_stream
            .read_exact(&mut channel)
            .await
            .map_err(CoreError::Io)?;
        match channel[0] {
            CHANNEL_CONTROL => {
                accept_control(
                    tls_stream,
                    peer_addr,
                    peer_device,
                    tls_exporter,
                    self_device,
                    hello,
                    accept_filter,
                    event_tx,
                )
                .await
            }
            CHANNEL_DATA => {
                match data_handler {
                    Some(h) => {
                        let peer_info = accept_data(
                            &mut tls_stream,
                            peer_addr,
                            peer_device,
                            tls_exporter,
                            accept_filter,
                        )
                        .await?;
                        h(peer_info.clone(), tls_stream);
                        Ok(Accepted::Data(peer_info))
                    }
                    // 未设置处理器：直接关闭。
                    None => Err(CoreError::Rejected("数据通道未开启".into())),
                }
            }
            other => Err(CoreError::Rejected(format!("未知通道字节 {other:#x}"))),
        }
    }
}

/// 控制通道入站：交换并校验 Hello → 启动读写任务（P1 流程，cert=None 的连接
/// 只放行 Pair/Ping/Pong/Error 帧）。
#[allow(clippy::too_many_arguments)] // P1 结构性：入站控制流集中入口。
async fn accept_control(
    tls_stream: DataStream,
    peer_addr: SocketAddr,
    peer_device: DeviceId,
    tls_exporter: [u8; 32],
    self_device: DeviceId,
    hello: &HelloInfo,
    accept_filter: &(dyn Fn(&PeerInfo) -> bool + Send + Sync),
    event_tx: mpsc::Sender<ConnEvent>,
) -> Result<Accepted, CoreError> {
    let (read_half, write_half) = split(tls_stream);
    let mut framed_read = FramedRead::new(read_half, FrameCodec);
    let mut framed_write = FramedWrite::new(write_half, FrameCodec);

    // 发送自己的 Hello。
    let own_hello = Hello {
        proto: PROTO_VERSION,
        caps: vec!["sync/1".into(), "pair/1".into()],
        cert: hello.cert.clone(),
        user_display_name: hello.display_name.clone(),
        listen_port: hello.listen_port,
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
        tls_exporter,
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
    // cert=None 的连接只放行 Pair/Ping/Pong/Error；其他帧回 Error 403 并丢弃。
    let uninit_peer = peer_info.cert.is_none();
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
                            if uninit_peer
                                && !matches!(
                                    frame,
                                    Frame::Pair(_) | Frame::Ping(_) | Frame::Pong(_) | Frame::Error { .. }
                                )
                            {
                                let _ = write_tx_for_pong
                                    .send(Frame::Error {
                                        code: 403,
                                        message: "对端未初始化，仅允许配对帧".into(),
                                    })
                                    .await;
                                continue;
                            }
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

    Ok(Accepted::Control(peer_info, connection))
}

impl Connection {
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

/// 数据连接处理器：接收已校验的对端身份与数据流。
pub type DataHandler = Arc<dyn Fn(PeerInfo, DataStream) + Send + Sync>;

/// 数据连接流。
pub type DataStream = tokio_rustls::TlsStream<TcpStream>;

/// 入站连接的分流结果。
pub(crate) enum Accepted {
    /// 控制通道：已建立的连接。
    Control(PeerInfo, Arc<Connection>),
    /// 数据通道：已交给处理器。
    Data(PeerInfo),
}

/// 数据通道入站：读首帧 DataHello 并校验身份与准入。校验通过后流的所有权
/// 仍归调用方，由其交给数据处理器。
async fn accept_data(
    tls_stream: &mut DataStream,
    peer_addr: SocketAddr,
    peer_device: DeviceId,
    tls_exporter: [u8; 32],
    accept_filter: &(dyn Fn(&PeerInfo) -> bool + Send + Sync),
) -> Result<PeerInfo, CoreError> {
    let frame = read_data_frame(tls_stream)
        .await
        .map_err(|e| CoreError::Tls(e.to_string()))?;
    let DataFrame::DataHello { cert } = frame else {
        return Err(CoreError::Proto("数据连接首帧必须是 DataHello".into()));
    };
    if peer_device != cert.device {
        return Err(CoreError::Rejected(
            "DataHello 证书与 TLS 对端不一致".into(),
        ));
    }
    cert.verify()
        .map_err(|e| CoreError::Rejected(e.to_string()))?;
    let peer_info = PeerInfo {
        device: cert.device,
        cert: Some(cert),
        display_name: String::new(),
        addr: peer_addr,
        dial_addr: peer_addr,
        path: PathClass::Lan,
        caps: vec![],
        outbound: false,
        tls_exporter,
    };
    if !accept_filter(&peer_info) {
        return Err(CoreError::Rejected("accept_filter 拒绝数据连接".into()));
    }
    Ok(peer_info)
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
            tls_exporter: [0u8; 32],
        };
        assert_eq!(info.device, device);
        assert!(info.outbound);
        assert_eq!(info.caps, vec!["sync/1"]);
    }
}
