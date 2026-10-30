// P1 核心骨架：连接管理器（Task 9）。
// 维护每对设备一条控制连接：监听入站、出站拨号、Hello 校验、重复连接裁决、帧事件分发。
// 注释中文，标识符英文。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch, RwLock};

use crate::net::conn::{
    Accepted, ConnEvent, Connection, DataHandler, DataStream, HelloInfo, PeerInfo,
};
use crate::net::tls::{make_tls_configs, TLS_SNI};
use lanmsg_proto::consts::CHANNEL_DATA;
use lanmsg_proto::data::{write_data_frame, DataFrame};
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::sign::DeviceKey;
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
}

/// 控制连接管理器（Clone，内部 Arc）。
#[derive(Clone)]
pub struct ConnManager {
    local_addr: SocketAddr,
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    connections: HashMap<DeviceId, ConnectionEntry>,
    events: mpsc::Sender<ConnEvent>,
    hello_rx: watch::Receiver<HelloInfo>,
    accept_filter: Arc<dyn Fn(&PeerInfo) -> bool + Send + Sync>,
    data_handler: Option<DataHandler>,
    self_device: DeviceId,
    device_key: Arc<DeviceKey>,
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
        let identity_guard = identity.read().await;
        let device_key = DeviceKey::from_bytes(&identity_guard.device.to_bytes());
        let self_device = identity_guard.device_id();
        drop(identity_guard);

