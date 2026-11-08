// P1 核心骨架：控制连接（Task 9）。
// 单条 TCP + TLS 控制连接：出站/入站建立、Hello 握手与校验、帧读写、Ping/Pong 心跳。
// 保活：写循环在 heartbeat 内无其他出帧时发 Ping，读侧收到 Ping 回 Pong；读循环以
// idle_timeout 为期限等待任一帧，超时视为对端消失（无 FIN 的断电 / 切网 / 进程被杀），
// 与写失败一样关闭连接并走自清理 + Down。时序见 net::timing（生产值在 lanmsg-proto consts）。
// 注释中文，标识符英文。

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use tokio::io::{split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;
use tokio_util::codec::{FramedRead, FramedWrite};

use lanmsg_proto::codec::FrameCodec;
use lanmsg_proto::consts::{
    CHANNEL_CONTROL, CHANNEL_DATA, CHANNEL_DATA_DIAL_BACK, PAIR_TLS_EXPORTER_LABEL, PROTO_VERSION,
};
use lanmsg_proto::data::{read_data_frame, DataFrame};
use lanmsg_proto::frame::{Frame, Hello};
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::sign::DeviceKey;
use lanmsg_proto::types::{DeviceCert, PathClass};
use rustls::pki_types::ServerName;

use crate::error::CoreError;
use crate::net::timing::{self, NetTiming};
use crate::net::tls::{make_tls_configs, make_tls_configs_expecting, peer_device_id, TLS_SNI};

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

/// 连接实例 ID 分配器：读循环退出时按实例原子移除管理器表中"自己那条"，
/// 避免同设备重连裁决替换后误删存活的新连接（或残留死连接条目）。
static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

/// 读循环退出清理回调：移除管理器表中仍是该连接实例的条目，
/// 返回是否确有移除（即退出时该连接仍是当前登记连接）。
pub(crate) type ExitCleanup = Arc<dyn Fn(u64) -> BoxFuture<'static, bool> + Send + Sync>;

/// 收包时钟：记录建立时刻与最近一次收到字节的时刻（相对 born 的毫秒）。
/// 由 `ActivityReader` 在有字节到达时刷新，空闲超时与重连裁决都基于它。
#[derive(Clone)]
struct RecvClock {
    born: Instant,
    last_recv_ms: Arc<AtomicU64>,
}

impl RecvClock {
    fn new() -> Self {
        RecvClock {
            born: Instant::now(),
            last_recv_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    fn touch(&self) {
        self.last_recv_ms
            .store(self.born.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    /// 距最近一次收到字节的时长（尚未收到时从建立时刻算起）。
    fn idle(&self) -> Duration {
        let last = Duration::from_millis(self.last_recv_ms.load(Ordering::Relaxed));
        self.born.elapsed().saturating_sub(last)
    }
}

/// 读侧包装：每次读到字节就刷新收包时钟。空闲判定因此按“字节到达”而非“整帧到达”，
/// 慢链路上耗时超过空闲超时的大帧不会被误判断线。
struct ActivityReader<R> {
    inner: R,
    clock: RecvClock,
}

impl<R: AsyncRead + Unpin> AsyncRead for ActivityReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(poll, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.clock.touch();
        }
        poll
    }
}

/// 已建立并完成 Hello 校验的单条连接。
pub(crate) struct Connection {
    peer_info: PeerInfo,
    write_tx: mpsc::Sender<Frame>,
    up_sent: Arc<AtomicBool>,
    /// `set_up_sent` 时唤醒读循环（读循环在 Up 发出前不上送对端的帧）。
    up_notify: Arc<tokio::sync::Notify>,
    shutdown_tx: mpsc::Sender<()>,
    /// 连接实例 ID（同设备重连裁决后区分新旧连接）。
    conn_id: u64,
    /// 收包时钟，供重连裁决判断旧连接是否已死。
    clock: RecvClock,
}

impl Connection {
    /// 出站连接：TCP → TLS → 交换并校验 Hello → 启动读写任务。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn connect_outbound(
        addr: SocketAddr,
        expect: Option<DeviceId>,
        device_key: &DeviceKey,
        self_device: DeviceId,
        hello: &HelloInfo,
        accept_filter: &(dyn Fn(&PeerInfo) -> bool + Send + Sync),
        event_tx: mpsc::Sender<ConnEvent>,
        cleanup: ExitCleanup,
    ) -> Result<(PeerInfo, Arc<Connection>), CoreError> {
        let stream = TcpStream::connect(addr).await.map_err(CoreError::Io)?;
        let peer_addr = stream.peer_addr().map_err(CoreError::Io)?;
        // 带期望设备时，握手内（客户端证书发出前）就校验服务端身份。
        let (_server_config, client_config) = make_tls_configs_expecting(device_key, expect)?;
        let domain = ServerName::try_from(TLS_SNI.as_bytes())
            .map_err(|e| CoreError::Tls(format!("invalid SNI: {e}")))?;
        let connector = TlsConnector::from(client_config);
        let tls_stream = connector
            .connect(domain, stream)
            .await
            .map_err(|e| CoreError::Tls(e.to_string()))?;
        let (_io, session) = tls_stream.get_ref();
        let peer_device = peer_device_id(session).map_err(|e| CoreError::Tls(e.to_string()))?;
        // 第二道防线：TLS 层已按 expect 校验过；这里再核对一次。
        if expect.is_some_and(|e| e != peer_device) {
            return Err(CoreError::Rejected(
                "peer device does not match the expected device".into(),
            ));
        }
        // TLS 导出值：双方一致当且仅当无中间人。
        let mut tls_exporter = [0u8; 32];
        session
            .export_keying_material(&mut tls_exporter, PAIR_TLS_EXPORTER_LABEL, None)
            .map_err(|e| CoreError::Tls(format!("failed to export keying material: {e}")))?;
        // 拨号方声明通道：控制通道。
        let mut tls_stream = tls_stream;
        tls_stream
            .write_all(&[CHANNEL_CONTROL])
            .await
            .map_err(CoreError::Io)?;

        let (read_half, write_half) = split(tls_stream);
        let clock = RecvClock::new();
        let read_half = ActivityReader {
            inner: read_half,
            clock: clock.clone(),
        };
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
                    "peer refused connection: code={code} message={message}"
                )));
            }
            Some(Ok(other)) => {
                return Err(CoreError::Proto(format!(
                    "expected Hello frame, got type {}",
                    other.type_id()
                )));
            }
            Some(Err(e)) => return Err(CoreError::Io(std::io::Error::other(e.to_string()))),
            None => {
                return Err(CoreError::Proto(
                    "peer closed connection before sending Hello".into(),
                ))
            }
        };

        // 校验（按顺序）。
        if let Some(cert) = &their_hello.cert {
            if peer_device != cert.device {
                return Err(CoreError::Rejected(
                    "Hello device certificate does not match TLS peer".into(),
                ));
            }
            cert.verify()
                .map_err(|e| CoreError::Rejected(e.to_string()))?;
        }
        if their_hello.proto != PROTO_VERSION {
            return Err(CoreError::Rejected("protocol version mismatch".into()));
        }
        let their_device = their_hello
            .cert
            .as_ref()
            .map(|c| c.device)
            .unwrap_or(peer_device);
        if their_device == self_device {
            return Err(CoreError::Rejected("peer is self".into()));
        }
        let peer_info = PeerInfo {
            device: peer_device,
            cert: their_hello.cert.clone(),
            display_name: their_hello.user_display_name,
            addr: peer_addr,
            dial_addr: SocketAddr::new(peer_addr.ip(), their_hello.listen_port),
            path: crate::net::path::classify(
                peer_addr.ip(),
                &crate::net::path::cached_ifaces(),
                false,
            ),
            caps: their_hello.caps,
            outbound: true,
            tls_exporter,
        };

        // 与入站对称：出站连接同样经准入规则（出错一律拒绝），不符直接断开。
        if !accept_filter(&peer_info) {
            return Err(CoreError::Rejected("accept_filter rejected".into()));
        }

        // 校验通过后，等待对端确认（对端若拒绝会发 Error 帧）。
        let confirm = tokio::time::timeout(Duration::from_secs(5), framed_read.next())
            .await
            .map_err(|_| CoreError::Proto("timed out waiting for peer acknowledgement".into()))?;
        match confirm {
            Some(Ok(Frame::Error { code, message })) => {
                return Err(CoreError::Rejected(format!(
                    "peer refused connection: code={code} message={message}"
                )));
            }
            Some(Err(e)) => return Err(CoreError::Io(std::io::Error::other(e.to_string()))),
            None => {
                return Err(CoreError::Proto(
                    "peer closed connection before acknowledging".into(),
                ))
            }
            Some(Ok(_)) => {} // 对端接受，继续
        }

        let connection = start_io(
            peer_info.clone(),
            framed_read,
            framed_write,
            event_tx,
            cleanup,
            false,
            clock,
            timing::current(),
        );

        Ok((peer_info, connection))
    }

    /// 入站连接分流：先读拨号方的通道字节，控制通道走 P1 Hello 流程，数据通道
    /// 校验 DataHello 后交给注册的处理器。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn accept_channel(
        stream: TcpStream,
        device_key: &DeviceKey,
        self_device: DeviceId,
        hello: &HelloInfo,
        accept_filter: &(dyn Fn(&PeerInfo) -> bool + Send + Sync),
        event_tx: mpsc::Sender<ConnEvent>,
        data_handler: Option<DataHandler>,
        dial_back: &crate::net::dialback::DialBackTable,
        cleanup: ExitCleanup,
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
            .map_err(|e| CoreError::Tls(format!("failed to export keying material: {e}")))?;
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
                    cleanup,
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
                    None => Err(CoreError::Rejected("data channel not enabled".into())),
                }
            }
            // 反向建立的数据通道：DataHello 之后必须是 DialBack{token}，
            // 令牌须由同一设备登记过且未过期；数据流交给等待者，不交给数据处理器。
            CHANNEL_DATA_DIAL_BACK => {
                let peer_info = accept_data(
                    &mut tls_stream,
                    peer_addr,
                    peer_device,
                    tls_exporter,
                    accept_filter,
                )
                .await?;
                let frame = read_data_frame(&mut tls_stream)
                    .await
                    .map_err(|e| CoreError::Tls(e.to_string()))?;
                let DataFrame::DialBack { token } = frame else {
                    return Err(CoreError::Proto(
                        "DataHello on a dial-back channel must be followed by DialBack".into(),
                    ));
                };
                let Some(waiter) = dial_back.claim(&token, &peer_info.device) else {
                    tracing::warn!(peer = %peer_info.device, "unknown, expired or foreign dial-back token");
                    return Err(CoreError::Rejected("invalid dial-back token".into()));
                };
                // 等待者已放弃（超时）时流随之丢弃。
                let _ = waiter.send(tls_stream);
                Ok(Accepted::Data(peer_info))
            }
            other => Err(CoreError::Rejected(format!(
                "unknown channel byte {other:#x}"
            ))),
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
    cleanup: ExitCleanup,
) -> Result<Accepted, CoreError> {
    let (read_half, write_half) = split(tls_stream);
    let clock = RecvClock::new();
    let read_half = ActivityReader {
        inner: read_half,
        clock: clock.clone(),
    };
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
                "peer refused connection: code={code} message={message}"
            )));
        }
        Some(Ok(other)) => {
            return Err(CoreError::Proto(format!(
                "expected Hello frame, got type {}",
                other.type_id()
            )));
        }
        Some(Err(e)) => return Err(CoreError::Io(std::io::Error::other(e.to_string()))),
        None => {
            return Err(CoreError::Proto(
                "peer closed connection before sending Hello".into(),
            ))
        }
    };

    if let Some(cert) = &their_hello.cert {
        if peer_device != cert.device {
            return Err(CoreError::Rejected(
                "Hello device certificate does not match TLS peer".into(),
            ));
        }
        cert.verify()
            .map_err(|e| CoreError::Rejected(e.to_string()))?;
    }
    if their_hello.proto != PROTO_VERSION {
        return Err(CoreError::Rejected("protocol version mismatch".into()));
    }
    let their_device = their_hello
        .cert
        .as_ref()
        .map(|c| c.device)
        .unwrap_or(peer_device);
    if their_device == self_device {
        return Err(CoreError::Rejected("peer is self".into()));
    }
    let peer_info = PeerInfo {
        device: peer_device,
        cert: their_hello.cert.clone(),
        display_name: their_hello.user_display_name,
        addr: peer_addr,
        dial_addr: SocketAddr::new(peer_addr.ip(), their_hello.listen_port),
        path: crate::net::path::classify(peer_addr.ip(), &crate::net::path::cached_ifaces(), false),
        caps: their_hello.caps,
        outbound: false,
        tls_exporter,
    };
    if !accept_filter(&peer_info) {
        // 向对端发送 Error 帧并关闭连接。
        let _ = framed_write
            .send(Frame::Error {
                code: 99,
                message: "accept_filter rejected".into(),
            })
            .await;
        let _ = framed_write.flush().await;
        return Err(CoreError::Rejected("accept_filter rejected".into()));
    }

    // 向出站方发送确认帧（Ping），出站方收到非 Error 帧即视为接受。
    let _ = framed_write.send(Frame::Ping(0)).await;
    let _ = framed_write.flush().await;

    // cert=None 的连接只放行 Pair/Ping/Pong/Error；其他帧回 Error 403 并丢弃。
    let uninit_peer = peer_info.cert.is_none();
    let connection = start_io(
        peer_info.clone(),
        framed_read,
        framed_write,
        event_tx,
        cleanup,
        uninit_peer,
        clock,
        timing::current(),
    );

    Ok(Accepted::Control(peer_info, connection))
}

