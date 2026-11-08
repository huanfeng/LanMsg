// P1 核心骨架：局域网发现（C6）。
// Discovery 统一收发设备通告：接收路径做 CBOR 解析、验签、协议版本与自身过滤后发出 DiscoveryEvent；
// 发送路径由 DiscoveryMode 选择 UDP 组播或内存总线。注释中文，标识符英文。

pub mod memory;
pub mod multicast;
pub mod unicast;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch, Notify};
use tokio::time::interval_at;

use lanmsg_proto::consts::{ANNOUNCE_INTERVAL, PROTO_VERSION};
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::sign::{from_cbor, to_cbor};
use lanmsg_proto::types::Announce;

pub use memory::MemoryBus;

/// 收到某设备通告后单播回应的最小间隔，冷却期内不重复回应。
const REPLY_COOLDOWN: Duration = Duration::from_secs(10);

/// 内部发包指令：`to` 为 None 表示组播泛洪（Memory 模式等价于总线广播），Some(addr) 表示单播。
pub(crate) struct OutPkt {
    pub(crate) bytes: Vec<u8>,
    pub(crate) to: Option<SocketAddr>,
}

/// 发现模式（C6）。
pub enum DiscoveryMode {
    /// UDP 组播（生产默认）。
    Multicast,
    /// 内存总线（测试与同机多实例）。
    Memory(MemoryBus),
    /// 关闭发现。
    Disabled,
}

/// 发现事件：一条通过校验的外部通告。
pub struct DiscoveryEvent {
    pub announce: Announce,
    pub from: SocketAddr,
    pub via_multicast: bool,
}

/// 发现表条目的保留时长（配对 Device 目标与后续任务使用）。
const DISCOVERED_TTL: Duration = Duration::from_secs(10 * 60);

/// 近期发现的设备表（P2 Task 7）：值为 (通告, 连接地址, 记录时间, 是否经组播发现)。
/// 连接地址 = 发现源 IP + 通告监听端口。
pub struct DiscoveredTable(Mutex<HashMap<DeviceId, (Announce, SocketAddr, Instant, bool)>>);

impl Default for DiscoveredTable {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscoveredTable {
    /// 建空表。
    pub fn new() -> DiscoveredTable {
        DiscoveredTable(Mutex::new(HashMap::new()))
    }

    /// 记录一条发现事件（本机未初始化时也记录）；顺带清理过期条目。
    pub fn record(&self, ev: &DiscoveryEvent) {
        let port = ev.announce.ports.first().copied().unwrap_or(0);
        let addr = if port == 0 {
            ev.from
        } else {
            SocketAddr::new(ev.from.ip(), port)
        };
        let mut g = self.0.lock().unwrap();
        let now = Instant::now();
        g.retain(|_, (_, _, t, _)| now.duration_since(*t) < DISCOVERED_TTL);
        g.insert(
            ev.announce.cert.device,
            (ev.announce.clone(), addr, now, ev.via_multicast),
        );
    }

    /// 取某设备的最近通告与连接地址。
    pub fn get(&self, d: &DeviceId) -> Option<(Announce, SocketAddr)> {
        let g = self.0.lock().unwrap();
        g.get(d).map(|(a, s, _, _)| (a.clone(), *s))
    }

    /// 取某设备的连接地址与该条目的年龄（距最近一次收到其通告）。
    pub fn addr_with_age(&self, d: &DeviceId) -> Option<(SocketAddr, Duration)> {
        let g = self.0.lock().unwrap();
        g.get(d).map(|(_, s, t, _)| (*s, t.elapsed()))
    }

