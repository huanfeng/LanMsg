// P1 核心骨架：连接管理器（Task 9）。
// 维护每对设备一条控制连接：监听入站、出站拨号、Hello 校验、重复连接裁决、帧事件分发。
// 注释中文，标识符英文。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};

use crate::net::conn::{
    Accepted, ConnEvent, Connection, DataHandler, DataStream, ExitCleanup, HelloInfo, PeerInfo,
};
use crate::net::dialback::{DialBackTable, DIAL_BACK_WAIT};
use crate::net::timing;
use crate::net::tls::{make_tls_configs_expecting, TLS_SNI};
use lanmsg_proto::consts::{CHANNEL_DATA, CHANNEL_DATA_DIAL_BACK, DUP_DIAL_GRACE_MS};
use lanmsg_proto::data::{write_data_frame, DataFrame};
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::sign::DeviceKey;
use lanmsg_proto::types::DeviceCert;
use rustls::pki_types::ServerName;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::error::CoreError;
use crate::identity::Identity;

/// 连接条目：持连接句柄与裁决所需元数据。
struct ConnectionEntry {
    connection: Arc<Connection>,
    peer_addr: SocketAddr,
    /// 发起此连接的设备 ID（出站为 self，入站为 peer），用于重复连接裁决。
    dialer_device: DeviceId,
    up_sent: bool,
    /// 连接实例 ID（与 Connection::conn_id 一致，读循环退出自清理时比对）。
    conn_id: u64,
}

/// 数据通道直拨超时：超过则改为请求对端拨回。
const DIRECT_DIAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// 拨回时的连接超时。
const DIAL_BACK_DIAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 控制连接管理器（Clone，内部 Arc）。
#[derive(Clone)]
pub struct ConnManager {
    local_addr: SocketAddr,
    inner: Arc<RwLock<Inner>>,
    /// 接受循环任务句柄：shutdown 时中止，真正关闭监听端口。
    accept_task: Arc<std::sync::Mutex<Option<tokio::task::AbortHandle>>>,
    /// 关停信号（shutdown 置 true）：后台重连任务据此打断退避睡眠与进行中的拨号。
    closed_tx: Arc<watch::Sender<bool>>,
}

struct Inner {
    connections: HashMap<DeviceId, ConnectionEntry>,
    events: mpsc::Sender<ConnEvent>,
    hello_rx: watch::Receiver<HelloInfo>,
    accept_filter: Arc<dyn Fn(&PeerInfo) -> bool + Send + Sync>,
    data_handler: Option<DataHandler>,
    self_device: DeviceId,
    device_key: Arc<DeviceKey>,
    /// 对端证书覆盖（收到 CertUpdate 后刷新；连接重建或断开时清除，以新握手为准）。
    cert_overrides: std::sync::Mutex<HashMap<DeviceId, DeviceCert>>,
    /// 已 shutdown：握手中的连接此后到达不得再登记（否则成为关不掉的僵尸连接）。
    closed: bool,
    /// 反向数据通道的等待者表。
    dial_back: Arc<DialBackTable>,
    /// 测试钩子：覆盖对端的 dial_addr（模拟对端在 NAT 后、直拨不通）。
    #[cfg(feature = "test-hooks")]
    dial_addr_overrides: std::sync::Mutex<HashMap<DeviceId, SocketAddr>>,
}

impl ConnManager {
    /// 在 `0.0.0.0:port` 监听（port 被占用且非 0 时回退 0 并 `warn!`）。
    pub async fn bind(
        identity: Arc<RwLock<Identity>>,
        port: u16,
        hello: watch::Receiver<HelloInfo>,
        accept_filter: Arc<dyn Fn(&PeerInfo) -> bool + Send + Sync>,
        events: mpsc::Sender<ConnEvent>,
    ) -> Result<ConnManager, CoreError> {
        // 毒化锁说明持锁线程已 panic，沿用仓库 lock().unwrap() 风格直接传播。
        let (device_key, self_device) = {
            let identity_guard = identity.read().unwrap();
            (
                DeviceKey::from_bytes(&identity_guard.device.to_bytes()),
                identity_guard.device_id(),
            )
        };

        let listener = if port == 0 {
            TcpListener::bind("0.0.0.0:0")
                .await
                .map_err(CoreError::Io)?
        } else {
            match TcpListener::bind(format!("0.0.0.0:{port}")).await {
                Ok(l) => l,
                Err(_) => {
                    tracing::warn!(port, "port in use, falling back to a random port");
                    TcpListener::bind("0.0.0.0:0")
                        .await
                        .map_err(CoreError::Io)?
                }
            }
        };
        let local_addr = listener.local_addr().map_err(CoreError::Io)?;

        let inner = Inner {
            connections: HashMap::new(),
            events,
            hello_rx: hello,
            accept_filter,
            data_handler: None,
            self_device,
            device_key: Arc::new(device_key),
            cert_overrides: std::sync::Mutex::new(HashMap::new()),
            closed: false,
            dial_back: Arc::new(DialBackTable::default()),
            #[cfg(feature = "test-hooks")]
            dial_addr_overrides: std::sync::Mutex::new(HashMap::new()),
        };

        let manager = ConnManager {
            local_addr,
            inner: Arc::new(RwLock::new(inner)),
            accept_task: Arc::new(std::sync::Mutex::new(None)),
            closed_tx: Arc::new(watch::channel(false).0),
        };

        // 启动 accept 循环。
        let listen_port = manager.local_addr.port();
        let inner_clone = manager.inner.clone();
        let task = tokio::spawn(async move {
            accept_loop(listener, listen_port, inner_clone).await;
        });
        *manager.accept_task.lock().unwrap() = Some(task.abort_handle());

        Ok(manager)
    }

    /// 监听地址。
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// 出站拨号：已连接则直接返回该设备。
    pub async fn connect(
        &self,
        addr: SocketAddr,
        _via_multicast: bool,
    ) -> Result<DeviceId, CoreError> {
        self.connect_expecting(addr, None).await
    }