/// 启动读写任务并返回连接句柄（出站/入站共用）。
///
/// - 读循环：以 idle_timeout 等待任一帧（任何帧都算活跃）；Ping 回 Pong，其余上报；
///   超时 / 出错 / 被 shutdown 时退出，先自清理管理器表条目，再仅当自己仍是登记
///   连接时通知外层 Down（被替换的旧连接不发，避免对存活的新连接误报离线）。
/// - 写循环：发送队列中的帧；heartbeat 内无出帧则发 Ping；写失败调 shutdown 让读循环
///   一并退出，读循环退出后写循环随之结束。
#[allow(clippy::too_many_arguments)] // 出站/入站共用的结构性入口。
fn start_io<R, W>(
    peer_info: PeerInfo,
    framed_read: FramedRead<R, FrameCodec>,
    framed_write: FramedWrite<W, FrameCodec>,
    event_tx: mpsc::Sender<ConnEvent>,
    cleanup: ExitCleanup,
    uninit_peer: bool,
    clock: RecvClock,
    timing: NetTiming,
) -> Arc<Connection>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (write_tx, write_rx) = mpsc::channel(256);
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
    let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
    let connection = Arc::new(Connection {
        peer_info,
        write_tx,
        up_sent: Arc::new(AtomicBool::new(false)),
        up_notify: Arc::new(tokio::sync::Notify::new()),
        shutdown_tx,
        conn_id,
        clock: clock.clone(),
    });

    let write_tx_for_pong = connection.write_tx.clone();
    let shutdown_tx_for_write = connection.shutdown_tx.clone();
    let device_for_events = connection.peer_info.device;
    let up_sent_flag = connection.up_sent.clone();
    let up_notify = connection.up_notify.clone();
    // 读循环退出信号：写循环据此结束（避免对已死连接继续发心跳）。
    let (closed_tx, mut closed_rx) = tokio::sync::watch::channel(false);

    tokio::spawn(async move {
        let mut reader = framed_read;
        let mut shutdown_rx = shutdown_rx;
        // 连接结束原因，仅用于日志
        let mut reason = String::from("closed locally (shutdown, replaced, or write failure)");
        'read: loop {
            tokio::select! {
                result = next_frame_or_idle(&mut reader, &clock, timing.idle_timeout) => {
                    let Some(result) = result else {
                        reason = format!("idle timeout ({} s without any bytes)", timing.idle_timeout.as_secs());
                        break;
                    };
                    match result {
                        Some(Ok(Frame::Ping(_n))) => {
                            // try_send：写队列满时丢弃 Pong，避免读循环被写侧拖住而失去超时检测。
                            let _ = write_tx_for_pong.try_send(Frame::Pong(0));
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
                                        message: "peer not initialized, only pairing frames allowed".into(),
                                    })
                                    .await;
                                continue;
                            }
                            if !matches!(frame, Frame::Pong(_)) {
                                // 对端的帧不得先于本端 Up 上送：宽限窗口里先登记的一侧会立刻
                                // 发 CertUpdate / SyncOffer，路由层查不到尚未登记的连接就会
                                // 整帧丢弃。Up 发出前在此挂起（TCP 自带背压），关闭则退出。
                                while !up_sent_flag.load(Ordering::SeqCst) {
                                    tokio::select! {
                                        _ = up_notify.notified() => {}
                                        _ = shutdown_rx.recv() => break 'read,
                                    }
                                }
                                let _ = event_tx.send(ConnEvent::Frame(device_for_events, frame)).await;
                            }
                        }
                        Some(Err(e)) => {
                            reason = format!("read error: {e}");
                            break;
                        }
                        None => {
                            reason = "peer closed the connection".into();
                            break;
                        }
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }
        let _ = closed_tx.send(true);
        if up_sent_flag.load(Ordering::SeqCst) && cleanup(conn_id).await {
            tracing::info!(peer = %device_for_events, reason = %reason, "peer connection down");
            let _ = event_tx.send(ConnEvent::Down(device_for_events)).await;
        }
    });

    tokio::spawn(async move {
        let mut writer = framed_write;
        let mut write_rx = write_rx;
        let write_loop = async {
            loop {
                let frame = tokio::select! {
                    f = write_rx.recv() => match f {
                        Some(f) => f,
                        None => break,
                    },
                    // 每轮重建计时：任何出帧都推迟下一次 Ping。
                    _ = tokio::time::sleep(timing.heartbeat) => Frame::Ping(0),
                };
                if writer.send(frame).await.is_err() {
                    // 写失败：通知读循环一并退出，连接随即自清理。
                    let _ = shutdown_tx_for_write.try_send(());
                    return;
                }
            }
            let _ = writer.flush().await;
        };
        tokio::select! {
            _ = write_loop => {}
            _ = closed_rx.wait_for(|c| *c) => {}
        }
    });

    connection
}