    /// 列出 `within` 内记录的设备：(通告, 连接地址, 是否经组播发现)。
    pub fn recent(&self, within: Duration) -> Vec<(Announce, SocketAddr, bool)> {
        let g = self.0.lock().unwrap();
        let now = Instant::now();
        g.values()
            .filter(|(_, _, t, _)| now.duration_since(*t) <= within)
            .map(|(a, s, _, via)| (a.clone(), *s, *via))
            .collect()
    }
}

/// 发现任务的句柄。
pub struct Discovery {
    shutdown: Arc<Notify>,
    /// 单播通告目标（仅组播模式生效）。
    unicast: Arc<Mutex<Vec<SocketAddr>>>,
}

impl Discovery {
    /// 启动发现任务。
    ///
    /// `announce_rx` 的值为 `None` 时不发送任何通告（未初始化或隐身）；
    /// 值变化时立即通告一次；此后每 `ANNOUNCE_INTERVAL` 周期通告。
    pub fn spawn(
        mode: DiscoveryMode,
        self_device: DeviceId,
        announce_rx: watch::Receiver<Option<Announce>>,
        events: mpsc::Sender<DiscoveryEvent>,
    ) -> Discovery {
        let shutdown = Arc::new(Notify::new());
        let (inbox_tx, inbox_rx) = mpsc::channel::<(Vec<u8>, SocketAddr)>(64);
        let (outbox_tx, outbox_rx) = mpsc::channel::<OutPkt>(64);
        let via_multicast = matches!(mode, DiscoveryMode::Multicast);
        match mode {
            DiscoveryMode::Multicast => {
                tokio::spawn(multicast::run(
                    announce_rx.clone(),
                    inbox_tx,
                    outbox_rx,
                    shutdown.clone(),
                ));
            }
            DiscoveryMode::Memory(bus) => {
                memory::spawn_source(&bus, inbox_tx);
                tokio::spawn(memory::run_sink(bus, announce_rx.clone(), outbox_rx));
            }
            DiscoveryMode::Disabled => {} // 收发通道随即关闭，主循环直接退出
        }
        let unicast = Arc::new(Mutex::new(Vec::new()));
        tokio::spawn(main_loop(
            self_device,
            announce_rx,
            events,
            inbox_rx,
            outbox_tx,
            via_multicast,
            unicast.clone(),
            shutdown.clone(),
        ));
        Discovery { shutdown, unicast }
    }

    /// 设置单播通告目标：之后每次周期通告，除组播外还向每个目标单播同一份通告字节。
    /// 内存总线与关闭模式下为空操作。
    pub fn set_unicast_targets(&self, targets: Vec<SocketAddr>) {
        *self.unicast.lock().unwrap_or_else(|e| e.into_inner()) = targets;
    }