    /// 拨号并要求对端就是 `expect`（TLS 握手后校验，不符立即断开，不发 Hello、不登记、不发 Up）。
    /// 按已知地址重连用它，避免把本机身份发给占用了旧地址的陌生主机。
    pub async fn connect_expecting(
        &self,
        addr: SocketAddr,
        expect: Option<DeviceId>,
    ) -> Result<DeviceId, CoreError> {
        if self.read_inner().closed {
            return Err(CoreError::PeerOffline);
        }
        // 快速路径：若已有连接到该地址且已上线，直接返回该设备。
        {
            let inner = self.read_inner();
            let replace_idle = timing::current().replace_idle;
            for (device, entry) in inner.connections.iter() {
                // 空闲过久的连接可能已死，不走快速路径，重新拨号交给裁决替换。
                // 入站连接的 peer_addr 是对端临时端口，用 dial_addr（对端监听地址）一并匹配，
                // 避免静态对端任务对已存活的入站连接反复重拨。
                let same_peer =
                    entry.peer_addr == addr || entry.connection.peer_info().dial_addr == addr;
                if same_peer
                    && entry.up_sent
                    && entry.connection.idle() < replace_idle
                    && expect.is_none_or(|e| e == *device)
                {
                    return Ok(*device);
                }
            }
        }

        let hello = self.build_hello();
        let accept_filter = self.read_inner().accept_filter.clone();
        let cleanup = make_exit_cleanup(&self.inner);
        // 整个拨号（TCP + TLS + Hello）限时：对端 TCP 已接受但应用层沉默时不能永久挂起。
        let (peer_info, connection) = tokio::time::timeout(
            timing::current().dial,
            Connection::connect_outbound(
                addr,
                expect,
                &self.device_key(),
                self.self_device(),
                &hello,
                accept_filter.as_ref(),
                self.events_clone(),
                cleanup,
            ),
        )
        .await
        .map_err(|_| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("dial to {addr} timed out"),
            ))
        })??;

        let device = peer_info.device;
        let peer_addr = peer_info.addr;
        let self_dev = self.self_device();

        // 读循环在本端发 Up 之前挂起（对端帧不先于本端 Up 上送），故宽限期间
        // 不会有 Frame 越过 Up 到达外层。
        // 同时拨号：裁决保留“拨号方 DeviceId 较小”的连接。本端较大时，本条出站
        // 连接若遇对端同时拨入，必被对端关闭；若此时先登记并发 Up，读循环随后
        // 自清理发 Down，对端拨入连接再发 Up，外层看到 Up/Down/Up。故先在宽限窗口内
        // 等待对端拨入连接登记；到达则下面的裁决会丢弃本条出站连接（不发 Up）。
        if self_dev > device {
            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_millis(DUP_DIAL_GRACE_MS);
            while tokio::time::Instant::now() < deadline
                && !self.read_inner().connections.contains_key(&device)
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }

        // 先预留事件通道名额（可能 await），再在锁内同步完成裁决/登记/发 Up：
        // 名额发送是同步的，Up 与读循环自清理后的 Down 仍严格有序，且锁内不 await。
        let permit = self.events_clone().reserve_owned().await.ok();
        // 旧连接已死被替换时需要先 Down 再 Up，多预留一个名额。
        let down_permit = self.events_clone().reserve_owned().await.ok();
        let replace_idle = timing::current().replace_idle;

        // 重复连接裁决：同一设备已有连接时，保留"拨号方 DeviceId 较小"的那条；
        // 例外：旧连接已空闲超过 replace_idle（对端无 FIN 消失后重连），无条件被新连接替换。
        let mut inner = self.write_inner();
        if inner.closed {
            connection.shutdown();
            return Err(CoreError::PeerOffline);
        }
        if let Some(existing) = inner.connections.get(&device) {
            let existing_dialer = existing.dialer_device;
            let dead = existing.connection.idle() >= replace_idle;
            if dead {
                // 旧连接已死：Down 再 Up，让上层重置该对端的同步状态并重新发 SyncOffer。
                // 旧连接读循环退出时实例 ID 不匹配，不会重复发 Down。
                existing.connection.shutdown();
                inner.connections.remove(&device);
                if let (Some(down), Some(up)) = (down_permit, permit) {
                    down.send(ConnEvent::Down(device));
                    tracing::info!(peer = %peer_info.device, "peer connection up (replaced dead connection)");
                    up.send(ConnEvent::Up(peer_info.clone()));
                }
                // 须在 Up 入队之后放行读循环，保证 Frame 事件排在 Up 之后。
                connection.set_up_sent();
                inner.connections.insert(
                    device,
                    ConnectionEntry {
                        conn_id: connection.conn_id(),
                        connection,
                        peer_addr,
                        dialer_device: self_dev,
                        up_sent: true,
                    },
                );
                let _ = inner.cert_overrides.lock().unwrap().remove(&device);
                return Ok(device);
            }
            if existing_dialer <= self_dev {
                // 保留旧连接，关闭新连接（未登记、未发 Up；其读循环退出时
                // up_sent 为假，不产生 Up/Down）。
                connection.shutdown();
                return Ok(device);
            } else {
                // 新连接拨号方较小，替换旧连接（旧连接已发 Up，此处不再发 Up 避免
                // 重复）。旧连接读循环退出时自清理：实例 ID 不匹配，不会发 Down。
                existing.connection.shutdown();
                inner.connections.remove(&device);
                connection.set_up_sent();
                inner.connections.insert(
                    device,
                    ConnectionEntry {
                        conn_id: connection.conn_id(),
                        connection,
                        peer_addr,
                        dialer_device: self_dev,
                        up_sent: true,
                    },
                );
                let _ = inner.cert_overrides.lock().unwrap().remove(&device);
                return Ok(device);
            }
        }

        if let Some(permit) = permit {
            tracing::info!(peer = %peer_info.device, "peer connection up");
            permit.send(ConnEvent::Up(peer_info.clone()));
        }
        // 须在 Up 入队之后放行读循环，保证 Frame 事件排在 Up 之后。
        connection.set_up_sent();
        inner.connections.insert(
            device,
            ConnectionEntry {
                conn_id: connection.conn_id(),
                connection,
                peer_addr,
                dialer_device: self_dev,
                up_sent: true,
            },
        );
        // 新握手证书是权威值，清除旧覆盖。
        let _ = inner.cert_overrides.lock().unwrap().remove(&device);

        Ok(device)
    }

    /// 发送帧：未连接 → `CoreError::PeerOffline`。
    pub async fn send(&self, device: &DeviceId, frame: Frame) -> Result<(), CoreError> {
        // 锁内只取出连接句柄，释放锁后再 await 发送（写队列满时不阻塞其他查询）。
        let connection = self
            .read_inner()
            .connections
            .get(device)
            .map(|e| e.connection.clone());
        match connection {
            Some(connection) => connection.send(device, frame).await,
            None => Err(CoreError::PeerOffline),
        }
    }

    /// 所有已连接对端的 PeerInfo（应用证书覆盖）。
    pub fn peers(&self) -> Vec<PeerInfo> {
        let inner = self.read_inner();
        inner
            .connections
            .values()
            .filter(|e| e.up_sent)
            .map(|e| {
                let mut pi = e.connection.peer_info().clone();
                Self::apply_cert_override(&inner, &mut pi);
                pi
            })
            .collect()
    }

    /// 更新对端证书视图（收到对端 CertUpdate 时调用）。
    ///
    /// 仅当该设备连接存在时记录；连接重建或断开时覆盖被清除，以新握手为准。
    pub(crate) async fn update_peer_cert(&self, device: &DeviceId, cert: DeviceCert) {
        let inner = self.read_inner();
        if inner.connections.contains_key(device) {
            let _ = inner.cert_overrides.lock().unwrap().insert(*device, cert);
        }
    }

    /// 是否已与指定设备建立连接。
    pub fn is_connected(&self, device: &DeviceId) -> bool {
        self.read_inner()
            .connections
            .get(device)
            .map(|e| e.up_sent)
            .unwrap_or(false)
    }

    /// 是否已 shutdown（监听已停）。
    pub fn is_shut_down(&self) -> bool {
        self.read_inner().closed
    }

    /// 已连接且最近 replace_idle 内收到过对端的包（连接看起来仍然存活）。
    ///
    /// 发现层据此判断是否需要拨号：连接表里有但长时间没动静的连接可能已死，
    /// 此时对端重新出现应拨号，由裁决用新连接替换。
    pub fn is_connected_alive(&self, device: &DeviceId) -> bool {
        let replace_idle = timing::current().replace_idle;
        self.read_inner()
            .connections
            .get(device)
            .map(|e| e.up_sent && e.connection.idle() < replace_idle)
            .unwrap_or(false)
    }

    /// 断开与指定设备的连接。
    ///
    /// 显式断开时由本函数补发 Down 通知外层（读循环退出自清理时条目已移除，
    /// 不会再发，保证 Down 恰好一次）。
    pub async fn disconnect(&self, device: &DeviceId) {
        // 先预留名额再持锁：锁内同步移除并发 Down，锁内不 await。
        let permit = self.events_clone().reserve_owned().await.ok();
        let mut inner = self.write_inner();
        if let Some(entry) = inner.connections.remove(device) {
            entry.connection.shutdown();
            if let Some(permit) = permit {
                permit.send(ConnEvent::Down(*device));
            }
        }
        let _ = inner.cert_overrides.lock().unwrap().remove(device);
    }

    /// 等待 shutdown（已关闭则立即返回）。
    pub async fn wait_shutdown(&self) {
        let _ = self.closed_tx.subscribe().wait_for(|c| *c).await;
    }

    /// 关闭所有连接与监听器。
    pub async fn shutdown(&self) {
        // 先停监听：否则对端的静态重连会立刻把刚关掉的连接又连回来。
        if let Some(task) = self.accept_task.lock().unwrap().take() {
            task.abort();
        }
        let mut inner = self.write_inner();
        inner.closed = true;
        self.closed_tx.send_replace(true);
        for (_device, entry) in inner.connections.drain() {
            entry.connection.shutdown();
        }
        inner.cert_overrides.lock().unwrap().clear();
    }

    // ---- 私有辅助 ----

    /// 把存储的证书覆盖应用到对端信息上（覆盖不存在时保持握手值）。
    fn apply_cert_override(inner: &Inner, pi: &mut PeerInfo) {
        // cert_overrides 锁仅在极短的同步区间内持有，直接阻塞获取。
        let map = inner.cert_overrides.lock().unwrap();
        if let Some(cert) = map.get(&pi.device) {
            pi.cert = Some(cert.clone());
        }
    }

    /// 注册数据连接处理器（SyncEngine 等模块在启动时调用）。
    ///
    /// std 写锁持有时间极短，阻塞获取即可。
    pub fn set_data_handler(&self, h: DataHandler) {
        self.write_inner().data_handler = Some(h);
    }

    /// 数据连接：先向 `peer.dial_addr` 直拨（不用 `addr`：被拨方看到的是对端
    /// 临时源端口）；直拨失败（出错或 3 秒超时，例如对端在 NAT 后）时改为经控制连接
    /// 请求对端拨回（见 [`ConnManager::open_data_via_dial_back`]）。
    pub async fn open_data(&self, device: &DeviceId) -> Result<DataStream, CoreError> {
        let peer = self.peer(device).ok_or(CoreError::PeerOffline)?;
        // 未初始化设备没有证书，无法自证身份，数据通道不可用。
        let own_cert = self.own_cert()?;
        let direct = tokio::time::timeout(
            DIRECT_DIAL_TIMEOUT,
            self.dial_data(
                self.direct_dial_addr(&peer),
                None,
                own_cert,
                CHANNEL_DATA,
                None,
            ),
        )
        .await;
        match direct {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(e)) => {
                tracing::debug!(peer = %device, error = %e, "direct data dial failed, requesting dial-back");
                self.open_data_via_dial_back(device).await
            }
            Err(_) => {
                tracing::debug!(peer = %device, "direct data dial timed out, requesting dial-back");
                self.open_data_via_dial_back(device).await
            }
        }
    }

    /// 本机设备证书（未初始化返回 Rejected）。
    fn own_cert(&self) -> Result<DeviceCert, CoreError> {
        let inner = self.read_inner();
        let cert = inner.hello_rx.borrow().cert.clone();
        cert.ok_or_else(|| {
            CoreError::Rejected("local identity not initialized, data channel unavailable".into())
        })
    }

    /// 建立一条数据连接：TCP → TLS（可校验对端设备）→ 通道字节 → DataHello →（可选）DialBack。
    async fn dial_data(
        &self,
        addr: SocketAddr,
        expect: Option<DeviceId>,
        own_cert: DeviceCert,
        channel: u8,
        token: Option<[u8; 16]>,
    ) -> Result<DataStream, CoreError> {
        let stream = TcpStream::connect(addr).await.map_err(CoreError::Io)?;
        let (_server_config, client_config) =
            make_tls_configs_expecting(&self.device_key(), expect)?;
        let domain = ServerName::try_from(TLS_SNI.as_bytes())
            .map_err(|e| CoreError::Tls(format!("invalid SNI: {e}")))?;
        let mut tls_stream = tokio_rustls::TlsStream::from(
            TlsConnector::from(client_config)
                .connect(domain, stream)
                .await
                .map_err(|e| CoreError::Tls(e.to_string()))?,
        );
        tls_stream
            .write_all(&[channel])
            .await
            .map_err(CoreError::Io)?;
        write_data_frame(&mut tls_stream, &DataFrame::DataHello { cert: own_cert })
            .await
            .map_err(|e| CoreError::Tls(e.to_string()))?;
        if let Some(token) = token {
            write_data_frame(&mut tls_stream, &DataFrame::DialBack { token })
                .await
                .map_err(|e| CoreError::Tls(e.to_string()))?;
        }
        Ok(tls_stream)
    }

    /// 反向建立：登记一次性令牌 → 经控制连接发 DataDialBack → 最多等 10 秒对端拨回。
    /// 超时返回 `PeerOffline`。返回的流虽由对方拨入（TLS 服务端角色），
    /// 逻辑上仍是请求方（先发 GetMeta/GetRange）。
    pub(crate) async fn open_data_via_dial_back(
        &self,
        device: &DeviceId,
    ) -> Result<DataStream, CoreError> {
        let table = self.read_inner().dial_back.clone();
        let (token, rx) = table.register(*device, DIAL_BACK_WAIT);
        if let Err(e) = self.send(device, Frame::DataDialBack { token }).await {
            table.cancel(&token);
            return Err(e);
        }
        match tokio::time::timeout(DIAL_BACK_WAIT, rx).await {
            Ok(Ok(stream)) => Ok(stream),
            _ => {
                table.cancel(&token);
                Err(CoreError::PeerOffline)
            }
        }
    }

    /// 收到对端的 DataDialBack：向其监听地址（控制连接远端 IP + Hello.listen_port）
    /// 建数据连接并出示令牌，随后按普通入站数据连接交给数据处理器（本端仍是服务方）。
    /// 只拨对端自己的地址，不会被诱导去连第三方。
    pub(crate) async fn serve_dial_back(
        &self,
        device: &DeviceId,
        token: [u8; 16],
    ) -> Result<(), CoreError> {
        let peer = self.peer(device).ok_or(CoreError::PeerOffline)?;
        let stream = self.dial_back_stream(device, token).await?;
        let handler = {
            let inner = self.read_inner();
            if inner.closed {
                return Err(CoreError::PeerOffline);
            }
            inner.data_handler.clone()
        };
        match handler {
            Some(h) => {
                h(peer, stream);
                Ok(())
            }
            None => Err(CoreError::Rejected("data channel not enabled".into())),
        }
    }

    /// 拨回并出示令牌，返回已就绪的数据流（不交给处理器）。
    pub(crate) async fn dial_back_stream(
        &self,
        device: &DeviceId,
        token: [u8; 16],
    ) -> Result<DataStream, CoreError> {
        let peer = self.peer(device).ok_or(CoreError::PeerOffline)?;
        let own_cert = self.own_cert()?;
        tokio::time::timeout(
            DIAL_BACK_DIAL_TIMEOUT,
            self.dial_data(
                peer.dial_addr,
                Some(*device),
                own_cert,
                CHANNEL_DATA_DIAL_BACK,
                Some(token),
            ),
        )
        .await
        .map_err(|_| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "dial-back connect timed out",
            ))
        })?
    }

    /// 直拨地址：生产环境即对端的 dial_addr；测试钩子可覆盖以模拟直拨不通。
    fn direct_dial_addr(&self, peer: &PeerInfo) -> SocketAddr {
        #[cfg(feature = "test-hooks")]
        if let Some(a) = self
            .read_inner()
            .dial_addr_overrides
            .lock()
            .unwrap()
            .get(&peer.device)
        {
            return *a;
        }
        peer.dial_addr
    }

    /// 测试辅助：覆盖本端看到的某对端 dial_addr（模拟该对端在 NAT 后，直拨不通）。
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn override_dial_addr_for_tests(&self, device: DeviceId, addr: SocketAddr) {
        self.read_inner()
            .dial_addr_overrides
            .lock()
            .unwrap()
            .insert(device, addr);
    }

    /// 查询已上线对端的信息（含 tls_exporter 与 dial_addr；应用证书覆盖）。
    pub fn peer(&self, device: &DeviceId) -> Option<PeerInfo> {
        let inner = self.read_inner();
        let mut pi = inner
            .connections
            .get(device)
            .filter(|e| e.up_sent)
            .map(|e| e.connection.peer_info().clone())?;
        Self::apply_cert_override(&inner, &mut pi);
        Some(pi)
    }

    // 以下辅助均为同步：锁毒化说明持锁线程已 panic，沿用仓库 lock().unwrap() 风格传播。
    fn read_inner(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap()
    }

    fn write_inner(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap()
    }

    fn build_hello(&self) -> HelloInfo {
        let hello_info = self.read_inner().hello_rx.borrow().clone();
        HelloInfo {
            display_name: hello_info.display_name,
            cert: hello_info.cert,
            listen_port: self.local_addr.port(),
        }
    }

    fn device_key(&self) -> Arc<DeviceKey> {
        self.read_inner().device_key.clone()
    }

    fn self_device(&self) -> DeviceId {
        self.read_inner().self_device
    }

    fn events_clone(&self) -> mpsc::Sender<ConnEvent> {
        self.read_inner().events.clone()
    }
}