        let listener = if port == 0 {
            TcpListener::bind("0.0.0.0:0")
                .await
                .map_err(CoreError::Io)?
        } else {
            match TcpListener::bind(format!("0.0.0.0:{port}")).await {
                Ok(l) => l,
                Err(_) => {
                    tracing::warn!(port, "端口被占用，回退到随机端口");
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
        };

        let manager = ConnManager {
            local_addr,
            inner: Arc::new(RwLock::new(inner)),
        };

        // 启动 accept 循环。
        let listen_port = manager.local_addr.port();
        let inner_clone = manager.inner.clone();
        tokio::spawn(async move {
            accept_loop(listener, listen_port, inner_clone).await;
        });

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
        // 快速路径：若已有连接到该地址且已上线，直接返回该设备。
        {
            let inner = self.inner.read().await;
            for (device, entry) in inner.connections.iter() {
                if entry.peer_addr == addr && entry.up_sent {
                    return Ok(*device);
                }
            }
        }

        let hello = self.build_hello().await;
        let (peer_info, connection) = Connection::connect_outbound(
            addr,
            &*self.device_key().await,
            self.self_device().await,
            &hello,
            self.events_clone().await,
        )
        .await?;

        let device = peer_info.device;
        let peer_addr = peer_info.addr;
        let self_dev = self.self_device().await;

        // 重复连接裁决：同一设备已有连接时，保留"拨号方 DeviceId 较小"的那条。
        let mut inner = self.inner.write().await;
        if let Some(existing) = inner.connections.get(&device) {
            let existing_dialer = existing.dialer_device;
            if existing_dialer <= self_dev {
                // 保留旧连接，关闭新连接（不产生 Up/Down）。
                let _ = connection.shutdown().await;
                return Ok(device);
            } else {
                // 新连接拨号方较小，替换旧连接（旧连接已发 Up，此处不再发 Up 避免重复）。
                let _ = existing.connection.shutdown().await;
                inner.connections.remove(&device);
                connection.set_up_sent();
                inner.connections.insert(
                    device,
                    ConnectionEntry {
                        connection,
                        peer_addr,
                        dialer_device: self_dev,
                        up_sent: true,
                    },
                );
                return Ok(device);
            }
        }

        connection.set_up_sent();
        let _ = inner.events.send(ConnEvent::Up(peer_info.clone())).await;
        inner.connections.insert(
            device,
            ConnectionEntry {
                connection,
                peer_addr,
                dialer_device: self_dev,
                up_sent: true,
            },
        );

        Ok(device)
    }

    /// 发送帧：未连接 → `CoreError::PeerOffline`。
    pub async fn send(&self, device: &DeviceId, frame: Frame) -> Result<(), CoreError> {
        let inner = self.inner.read().await;
        match inner.connections.get(device) {
            Some(entry) => entry.connection.send(device, frame).await,
            None => Err(CoreError::PeerOffline),
        }
    }

    /// 所有已连接对端的 PeerInfo。
    pub fn peers(&self) -> Vec<PeerInfo> {
        let inner = self.inner.try_read().ok();
        match inner {
            Some(inner) => inner
                .connections
                .values()
                .filter(|e| e.up_sent)
                .map(|e| e.connection.peer_info().clone())
                .collect(),
            None => Vec::new(),
        }
    }

    /// 是否已与指定设备建立连接。
    pub fn is_connected(&self, device: &DeviceId) -> bool {
        let inner = self.inner.try_read().ok();
        match inner {
            Some(inner) => inner
                .connections
                .get(device)
                .map(|e| e.up_sent)
                .unwrap_or(false),
            None => false,
        }
    }

    /// 断开与指定设备的连接。
    pub async fn disconnect(&self, device: &DeviceId) {
        let mut inner = self.inner.write().await;
        if let Some(entry) = inner.connections.remove(device) {
            let _ = entry.connection.shutdown().await;
        }
    }

    /// 关闭所有连接与监听器。
    pub async fn shutdown(&self) {
        let mut inner = self.inner.write().await;
        for (_device, entry) in inner.connections.drain() {
            let _ = entry.connection.shutdown().await;
        }
    }

    // ---- 私有辅助 ----

    /// 注册数据连接处理器（SyncEngine 等模块在启动时调用）。
    ///
    /// 约定在服务启动早期、无并发持锁时调用；写锁被占用说明违反该约定，
    /// 按 fail-fast 直接 panic。
    pub fn set_data_handler(&self, h: DataHandler) {
        let mut inner = self
            .inner
            .try_write()
            .expect("set_data_handler 需要写锁空闲：应在服务启动早期调用");
        inner.data_handler = Some(h);
    }

    /// 数据连接：向 `peer.dial_addr` 拨号（不用 `addr`：被拨方看到的是对端
    /// 临时源端口），TLS 后写通道字节 CHANNEL_DATA 与 DataHello。
    pub async fn open_data(&self, device: &DeviceId) -> Result<DataStream, CoreError> {
        let peer = self.peer(device).ok_or(CoreError::PeerOffline)?;
        let addr = peer.dial_addr;
        let device_key = self.device_key().await;
        // 未初始化设备没有证书，无法自证身份，数据通道不可用。
        let own_cert = {
            let inner = self.inner.read().await;
            let cert = inner.hello_rx.borrow().cert.clone();
            cert
        }
        .ok_or_else(|| CoreError::Rejected("本机身份未初始化，数据通道不可用".into()))?;

        let stream = TcpStream::connect(addr).await.map_err(CoreError::Io)?;
        let (_server_config, client_config) = make_tls_configs(&device_key)?;
        let domain = ServerName::try_from(TLS_SNI.as_bytes())
            .map_err(|e| CoreError::Tls(format!("非法 SNI: {e}")))?;
        let mut tls_stream = tokio_rustls::TlsStream::from(
            TlsConnector::from(client_config)
                .connect(domain, stream)
                .await
                .map_err(|e| CoreError::Tls(e.to_string()))?,
        );
        tls_stream
            .write_all(&[CHANNEL_DATA])
            .await
            .map_err(CoreError::Io)?;
        write_data_frame(&mut tls_stream, &DataFrame::DataHello { cert: own_cert })
            .await
            .map_err(|e| CoreError::Tls(e.to_string()))?;
        Ok(tls_stream)
    }

    /// 查询已上线对端的信息（含 tls_exporter 与 dial_addr）。
    pub fn peer(&self, device: &DeviceId) -> Option<PeerInfo> {
        let inner = self.inner.try_read().ok()?;
        inner
            .connections
            .get(device)
            .filter(|e| e.up_sent)
            .map(|e| e.connection.peer_info().clone())
    }

    async fn build_hello(&self) -> HelloInfo {
        let hello_info = self.inner.read().await.hello_rx.borrow().clone();
        HelloInfo {
            display_name: hello_info.display_name,
            cert: hello_info.cert,
            listen_port: self.local_addr.port(),
        }
    }

    async fn device_key(&self) -> Arc<DeviceKey> {
        self.inner.read().await.device_key.clone()
    }

    async fn self_device(&self) -> DeviceId {
        self.inner.read().await.self_device
    }

    async fn events_clone(&self) -> mpsc::Sender<ConnEvent> {
        self.inner.read().await.events.clone()
    }
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
            let (self_device, device_key, accept_filter, data_handler, events) = {
                let guard = inner_clone.read().await;
                (
                    guard.self_device,
                    DeviceKey::from_bytes(&guard.device_key.to_bytes()),
                    guard.accept_filter.clone(),
                    guard.data_handler.clone(),
                    guard.events.clone(),
                )
            };

            let hello_info = {
                let guard = inner_clone.read().await;
                let borrowed = guard.hello_rx.borrow();
                HelloInfo {
                    display_name: borrowed.display_name.clone(),
                    cert: borrowed.cert.clone(),
                    listen_port,
                }
            };

            match Connection::accept_channel(
                stream,
                &device_key,
                self_device,
                &hello_info,
                accept_filter.as_ref(),
                events,
                data_handler,
            )
            .await
            {
                Ok(Accepted::Control(peer_info, connection)) => {
                    let device = peer_info.device;
                    let mut guard = inner_clone.write().await;
                    // 重复连接裁决：入站连接的拨号方为 peer。
                    if let Some(existing) = guard.connections.get(&device) {
                        let existing_dialer = existing.dialer_device;
                        let new_dialer = peer_info.device;
                        if existing_dialer <= new_dialer {
                            let _ = connection.shutdown().await;
                            return;
                        } else {
                            let _ = existing.connection.shutdown().await;
                            guard.connections.remove(&device);
                        }
                    }
                    connection.set_up_sent();
                    let _ = guard.events.send(ConnEvent::Up(peer_info.clone())).await;
                    guard.connections.insert(
                        device,
                        ConnectionEntry {
                            connection,
                            peer_addr: peer_info.addr,
                            dialer_device: peer_info.device,
                            up_sent: true,
                        },
                    );
                }
                Ok(Accepted::Data(peer_info)) => {
                    // 数据连接已交给注册的处理器，无需登记；仅记录审计日志。
                    tracing::debug!(device = %peer_info.device, "数据连接已建立");
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
    use std::time::Duration;
    use tokio::sync::{mpsc, watch, RwLock};

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
    #[allow(dead_code)] // 数据测试分批使用
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
}