    /// 请求发现任务停止；任务在下一个轮询点退出。
    pub fn shutdown(&self) {
        self.shutdown.notify_one();
    }
}

/// 发现主循环：通告发送（启动、值变化、周期）、包处理与回应。
#[allow(clippy::too_many_arguments)] // 发现主循环的各通道与共享状态，拆开反而更绕。
async fn main_loop(
    self_device: DeviceId,
    mut announce_rx: watch::Receiver<Option<Announce>>,
    events: mpsc::Sender<DiscoveryEvent>,
    mut inbox_rx: mpsc::Receiver<(Vec<u8>, SocketAddr)>,
    outbox: mpsc::Sender<OutPkt>,
    via_multicast: bool,
    unicast: Arc<Mutex<Vec<SocketAddr>>>,
    shutdown: Arc<Notify>,
) {
    // 启动时立即通告一次（值存在时）。
    if announce_rx.borrow_and_update().is_some() {
        send_announce(
            &announce_rx,
            &outbox,
            &current_targets(&unicast, via_multicast),
        )
        .await;
    }
    let mut ticker = interval_at(
        tokio::time::Instant::now() + ANNOUNCE_INTERVAL,
        ANNOUNCE_INTERVAL,
    );
    let mut last_replied: HashMap<DeviceId, std::time::Instant> = HashMap::new();
    loop {
        tokio::select! {
            _ = shutdown.notified() => break,
            changed = announce_rx.changed() => {
                if changed.is_err() {
                    break; // 所有 watch 发送端已关闭，任务退出
                }
                if announce_rx.borrow().is_some() {
                    send_announce(&announce_rx, &outbox, &current_targets(&unicast, via_multicast)).await;
                }
            }
            _ = ticker.tick() => {
                if announce_rx.borrow().is_some() {
                    send_announce(&announce_rx, &outbox, &current_targets(&unicast, via_multicast)).await;
                }
            }
            pkt = inbox_rx.recv() => {
                let (bytes, from) = match pkt {
                    Some(p) => p,
                    None => break, // 收件箱关闭（模式任务已退出）
                };
                on_packet(
                    &bytes[..], from, via_multicast, self_device,
                    announce_rx.clone(), events.clone(), &mut last_replied, &outbox,
                ).await;
            }
        }
    }
}

/// 取当前单播目标（只有组播模式才发单播通告）。
fn current_targets(unicast: &Mutex<Vec<SocketAddr>>, via_multicast: bool) -> Vec<SocketAddr> {
    if via_multicast {
        unicast.lock().unwrap_or_else(|e| e.into_inner()).clone()
    } else {
        Vec::new()
    }
}

/// 一次周期通告要发的包：一份组播，外加向每个单播目标发同一份字节。
fn announce_packets(own: &Announce, targets: &[SocketAddr]) -> Vec<OutPkt> {
    let bytes = to_cbor(own);
    let mut pkts = vec![OutPkt {
        bytes: bytes.clone(),
        to: None,
    }];
    pkts.extend(targets.iter().map(|t| OutPkt {
        bytes: bytes.clone(),
        to: Some(*t),
    }));
    pkts
}

/// 当前通告存在时，以组播（`to: None`）并向单播目标发出一份自己的通告。
async fn send_announce(
    announce_rx: &watch::Receiver<Option<Announce>>,
    outbox: &mpsc::Sender<OutPkt>,
    targets: &[SocketAddr],
) {
    // 先把通告值 clone 出来，释放 watch::Ref（非 Send），再做 await。
    let own = announce_rx.borrow().as_ref().cloned();
    if let Some(own) = own {
        for pkt in announce_packets(&own, targets) {
            let _ = outbox.send(pkt).await;
        }
    }
}

/// 处理一包：解析 → 验签 → 过滤 → 发事件 → 按冷却期单播回应。
#[allow(clippy::too_many_arguments)] // P1 结构性：包处理集中入口。
async fn on_packet(
    bytes: &[u8],
    from: SocketAddr,
    via_multicast: bool,
    self_device: DeviceId,
    announce_rx: watch::Receiver<Option<Announce>>,
    events: mpsc::Sender<DiscoveryEvent>,
    last_replied: &mut HashMap<DeviceId, std::time::Instant>,
    outbox: &mpsc::Sender<OutPkt>,
) {
    let ann: Announce = match from_cbor(bytes) {
        Ok(a) => a,
        Err(e) => {
            tracing::debug!(peer = %from, error = %e, "announce CBOR decode failed, dropped");
            return;
        }
    };
    if let Err(e) = ann.verify() {
        tracing::debug!(peer = %from, error = %e, "announce signature verification failed, dropped");
        return;
    }
    if ann.proto != PROTO_VERSION {
        tracing::debug!(peer = %from, proto = ann.proto, "announce protocol version mismatch, dropped");
        return;
    }
    if ann.cert.device == self_device {
        tracing::debug!(peer = %from, "received own announce, dropped");
        return;
    }
    // 来自本机网段之外的包只能是单播（用户配置的跨网段目标及其回应），不算组播。
    let via_multicast = via_multicast
        && crate::net::path::on_local_subnet(from.ip(), &crate::net::path::cached_ifaces());
    if events
        .send(DiscoveryEvent {
            announce: ann.clone(),
            from,
            via_multicast,
        })
        .await
        .is_err()
    {
        tracing::debug!("discovery event receiver closed, event dropped");
    }
    // 首次（或距上次回应超过冷却期）时，向 from 单播回一份自己的通告。
    let now = std::time::Instant::now();
    let should_reply = match last_replied.get(&ann.cert.device) {
        Some(t) => now.duration_since(*t) > REPLY_COOLDOWN,
        None => true,
    };
    if should_reply {
        // 先把通告值 clone 出来，释放 watch::Ref（非 Send），再做 await。
        let own = announce_rx.borrow().as_ref().cloned();
        if let Some(own) = own {
            let _ = outbox
                .send(OutPkt {
                    bytes: to_cbor(&own),
                    to: Some(from),
                })
                .await;
        }
        last_replied.insert(ann.cert.device, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::{mpsc, watch};

    use lanmsg_proto::consts::PROTO_VERSION;
    use lanmsg_proto::sign::{sign_with_zeroed_sig, to_cbor, DeviceKey, UserKey};
    use lanmsg_proto::types::{Announce, DeviceCert, DeviceType, Signature64, Status};

    /// 构造合法通告：生成 UserKey/DeviceKey → 签发设备证书 → 签通告。
    fn test_announce(display: &str, port: u16) -> Announce {
        let user = UserKey::generate();
        let dev = DeviceKey::generate();
        let cert = DeviceCert::issue(
            &user,
            dev.device_id(),
            format!("{display}-设备"),
            DeviceType::Desktop,
            1_700_000_000_000,
        );
        let mut ann = Announce {
            proto: PROTO_VERSION,
            cert,
            display_name: display.into(),
            avatar_hash: None,
            ports: vec![port],
            status: Status::Online,
            sig: Signature64([0u8; 64]),
        };
        sign_with_zeroed_sig(&mut ann, |b| dev.sign(b));
        ann
    }

    /// 断言在时限内收不到任何发现事件。
    async fn expect_silent(rx: &mut mpsc::Receiver<DiscoveryEvent>, ms: u64) {
        match tokio::time::timeout(Duration::from_millis(ms), rx.recv()).await {
            Err(_elapsed) => {} // 超时即静默，符合预期
            Ok(Some(evt)) => panic!("不应收到发现事件，却收到来自 {}", evt.from),
            Ok(None) => panic!("发现事件通道被提前关闭"),
        }
    }

    #[tokio::test]
    async fn memory_bus_delivers_announce_to_other_node() {
        let bus = MemoryBus::new();
        let ann_a = test_announce("甲机", 40001);
        let ann_b = test_announce("乙机", 40002);
        let a_device = ann_a.cert.device;
        let b_device = ann_b.cert.device;
        let (tx_a, rx_a) = watch::channel(None);
        tx_a.send_replace(Some(ann_a));
        let (tx_b, rx_b) = watch::channel(None);
        tx_b.send_replace(Some(ann_b));
        let (_evt_a_tx, _evt_a_rx) = mpsc::channel(32);
        let (evt_b_tx, mut evt_b_rx) = mpsc::channel(32);
        let _a = Discovery::spawn(
            DiscoveryMode::Memory(bus.clone()),
            a_device,
            rx_a,
            _evt_a_tx,
        );
        let _b = Discovery::spawn(DiscoveryMode::Memory(bus), b_device, rx_b, evt_b_tx);
        let evt = tokio::time::timeout(Duration::from_secs(5), evt_b_rx.recv())
            .await
            .expect("5 秒内未收到对端通告")
            .expect("发现事件通道被提前关闭");
        assert_eq!(evt.announce.cert.device, a_device);
        assert!(!evt.via_multicast);
    }

    #[tokio::test]
    async fn ignores_tampered_and_self_announces() {
        let bus = MemoryBus::new();
        let ann_a = test_announce("甲机", 40011);
        let ann_b = test_announce("乙机", 40012);
        let (_tx_a, rx_a) = watch::channel(None);
        let (_tx_b, rx_b) = watch::channel(None);
        let (evt_a_tx, mut evt_a_rx) = mpsc::channel(32);
        let (evt_b_tx, mut evt_b_rx) = mpsc::channel(32);
        // 阶段一：仅 A 在跑，注入自身的合法通告与篡改通告，A 应静默。
        let _a = Discovery::spawn(
            DiscoveryMode::Memory(bus.clone()),
            ann_a.cert.device,
            rx_a,
            evt_a_tx,
        );
        bus.broadcast(to_cbor(&ann_a), SocketAddr::from(([127, 0, 0, 1], 40011)));
        let mut tampered = ann_b.clone();
        tampered.display_name = "冒名".into();
        bus.broadcast(
            to_cbor(&tampered),
            SocketAddr::from(([127, 0, 0, 1], 40012)),
        );
        expect_silent(&mut evt_a_rx, 300).await;
        // 阶段二：B 上线（订阅晚于前面的注入，收不到旧包），再注入篡改通告，B 应静默。
        let _b = Discovery::spawn(
            DiscoveryMode::Memory(bus.clone()),
            ann_b.cert.device,
            rx_b,
            evt_b_tx,
        );
        bus.broadcast(
            to_cbor(&tampered),
            SocketAddr::from(([127, 0, 0, 1], 40012)),
        );
        expect_silent(&mut evt_b_rx, 500).await;
    }

    #[tokio::test]
    async fn none_announce_sends_nothing() {
        let bus = MemoryBus::new();
        let (_tx_a, rx_a) = watch::channel(None);
        let (_tx_b, rx_b) = watch::channel(None);
        let (evt_a_tx, _evt_a_rx) = mpsc::channel(32);
        let (evt_b_tx, mut evt_b_rx) = mpsc::channel(32);
        let _a = Discovery::spawn(
            DiscoveryMode::Memory(bus.clone()),
            test_announce("甲机", 40021).cert.device,
            rx_a,
            evt_a_tx,
        );
        let _b = Discovery::spawn(
            DiscoveryMode::Memory(bus),
            test_announce("乙机", 40022).cert.device,
            rx_b,
            evt_b_tx,
        );
        expect_silent(&mut evt_b_rx, 500).await;
    }

    #[test]
    fn announce_packets_add_unicast_per_target() {
        let ann = test_announce("A", 7000);
        let targets: Vec<SocketAddr> = vec![
            "10.1.1.1:53320".parse().unwrap(),
            "10.1.1.2:53320".parse().unwrap(),
        ];
        let pkts = announce_packets(&ann, &targets);
        assert_eq!(pkts.len(), 3);
        assert!(pkts[0].to.is_none());
        assert_eq!(pkts[1].to, Some(targets[0]));
        assert_eq!(pkts[2].to, Some(targets[1]));
        assert!(pkts.iter().all(|p| p.bytes == to_cbor(&ann)));
        // 无目标时只有组播一份。
        assert_eq!(announce_packets(&ann, &[]).len(), 1);
    }

    #[test]
    fn memory_mode_ignores_unicast_targets() {
        let unicast = Mutex::new(vec!["10.1.1.1:53320".parse().unwrap()]);
        assert!(current_targets(&unicast, false).is_empty());
        assert_eq!(current_targets(&unicast, true).len(), 1);
    }

    /// 发现表记录来源方式；单播来源的跨网段条目在发现视图里不是 Lan，组播来源才是。
    #[test]
    fn unicast_discovery_entry_is_not_lan_in_view() {
        use lanmsg_api::types::PathClassDto;
        let table = DiscoveredTable::new();
        let ann = test_announce("远端", 53320);
        // 公网地址不会落在任何本机网段里，结果与测试机网卡无关。
        let from: SocketAddr = "203.0.113.7:40000".parse().unwrap();
        table.record(&DiscoveryEvent {
            announce: ann.clone(),
            from,
            via_multicast: false,
        });
        let got = table.recent(Duration::from_secs(60));
        assert_eq!(got.len(), 1);
        let (a, addr, via) = &got[0];
        assert!(!via);
        let store = crate::store::Store::open_in_memory().unwrap();
        let unicast = crate::service::views::discovered_peer_view(&store, a, *addr, *via, None);
        assert_ne!(unicast.path, PathClassDto::Lan);
        let multicast = crate::service::views::discovered_peer_view(&store, a, *addr, true, None);
        assert_eq!(multicast.path, PathClassDto::Lan);
    }
}