/// 构造连接读循环退出清理回调：持写锁后仅当表中条目仍是该连接实例时移除。
///
/// 返回是否确有移除——被替换或已显式断开的连接返回 false，读循环据此跳过
/// Down 通知，避免对存活的替换连接误报离线。
fn make_exit_cleanup(inner: &Arc<RwLock<Inner>>) -> ExitCleanup {
    let inner = Arc::clone(inner);
    Arc::new(move |conn_id| {
        let inner = Arc::clone(&inner);
        Box::pin(async move {
            let mut guard = inner.write().unwrap();
            let mine = guard
                .connections
                .iter()
                .find(|(_, e)| e.conn_id == conn_id)
                .map(|(d, _)| *d);
            match mine {
                Some(device) => {
                    guard.connections.remove(&device);
                    true
                }
                None => false,
            }
        })
    })
}

/// 接受循环：持续接受入站 TCP 连接并进行 TLS + Hello 校验。
async fn accept_loop(listener: TcpListener, listen_port: u16, inner: Arc<RwLock<Inner>>) {
    loop {
        let (stream, _peer_addr) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => return,
        };

        let inner_clone = inner.clone();
        tokio::spawn(async move {
            let (self_device, device_key, accept_filter, data_handler, events, cleanup, dial_back) = {
                let guard = inner_clone.read().unwrap();
                (
                    guard.self_device,
                    DeviceKey::from_bytes(&guard.device_key.to_bytes()),
                    guard.accept_filter.clone(),
                    guard.data_handler.clone(),
                    guard.events.clone(),
                    make_exit_cleanup(&inner_clone),
                    guard.dial_back.clone(),
                )
            };

            let hello_info = {
                let guard = inner_clone.read().unwrap();
                let borrowed = guard.hello_rx.borrow();
                HelloInfo {
                    display_name: borrowed.display_name.clone(),
                    cert: borrowed.cert.clone(),
                    listen_port,
                }
            };

            // 已 shutdown 后到达的数据连接不再交给处理器。
            let data_handler = data_handler.map(|h| {
                let inner = inner_clone.clone();
                Arc::new(move |peer: PeerInfo, stream: DataStream| {
                    if !inner.read().unwrap().closed {
                        h(peer, stream);
                    }
                }) as DataHandler
            });

            let events_for_up = events.clone();
            // 入站握手限时：只连接不发数据的对端在时限后被关闭（丢弃 stream 释放 fd）。
            let accepted = tokio::time::timeout(
                timing::current().accept_handshake,
                Connection::accept_channel(
                    stream,
                    &device_key,
                    self_device,
                    &hello_info,
                    accept_filter.as_ref(),
                    events,
                    data_handler,
                    dial_back.as_ref(),
                    cleanup,
                ),
            )
            .await
            .unwrap_or_else(|_| {
                Err(CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "inbound handshake timed out",
                )))
            });
            match accepted {
                Ok(Accepted::Control(peer_info, connection)) => {
                    let device = peer_info.device;
                    // 同时拨号：拨入方 DeviceId 较大时，本端若也在向其拨号，该拨入连接
                    // 必被对端关闭；先登记并发 Up 会在随后读循环自清理时产生 Down，
                    // 本端出站连接再发 Up。故在宽限窗口内等待本端出站连接登记，
                    // 登记后下面的裁决会丢弃本条拨入连接（不发 Up）。
                    if device > self_device {
                        let deadline = tokio::time::Instant::now()
                            + std::time::Duration::from_millis(DUP_DIAL_GRACE_MS);
                        while tokio::time::Instant::now() < deadline
                            && !inner_clone
                                .read()
                                .unwrap()
                                .connections
                                .contains_key(&device)
                        {
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                    }
                    // 先预留事件名额再持锁，锁内同步发 Up，不跨 await 持锁。
                    let permit = events_for_up.clone().reserve_owned().await.ok();
                    let down_permit = events_for_up.reserve_owned().await.ok();
                    let replace_idle = timing::current().replace_idle;
                    let mut guard = inner_clone.write().unwrap();
                    if guard.closed {
                        connection.shutdown();
                        return;
                    }
                    // 重复连接裁决：入站连接的拨号方为 peer；旧连接已空闲超过
                    // replace_idle（对端重启 / 无 FIN 消失）时无条件替换并 Down+Up。
                    let mut dead = false;
                    let replaced = if let Some(existing) = guard.connections.get(&device) {
                        let existing_dialer = existing.dialer_device;
                        let new_dialer = peer_info.device;
                        dead = existing.connection.idle() >= replace_idle;
                        if !dead && existing_dialer <= new_dialer {
                            connection.shutdown();
                            return;
                        } else {
                            existing.connection.shutdown();
                            guard.connections.remove(&device);
                            true
                        }
                    } else {
                        false
                    };
                    // 仅新登记才发 Up：替换路径上旧连接已发过 Up 且从未离线，
                    // 重复 Up 会破坏外层在线计数（被替换连接退出也不再发 Down）。
                    if !replaced || dead {
                        if dead {
                            if let Some(down) = down_permit {
                                down.send(ConnEvent::Down(device));
                            }
                        }
                        if let Some(permit) = permit {
                            tracing::info!(peer = %peer_info.device, "peer connection up");
                            permit.send(ConnEvent::Up(peer_info.clone()));
                        }
                    }
                    // 须在 Up 入队之后放行读循环，保证 Frame 事件排在 Up 之后。
                    connection.set_up_sent();
                    guard.connections.insert(
                        device,
                        ConnectionEntry {
                            conn_id: connection.conn_id(),
                            connection,
                            peer_addr: peer_info.addr,
                            dialer_device: peer_info.device,
                            up_sent: true,
                        },
                    );
                    // 新握手证书是权威值，清除旧覆盖。
                    let _ = guard.cert_overrides.lock().unwrap().remove(&device);
                }
                Ok(Accepted::Data(peer_info)) => {
                    // 数据连接已交给注册的处理器，无需登记；仅记录审计日志。
                    tracing::debug!(device = %peer_info.device, "data connection established");
                }
                Err(_) => {
                    // 校验失败：已在 Connection 内发送 Error 帧并关闭。
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::tls::make_tls_configs;
    use std::time::Duration;
    use tokio::sync::{mpsc, watch};

    use crate::identity::Identity;
    use lanmsg_proto::ids::DeviceId;
    use lanmsg_proto::sign::{DeviceKey, UserKey};
    use lanmsg_proto::types::{DeviceCert, DeviceType, Signature64};

    fn make_identity() -> (DeviceKey, Identity) {
        let device = DeviceKey::generate();
        let identity = Identity {
            device: DeviceKey::from_bytes(&device.to_bytes()),
            user: None,
            cert: None,
        };
        (device, identity)
    }

    fn make_cert_for(device: DeviceId) -> DeviceCert {
        let user = UserKey::generate();
        DeviceCert::issue(
            &user,
            device,
            "测试设备".into(),
            DeviceType::Desktop,
            1_700_000_000_000,
        )
    }

    async fn make_manager() -> (ConnManager, mpsc::Receiver<ConnEvent>, DeviceId) {
        let (device, identity) = make_identity();
        let identity = Arc::new(RwLock::new(identity));
        let (hello_tx, hello_rx) = watch::channel(HelloInfo {
            display_name: "测试".into(),
            cert: None,
            listen_port: 0,
        });
        let (events_tx, events_rx) = mpsc::channel(32);
        let filter = Arc::new(|_: &PeerInfo| true);
        let manager = ConnManager::bind(identity.clone(), 0, hello_rx, filter, events_tx)
            .await
            .expect("bind 失败");
        let _ = hello_tx;
        let device_id = device.device_id();
        (manager, events_rx, device_id)
    }

    /// 带证书的管理器：用于数据通道与身份校验测试。
    async fn make_manager_with_cert(
    ) -> (ConnManager, mpsc::Receiver<ConnEvent>, DeviceId, DeviceCert) {
        let (device, _identity) = make_identity();
        let cert = make_cert_for(device.device_id());
        let identity = Identity {
            device: DeviceKey::from_bytes(&device.to_bytes()),
            user: None,
            cert: Some(cert.clone()),
        };
        let identity = Arc::new(RwLock::new(identity));
        let (hello_tx, hello_rx) = watch::channel(HelloInfo {
            display_name: "测试".into(),
            cert: Some(cert.clone()),
            listen_port: 0,
        });
        let (events_tx, events_rx) = mpsc::channel(32);
        let filter = Arc::new(|_: &PeerInfo| true);
        let manager = ConnManager::bind(identity.clone(), 0, hello_rx, filter, events_tx)
            .await
            .expect("bind 失败");
        let _ = hello_tx;
        let device_id = device.device_id();
        (manager, events_rx, device_id, cert)
    }

    async fn expect_up(rx: &mut mpsc::Receiver<ConnEvent>, device: DeviceId) {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(ConnEvent::Up(peer))) => assert_eq!(peer.device, device),
            Ok(Some(other)) => panic!("期望 Up({device:?})，收到 {:?}", other),
            Ok(None) => panic!("事件通道已关闭"),
            Err(_) => panic!("5 秒内未收到 Up({device:?})"),
        }
    }

    async fn expect_no_up(rx: &mut mpsc::Receiver<ConnEvent>) {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Err(_) => {}
            Ok(Some(ConnEvent::Up(p))) => panic!("不应收到 Up，却收到来自 {:?}", p.device),
            Ok(None) => panic!("事件通道被提前关闭"),
            Ok(Some(other)) => panic!("不应收到事件，却收到 {:?}", other),
        }
    }

    /// 生成一对带证书的管理器，使 (larger, smaller) 满足 larger.device > smaller.device
    /// （与裁决代码一致的 DeviceId 比较），不满足则重新生成。
    async fn make_ordered_pair() -> (
        (ConnManager, mpsc::Receiver<ConnEvent>, DeviceId),
        (ConnManager, mpsc::Receiver<ConnEvent>, DeviceId),
    ) {
        loop {
            let (m1, e1, d1, _) = make_manager_with_cert().await;
            let (m2, e2, d2, _) = make_manager_with_cert().await;
            if d1 > d2 {
                return ((m1, e1, d1), (m2, e2, d2));
            } else if d2 > d1 {
                return ((m2, e2, d2), (m1, e1, d1));
            }
        }
    }

    /// DeviceId 较大的 A 单向拨较小的 B：出站侧（A.connect）与入站侧（B.accept）
    /// 都因“拨号方较大”先等待宽限窗口，故两侧 Up 都不早于 DUP_DIAL_GRACE_MS。
    #[tokio::test]
    async fn larger_dialer_only_connects_after_grace() {
        let ((mgr_a, mut ev_a, dev_a), (mgr_b, mut ev_b, dev_b)) = make_ordered_pair().await;
        assert!(dev_a > dev_b);

        let grace = Duration::from_millis(DUP_DIAL_GRACE_MS);
        // 在拨号前启动观察任务，记录 B 收到 Up(A) 的真实时刻；不能在 connect 返回
        // 之后再取 elapsed（出站侧已等过宽限，必然 >= grace，无法区分入站宽限）。
        // B 的登记只取决于 B 自己的握手与入站宽限，与 A 的出站宽限无关。
        let watcher = tokio::spawn(async move {
            loop {
                match tokio::time::timeout(Duration::from_secs(5), ev_b.recv()).await {
                    Ok(Some(ConnEvent::Up(p))) => {
                        assert_eq!(p.device, dev_a);
                        return (ev_b, tokio::time::Instant::now());
                    }
                    Ok(Some(_)) => {}
                    other => panic!("B 应收到 Up(A)，却收到 {:?}", other),
                }
            }
        });
        let start = tokio::time::Instant::now();
        mgr_a
            .connect(mgr_b.local_addr(), false)
            .await
            .expect("A 拨号 B 失败");
        // 出站侧：connect 返回（已登记并发 Up）不早于宽限。
        assert!(
            start.elapsed() >= grace,
            "出站侧未等待宽限: {:?}",
            start.elapsed()
        );
        expect_up(&mut ev_a, dev_b).await;
        // 入站侧：B 的 Up 到达时刻（观察任务记录）不早于宽限。
        let (mut ev_b, t_b_up) = watcher.await.expect("观察任务失败");
        assert!(
            t_b_up - start >= grace,
            "入站侧未等待宽限: {:?}",
            t_b_up - start
        );

        // Up 已消费，这里复核无多余事件、连接数与双向帧。
        expect_no_up(&mut ev_a).await;
        expect_no_up(&mut ev_b).await;
        assert_eq!(mgr_a.peers().len(), 1);
        assert_eq!(mgr_b.peers().len(), 1);
        mgr_a
            .send(&dev_b, Frame::Presence(lanmsg_proto::types::Status::Online))
            .await
            .expect("A 发帧失败");
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), ev_b.recv()).await,
            Ok(Some(ConnEvent::Frame(d, Frame::Presence(_)))) if d == dev_a
        ));
        mgr_b
            .send(&dev_a, Frame::Presence(lanmsg_proto::types::Status::Online))
            .await
            .expect("B 发帧失败");
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), ev_a.recv()).await,
            Ok(Some(ConnEvent::Frame(d, Frame::Presence(_)))) if d == dev_b
        ));
    }

    /// 较小的 B 拨较大的 A：两侧宽限条件（出站 self>peer、入站 peer>self）均不成立，
    /// 无需等待直接连上，双方恰好一次 Up、可互发帧。
    #[tokio::test]
    async fn smaller_dialer_connects_to_larger_acceptor() {
        let ((mgr_a, mut ev_a, dev_a), (mgr_b, mut ev_b, dev_b)) = make_ordered_pair().await;
        assert!(dev_a > dev_b);

        mgr_b
            .connect(mgr_a.local_addr(), false)
            .await
            .expect("B 拨号 A 失败");
        // 不断言耗时上界：冷启动/高负载下握手本身可能超过宽限窗口，会造成误报。
        expect_up(&mut ev_b, dev_a).await;
        expect_up(&mut ev_a, dev_b).await;
        // 以下复核无多余 Up、连接数与双向帧（Up 已在上面消费）。
        expect_no_up(&mut ev_b).await;
        expect_no_up(&mut ev_a).await;
        assert_eq!(mgr_a.peers().len(), 1);
        assert_eq!(mgr_b.peers().len(), 1);
        mgr_b
            .send(&dev_a, Frame::Presence(lanmsg_proto::types::Status::Online))
            .await
            .expect("B 发帧失败");
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), ev_a.recv()).await,
            Ok(Some(ConnEvent::Frame(d, Frame::Presence(_)))) if d == dev_b
        ));
        mgr_a
            .send(&dev_b, Frame::Presence(lanmsg_proto::types::Status::Online))
            .await
            .expect("A 发帧失败");
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), ev_b.recv()).await,
            Ok(Some(ConnEvent::Frame(d, Frame::Presence(_)))) if d == dev_a
        ));
    }

    #[tokio::test]
    async fn two_managers_connect_and_exchange_frames() {
        // 带证书的管理器：Presence 等普通帧在全身份下自由交换；
        // cert=None 场景的帧过滤由 uninitialized_peer_only_allowed_pair_frames 覆盖。
        let (mgr_a, mut events_a, dev_a, _cert_a) = make_manager_with_cert().await;
        let (mgr_b, mut events_b, dev_b, _cert_b) = make_manager_with_cert().await;

        let _ = mgr_a
            .connect(mgr_b.local_addr(), false)
            .await
            .expect("A 拨号 B 失败");
        expect_up(&mut events_a, dev_b).await;
        expect_up(&mut events_b, dev_a).await;

        assert!(mgr_a.is_connected(&dev_b));
        assert!(mgr_b.is_connected(&dev_a));
        assert_eq!(mgr_a.peers().len(), 1);
        assert_eq!(mgr_b.peers().len(), 1);

        // A 向 B 发 Presence(Online)。
        mgr_a
            .send(&dev_b, Frame::Presence(lanmsg_proto::types::Status::Online))
            .await
            .expect("A 发送帧失败");
        match tokio::time::timeout(Duration::from_secs(5), events_b.recv()).await {
            Ok(Some(ConnEvent::Frame(d, f))) => {
                assert_eq!(d, dev_a);
                assert!(matches!(f, Frame::Presence(_)));
            }
            other => panic!("B 应收到 Frame，却收到 {:?}", other),
        }
    }

    #[tokio::test]
    async fn simultaneous_dial_keeps_one_connection() {
        let (mgr_a, mut events_a, _dev_a) = make_manager().await;
        let (mgr_b, mut events_b, _dev_b) = make_manager().await;

        let _ = tokio::join!(
            mgr_a.connect(mgr_b.local_addr(), false),
            mgr_b.connect(mgr_a.local_addr(), false),
        );

        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(mgr_a.peers().len(), 1);
        assert_eq!(mgr_b.peers().len(), 1);

        // 双方各自收到的 Up 事件恰好 1 次。
        let mut up_a = 0;
        let mut up_b = 0;
        for _ in 0..4 {
            match tokio::time::timeout(Duration::from_secs(2), events_a.recv()).await {
                Ok(Some(ConnEvent::Up(_))) => up_a += 1,
                Ok(Some(ConnEvent::Down(_))) => {}
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => break,
            }
        }
        for _ in 0..4 {
            match tokio::time::timeout(Duration::from_secs(2), events_b.recv()).await {
                Ok(Some(ConnEvent::Up(_))) => up_b += 1,
                Ok(Some(ConnEvent::Down(_))) => {}
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => break,
            }
        }
        assert_eq!(up_a, 1, "A 应恰好收到 1 次 Up");
        assert_eq!(up_b, 1, "B 应恰好收到 1 次 Up");
    }

    /// 回归：有 send 在写队列上挂起（对端事件通道不被消费 → 读循环停滞 → 写队列
    /// 逐步占满）并持续有写锁请求时，peer()/peers()/is_connected() 仍须返回对端，
    /// 不得因 try_read 失败而返回空。
    #[tokio::test]
    async fn peer_lookup_not_blocked_by_pending_writer() {
        let (mgr_a, mut events_a, _dev_a) = make_manager().await;
        let (mgr_b, _events_b, dev_b) = make_manager().await;
        mgr_a
            .connect(mgr_b.local_addr(), false)
            .await
            .expect("拨号失败");
        expect_up(&mut events_a, dev_b).await;

        // 持续发送且不消费 B 的事件，直到发送方在写队列上挂起。
        let sender = mgr_a.clone();
        let flood = tokio::spawn(async move {
            loop {
                if sender
                    .send(&dev_b, Frame::Presence(lanmsg_proto::types::Status::Online))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        // 同时不断发起需要写锁的操作（update_peer_cert 外的写路径：disconnect 的
        // 对象不存在时也会取写锁）。
        let writer = mgr_a.clone();
        let churn = tokio::spawn(async move {
            let ghost = DeviceKey::generate().device_id();
            loop {
                writer.disconnect(&ghost).await;
                tokio::task::yield_now().await;
            }
        });

        for _ in 0..200 {
            assert!(mgr_a.is_connected(&dev_b), "is_connected 不应返回 false");
            assert!(mgr_a.peer(&dev_b).is_some(), "peer 不应返回 None");
            assert_eq!(mgr_a.peers().len(), 1, "peers 不应返回空");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        flood.abort();
        churn.abort();
    }

    #[tokio::test]
    async fn rejects_hello_cert_mismatching_tls_key() {
        let (_mgr_a, _events_a, _dev_a) = make_manager().await;
        let (mgr_b, mut _events_b, _dev_b) = make_manager().await;

        // A 的 HelloInfo.cert 使用另一台设备 C 的合法证书（device C ≠ A 的 TLS 设备 ID）。
        let device_c = DeviceKey::generate().device_id();
        let cert_c = make_cert_for(device_c);
        let (_hello_tx, hello_rx) = watch::channel(HelloInfo {
            display_name: "A".into(),
            cert: Some(cert_c),
            listen_port: 0,
        });
        // 改为新建一个 A'（带错误的 cert）。
        let (_device_a2, _identity_raw) = make_identity();
        let identity_a2 = Arc::new(RwLock::new(Identity {
            device: DeviceKey::from_bytes(&_device_a2.to_bytes()),
            user: None,
            cert: None,
        }));
        let (events_tx_a2, mut events_a2) = mpsc::channel(32);
        let filter = Arc::new(|_: &PeerInfo| true);
        let mgr_a2 = ConnManager::bind(identity_a2, 0, hello_rx, filter, events_tx_a2)
            .await
            .expect("A' bind 失败");

        let _ = mgr_a2.connect(mgr_b.local_addr(), false).await;
        // B 不应产生 Up。
        expect_no_up(&mut _events_b).await;
        // A' 也不应产生 Up（连接被拒）。
        expect_no_up(&mut events_a2).await;
    }

    #[tokio::test]
    async fn accept_filter_false_rejects_peer() {
        let (_mgr_a, mut events_a, _dev_a) = make_manager().await;
        let (_device_b, _identity_raw) = make_identity();
        let identity_b = Arc::new(RwLock::new(Identity {
            device: DeviceKey::from_bytes(&_device_b.to_bytes()),
            user: None,
            cert: None,
        }));
        let (hello_tx, hello_rx) = watch::channel(HelloInfo {
            display_name: "B".into(),
            cert: None,
            listen_port: 0,
        });
        let (events_tx_b, _events_b) = mpsc::channel(32);
        // B 的 filter 恒 false。
        let filter = Arc::new(|_: &PeerInfo| false);
        let mgr_b = ConnManager::bind(identity_b, 0, hello_rx, filter, events_tx_b)
            .await
            .expect("B bind 失败");
        drop(hello_tx);

        let _ = _mgr_a.connect(mgr_b.local_addr(), false).await;
        // A 不应收到 Up。
        expect_no_up(&mut events_a).await;
    }

    /// 出站连接与入站对称：本端 filter 拒绝对端时，拨号失败且不登记、不发 Up。
    #[tokio::test]
    async fn outbound_connection_also_goes_through_accept_filter() {
        let (mgr_b, _events_b, _dev_b) = make_manager().await;
        let (device_a, _) = make_identity();
        let identity_a = Arc::new(RwLock::new(Identity {
            device: DeviceKey::from_bytes(&device_a.to_bytes()),
            user: None,
            cert: None,
        }));
        let (_hello_tx, hello_rx) = watch::channel(HelloInfo {
            display_name: "A".into(),
            cert: None,
            listen_port: 0,
        });
        let (events_tx_a, mut events_a) = mpsc::channel(32);
        let filter = Arc::new(|_: &PeerInfo| false);
        let mgr_a = ConnManager::bind(identity_a, 0, hello_rx, filter, events_tx_a)
            .await
            .expect("A bind failed");
        assert!(mgr_a.connect(mgr_b.local_addr(), false).await.is_err());
        expect_no_up(&mut events_a).await;
        assert!(mgr_a.peers().is_empty());
    }

    #[tokio::test]
    async fn down_event_when_peer_shuts_down() {
        let (mgr_a, mut events_a, dev_a) = make_manager().await;
        let (mgr_b, mut events_b, dev_b) = make_manager().await;

        let _ = mgr_a.connect(mgr_b.local_addr(), false).await;
        expect_up(&mut events_a, dev_b).await;
        expect_up(&mut events_b, dev_a).await;

        mgr_b.shutdown().await;

        // A 5 秒内收到 Down(B)。
        match tokio::time::timeout(Duration::from_secs(5), events_a.recv()).await {
            Ok(Some(ConnEvent::Down(d))) => assert_eq!(d, dev_b),
            other => panic!("A 应收到 Down(B)，却收到 {:?}", other),
        }
        let _ = dev_a;
    }

    /// 对端关闭后：本端表条目被读循环自清理，peers()/peer() 不再返回死连接，
    /// 向死连接 send 返回 PeerOffline。
    #[tokio::test]
    async fn dead_connection_removed_and_send_reports_peer_offline() {
        let (mgr_a, mut events_a, dev_a) = make_manager().await;
        let (mgr_b, _events_b, dev_b) = make_manager().await;

        mgr_a.connect(mgr_b.local_addr(), false).await.unwrap();
        expect_up(&mut events_a, dev_b).await;

        // B 关闭：A 的读循环退出 → 自清理表条目 → Down 恰一次。
        mgr_b.shutdown().await;
        match tokio::time::timeout(Duration::from_secs(5), events_a.recv()).await {
            Ok(Some(ConnEvent::Down(d))) => assert_eq!(d, dev_b),
            other => panic!("A 应收到 Down(B)，却收到 {:?}", other),
        }
        let _ = dev_a;

        // 断开后立即：无死条目、未连接、send 报 PeerOffline。
        assert!(mgr_a.peer(&dev_b).is_none(), "peer() 不应返回死连接");
        assert!(!mgr_a.is_connected(&dev_b));
        assert_eq!(mgr_a.peers().len(), 0, "peers() 不应含死条目");
        let err = mgr_a
            .send(&dev_b, Frame::Presence(lanmsg_proto::types::Status::Online))
            .await
            .unwrap_err();
        assert!(
            matches!(err, CoreError::PeerOffline),
            "应报 PeerOffline，实际 {err:?}"
        );
    }

    /// 双向同时拨号的裁决竞态：被淘汰连接不发虚假 Down，存活连接条目不被
    /// 误删且帧可送达（若被误删，send 返回 PeerOffline）。
    #[tokio::test]
    async fn arbitration_keeps_alive_connection_usable() {
        let (mgr_a, mut events_a, dev_a, _ca) = make_manager_with_cert().await;
        let (mgr_b, mut events_b, dev_b, _cb) = make_manager_with_cert().await;

        let _ = tokio::join!(
            mgr_a.connect(mgr_b.local_addr(), false),
            mgr_b.connect(mgr_a.local_addr(), false),
        );
        tokio::time::sleep(Duration::from_millis(500)).await;

        assert_eq!(mgr_a.peers().len(), 1);
        assert_eq!(mgr_b.peers().len(), 1);

        // 存活连接可用：A 发 Presence，B 排空先导事件后收到。
        mgr_a
            .send(&dev_b, Frame::Presence(lanmsg_proto::types::Status::Online))
            .await
            .expect("存活连接应可发送");
        loop {
            match tokio::time::timeout(Duration::from_secs(2), events_b.recv()).await {
                Ok(Some(ConnEvent::Frame(d, f))) => {
                    assert_eq!(d, dev_a);
                    assert!(matches!(f, Frame::Presence(_)));
                    break;
                }
                Ok(Some(_)) => continue,
                other => panic!("B 应收到帧，却收到 {:?}", other),
            }
        }

        // 无虚假 Down：被淘汰连接不通知外层（500ms 内 A 侧不出现 Down）。
        if let Ok(Some(ConnEvent::Down(d))) =
            tokio::time::timeout(Duration::from_millis(500), events_a.recv()).await
        {
            panic!("不应有虚假 Down({d:?})");
        }
    }

    #[test]
    fn cert_signature_roundtrip() {
        let device = DeviceId([1u8; 32]);
        let user = UserKey::generate();
        let cert = DeviceCert::issue(
            &user,
            device,
            "台式机".into(),
            DeviceType::Desktop,
            1_700_000_000_000,
        );
        assert!(cert.verify().is_ok());
        let _ = Signature64([0u8; 64]);
    }

    #[tokio::test]
    async fn both_sides_see_same_tls_exporter() {
        let (mgr_a, mut events_a, dev_a) = make_manager().await;
        let (mgr_b, mut events_b, dev_b) = make_manager().await;

        mgr_a
            .connect(mgr_b.local_addr(), false)
            .await
            .expect("A 拨号 B 失败");
        expect_up(&mut events_a, dev_b).await;
        expect_up(&mut events_b, dev_a).await;

        let a_view = mgr_a.peer(&dev_b).expect("A 看不到 B");
        let b_view = mgr_b.peer(&dev_a).expect("B 看不到 A");
        assert_eq!(
            a_view.tls_exporter, b_view.tls_exporter,
            "双方 TLS 导出值应一致"
        );
        assert_ne!(a_view.tls_exporter, [0u8; 32], "导出值不应为全零");
    }

    #[tokio::test]
    async fn data_channel_reaches_handler_with_peer_identity() {
        let (mgr_a, mut events_a, dev_a, _cert_a) = make_manager_with_cert().await;
        let (mgr_b, mut events_b, dev_b, _cert_b) = make_manager_with_cert().await;

        mgr_a
            .connect(mgr_b.local_addr(), false)
            .await
            .expect("A 拨号 B 失败");
        expect_up(&mut events_a, dev_b).await;
        expect_up(&mut events_b, dev_a).await;

        // B 注册数据处理器：把对端设备 ID 转发给测试。
        let (data_tx, mut data_rx) = mpsc::unbounded_channel();
        let data_tx_for_handler = data_tx.clone();
        mgr_b.set_data_handler(Arc::new(move |peer, _stream| {
            let _ = data_tx_for_handler.send(peer.device);
        }));

        // A 打开数据连接，B 的处理器应收到 A 的身份。
        let _stream = mgr_a.open_data(&dev_b).await.expect("open_data 应成功");
        let device = tokio::time::timeout(Duration::from_secs(5), data_rx.recv())
            .await
            .expect("5 秒内 handler 未被调用")
            .expect("handler 通道关闭");
        assert_eq!(device, dev_a, "handler 收到的对端身份应为 A");
    }

    #[tokio::test]
    async fn uninitialized_peer_only_allowed_pair_frames() {
        let (mgr_a, mut events_a, dev_a) = make_manager().await;
        let (mgr_b, mut events_b, dev_b) = make_manager().await;

        mgr_a
            .connect(mgr_b.local_addr(), false)
            .await
            .expect("A 拨号 B 失败");
        expect_up(&mut events_a, dev_b).await;
        expect_up(&mut events_b, dev_a).await;

        // A 无证书，发 Presence → B 的读循环应拦下（不产生 Frame 事件），A 收到 Error 403。
        mgr_a
            .send(&dev_b, Frame::Presence(lanmsg_proto::types::Status::Online))
            .await
            .expect("A 发送 Presence 失败");
        match tokio::time::timeout(Duration::from_secs(5), events_a.recv()).await {
            Ok(Some(ConnEvent::Frame(d, Frame::Error { code, .. }))) => {
                assert_eq!(d, dev_b);
                assert_eq!(code, 403, "未初始化对端发普通帧应收到 403");
            }
            other => panic!("A 应收到 Error 403，却收到 {:?}", other),
        }
        // B 的事件通道不应收到 Frame 事件（超时即通过）。
        match tokio::time::timeout(Duration::from_millis(500), events_b.recv()).await {
            Err(_) => {}
            Ok(Some(ConnEvent::Frame(d, f))) => {
                panic!("B 不应收到未初始化对端的帧，却收到 {:?} 的 {:?}", d, f)
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("B 的事件通道提前关闭"),
        }

        // A 发 Pair(Abort) → B 应收到 Frame 事件。
        mgr_a
            .send(
                &dev_b,
                Frame::Pair(lanmsg_proto::types::PairMsg::Abort {
                    reason: "测试".into(),
                }),
            )
            .await
            .expect("A 发送 Pair(Abort) 失败");
        match tokio::time::timeout(Duration::from_secs(5), events_b.recv()).await {
            Ok(Some(ConnEvent::Frame(d, f))) => {
                assert_eq!(d, dev_a);
                assert!(
                    matches!(f, Frame::Pair(_)),
                    "Pair 帧应放行给未初始化对端，收到 {:?}",
                    f
                );
            }
            other => panic!("B 应收到 Pair(Abort)，却收到 {:?}", other),
        }
    }

    #[tokio::test]
    async fn data_hello_with_foreign_cert_rejected() {
        let (_mgr_a, mut events_a, _dev_a, _cert_a) = make_manager_with_cert().await;
        let (mgr_b, mut events_b, _dev_b, _cert_b) = make_manager_with_cert().await;
        // B 注册 handler，若被调用则测试失败。
        let (data_tx, mut data_rx) = mpsc::unbounded_channel();
        let data_tx_for_handler = data_tx.clone();
        mgr_b.set_data_handler(Arc::new(move |peer, _stream| {
            let _ = data_tx_for_handler.send(peer.device);
        }));

        // 手工构造数据连接：TLS 用攻击者自己的设备密钥，DataHello 却带别人的证书。
        let attacker_key = DeviceKey::generate();
        let victim_cert = make_cert_for(DeviceKey::generate().device_id());
        let stream = TcpStream::connect(mgr_b.local_addr())
            .await
            .expect("拨号 B 失败");
        let (_server_config, client_config) =
            make_tls_configs(&attacker_key).expect("TLS 配置失败");
        let domain = ServerName::try_from(TLS_SNI.as_bytes()).expect("非法 SNI");
        let mut tls = tokio_rustls::TlsStream::from(
            TlsConnector::from(client_config)
                .connect(domain, stream)
                .await
                .expect("TLS 握手失败"),
        );
        tls.write_all(&[CHANNEL_DATA])
            .await
            .expect("写通道字节失败");
        lanmsg_proto::data::write_data_frame(&mut tls, &DataFrame::DataHello { cert: victim_cert })
            .await
            .expect("写 DataHello 复赛 失败");

        // B 的 handler 不应被调用（500ms 内无事件），A' 也不应有 Up。
        match tokio::time::timeout(Duration::from_millis(500), data_rx.recv()).await {
            Err(_) => {} // 超时 = 未被调用，通过
            Ok(Some(d)) => panic!("假证书数据连接的 handler 不应被调用，却收到 {:?}", d),
            Ok(None) => panic!("handler 通道提前关闭"),
        }
        expect_no_up(&mut events_b).await;
        expect_no_up(&mut events_a).await;
    }

    /// 两个已互连、带证书的管理器（A 拨 B）。
    async fn connected_pair() -> (ConnManager, ConnManager, DeviceId, DeviceId) {
        let (mgr_a, mut events_a, dev_a, _ca) = make_manager_with_cert().await;
        let (mgr_b, mut events_b, dev_b, _cb) = make_manager_with_cert().await;
        mgr_a.connect(mgr_b.local_addr(), false).await.unwrap();
        expect_up(&mut events_a, dev_b).await;
        expect_up(&mut events_b, dev_a).await;
        // 事件通道不再被读取时发送端会阻塞，后台持续排空。
        tokio::spawn(async move { while events_a.recv().await.is_some() {} });
        tokio::spawn(async move { while events_b.recv().await.is_some() {} });
        (mgr_a, mgr_b, dev_a, dev_b)
    }

    /// 令牌由设备 A 登记，等待者交给 A；B 拨回出示令牌后 A 拿到数据流，令牌随即作废。
    #[tokio::test]
    async fn dial_back_token_single_use() {
        let (mgr_a, mgr_b, dev_a, dev_b) = connected_pair().await;
        let table = mgr_a.read_inner().dial_back.clone();
        let (token, rx) = table.register(dev_b, DIAL_BACK_WAIT);

        // 第一次：B 拨回，A 的等待者收到流。
        let _stream = mgr_b.dial_back_stream(&dev_a, token).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), rx).await;
        assert!(matches!(got, Ok(Ok(_))), "waiter should receive the stream");

        // 第二次：同一令牌已作废，A 关闭连接，B 读不到数据。
        let mut stream2 = mgr_b.dial_back_stream(&dev_a, token).await.unwrap();
        use tokio::io::AsyncReadExt;
        let mut buf = [0u8; 1];
        let r = tokio::time::timeout(Duration::from_secs(5), stream2.read(&mut buf)).await;
        assert!(
            matches!(r, Ok(Ok(0)) | Ok(Err(_))),
            "reused token must be rejected, got {r:?}"
        );
    }

    /// 令牌登记给另一台设备时，B 出示该令牌被拒绝，且不消耗令牌。
    #[tokio::test]
    async fn dial_back_rejects_wrong_device() {
        let (mgr_a, mgr_b, dev_a, _dev_b) = connected_pair().await;
        let table = mgr_a.read_inner().dial_back.clone();
        let other = DeviceId([0x55; 32]);
        let (token, mut rx) = table.register(other, DIAL_BACK_WAIT);

        let mut stream = mgr_b.dial_back_stream(&dev_a, token).await.unwrap();
        use tokio::io::AsyncReadExt;
        let mut buf = [0u8; 1];
        let r = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await;
        assert!(
            matches!(r, Ok(Ok(0)) | Ok(Err(_))),
            "foreign token must be rejected, got {r:?}"
        );
        assert!(rx.try_recv().is_err(), "waiter must not receive a stream");
        // 令牌仍属于原设备（未被第三方烧掉）。
        assert!(table.claim(&token, &other).is_some());
    }
}