/// 等待下一帧；距最近一次收到字节超过 idle_timeout 才算空闲（返回 None）。
/// 大帧还在持续到达时只续等，不判超时。
async fn next_frame_or_idle<R: AsyncRead + Unpin>(
    reader: &mut FramedRead<R, FrameCodec>,
    clock: &RecvClock,
    idle_timeout: Duration,
) -> Option<Option<Result<Frame, lanmsg_proto::frame::CodecError>>> {
    loop {
        let wait = idle_timeout.saturating_sub(clock.idle());
        match tokio::time::timeout(wait, reader.next()).await {
            Ok(result) => return Some(result),
            Err(_) if clock.idle() >= idle_timeout => return None,
            Err(_) => {}
        }
    }
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

    /// 标记 Up 已发出（须在 Up 事件入队之后调用），放行读循环挂起的帧。
    pub(crate) fn set_up_sent(&self) {
        self.up_sent.store(true, Ordering::SeqCst);
        self.up_notify.notify_one();
    }

    /// 连接实例 ID（管理器表登记与读循环退出自清理比对用）。
    pub(crate) fn conn_id(&self) -> u64 {
        self.conn_id
    }

    /// 距最近一次收到对端任意帧的时长（刚建立未收包时从建立时刻算起）。
    pub(crate) fn idle(&self) -> Duration {
        self.clock.idle()
    }

    /// 关闭连接：通知读循环退出。
    ///
    /// 用 try_send 非阻塞投递：调用方（connect/disconnect/shutdown）持管理器
    /// 写锁，若 await 发送而读循环恰好卡在退出自清理等写锁，会互相等死。
    /// 令牌已在通道时说明读循环即将退出，重复投递可忽略。
    pub(crate) fn shutdown(&self) {
        let _ = self.shutdown_tx.try_send(());
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
        return Err(CoreError::Proto(
            "first frame of a data connection must be DataHello".into(),
        ));
    };
    if peer_device != cert.device {
        return Err(CoreError::Rejected(
            "DataHello certificate does not match TLS peer".into(),
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
        return Err(CoreError::Rejected(
            "accept_filter rejected data connection".into(),
        ));
    }
    Ok(peer_info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// 任何写入都失败的写端：模拟对端已 RST 而本端读端仍无动静。
    struct FailWriter;

    impl AsyncWrite for FailWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Err(std::io::Error::other("写失败")))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// 写循环 send 失败须关闭连接：读端始终静默（空闲超时是 45 秒），
    /// 自清理仍应在 2 秒内触发并上报 Down。
    #[tokio::test]
    async fn write_failure_shuts_connection_down() {
        // _quiet_peer 保持存活，读端因此既不 EOF 也不出数据。
        let (quiet_local, _quiet_peer) = tokio::io::duplex(1024);
        let info = PeerInfo {
            device: DeviceId([9u8; 32]),
            cert: None,
            display_name: "写失败".into(),
            addr: "127.0.0.1:1".parse().unwrap(),
            dial_addr: "127.0.0.1:1".parse().unwrap(),
            path: lanmsg_proto::types::PathClass::Lan,
            caps: vec![],
            outbound: true,
            tls_exporter: [0u8; 32],
        };
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (cleaned_tx, mut cleaned_rx) = mpsc::channel(1);
        let cleanup: ExitCleanup = Arc::new(move |id| {
            let cleaned_tx = cleaned_tx.clone();
            Box::pin(async move {
                let _ = cleaned_tx.send(id).await;
                true
            })
        });
        let clock = RecvClock::new();
        let connection = start_io(
            info,
            FramedRead::new(
                ActivityReader {
                    inner: quiet_local,
                    clock: clock.clone(),
                },
                FrameCodec,
            ),
            FramedWrite::new(FailWriter, FrameCodec),
            event_tx,
            cleanup,
            false,
            clock,
            timing::current(),
        );
        connection.set_up_sent();
        connection
            .send(&DeviceId([9u8; 32]), Frame::Ping(1))
            .await
            .unwrap();

        let id = tokio::time::timeout(Duration::from_secs(2), cleaned_rx.recv())
            .await
            .expect("写失败后连接应在 2 秒内自清理")
            .unwrap();
        assert_eq!(id, connection.conn_id());
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), event_rx.recv()).await,
            Ok(Some(ConnEvent::Down(_)))
        ));
    }

    /// 慢链路上的大帧：整帧耗时远超空闲超时，但字节持续到达，不得被判断线；
    /// 帧到齐后照常上报，且无 Down。
    #[tokio::test]
    async fn slow_large_frame_is_not_idle_timeout() {
        let idle_timeout = Duration::from_millis(300);
        let timing = NetTiming {
            heartbeat: Duration::from_secs(60),
            idle_timeout,
            replace_idle: Duration::from_secs(60),
            dial: Duration::from_secs(10),
            accept_handshake: Duration::from_secs(10),
        };
        let (local, mut remote) = tokio::io::duplex(64 * 1024);
        let (local_read, local_write) = split(local);
        let info = PeerInfo {
            device: DeviceId([8u8; 32]),
            cert: None,
            display_name: "慢链路".into(),
            addr: "127.0.0.1:1".parse().unwrap(),
            dial_addr: "127.0.0.1:1".parse().unwrap(),
            path: lanmsg_proto::types::PathClass::Lan,
            caps: vec![],
            outbound: false,
            tls_exporter: [0u8; 32],
        };
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let cleanup: ExitCleanup = Arc::new(|_| Box::pin(async { true }));
        let clock = RecvClock::new();
        let connection = start_io(
            info,
            FramedRead::new(
                ActivityReader {
                    inner: local_read,
                    clock: clock.clone(),
                },
                FrameCodec,
            ),
            FramedWrite::new(local_write, FrameCodec),
            event_tx,
            cleanup,
            false,
            clock,
            timing,
        );
        connection.set_up_sent();

        let mut encoder = FramedWrite::new(Vec::new(), FrameCodec);
        encoder
            .send(Frame::Error {
                code: 1,
                message: "x".repeat(20_000),
            })
            .await
            .unwrap();
        let wire = encoder.into_inner();
        // 20 块 × 40ms ≈ 800ms，超过空闲超时 300ms 的两倍多；块间隔远小于空闲超时。
        let sender = tokio::spawn(async move {
            for chunk in wire.chunks(1_000) {
                remote.write_all(chunk).await.unwrap();
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            remote
        });

        let first = tokio::time::timeout(Duration::from_secs(3), event_rx.recv())
            .await
            .expect("大帧应在 3 秒内到齐");
        assert!(
            matches!(
                first,
                Some(ConnEvent::Frame(_, Frame::Error { code: 1, .. }))
            ),
            "慢大帧应被完整上报而非断线: {first:?}"
        );
        let _remote = sender.await.unwrap();
    }

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
