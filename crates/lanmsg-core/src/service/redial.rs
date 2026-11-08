// 受信对端的主动重连：已配对设备（自己的设备与已接受联系人）断开后，
// 不再只等组播通告，而是按最近已知地址拨号，失败按 1s..60s 指数退避（带抖动）。
// 双方同时拨号由 ConnManager 的重复连接裁决收敛（保留拨号方 DeviceId 较小的一条）。
// 注释中文，标识符英文。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lanmsg_proto::consts::{ANNOUNCE_INTERVAL, RETRY_MIN};
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::types::Status;
use tokio::sync::{watch, Notify};

use super::{DialingGuard, Inner};
use crate::net::path::{local_ifaces, on_local_subnet as ip_on_local_subnet};
use crate::store::ContactState;

/// 受信重连退避上限：设备不在当前网络时不必高频拨号（耗电）。
/// 网络变化或收到该设备的通告会立即恢复拨号。
const REDIAL_MAX: Duration = Duration::from_secs(5 * 60);

/// 连接稳定存活超过此时长才清零退避；更短视为抖动，继续沿用退避。
const STABLE_AFTER: Duration = Duration::from_secs(30);

/// 已连接 / 暂不允许外联时的兜底轮询间隔（断开与发现事件会直接唤醒）。
const IDLE_POLL: Duration = Duration::from_secs(10);

/// 网络变化去抖：窗口内的多次通知合并成一次（窗口结束时触发）。
const NET_DEBOUNCE: Duration = Duration::from_secs(2);

/// 发现表条目在此窗口内才视为比持久化地址更可信（3 个通告周期）。
const FRESH_DISCOVERY: Duration = Duration::from_secs(3 * ANNOUNCE_INTERVAL.as_secs());

/// 每个重连任务的专属信号：唤醒、是否要求清零退避、最近一次发现地址。
struct TaskSignals {
    notify: Notify,
    reset: AtomicBool,
    /// 自上次消费以来收到过该设备的发现事件（区别于断开事件的唤醒）。
    discovered: AtomicBool,
    last_discovered: Mutex<Option<SocketAddr>>,
}

impl TaskSignals {
    fn new() -> Self {
        Self {
            notify: Notify::new(),
            reset: AtomicBool::new(false),
            discovered: AtomicBool::new(false),
            last_discovered: Mutex::new(None),
        }
    }

    /// 收到该设备的发现事件：只唤醒；仅当发现地址相对上次变了才要求清零退避
    /// （否则每 30 秒一次的通告会让退避上限永远用不上）。返回是否要求清零。
    fn on_discovered(&self, addr: Option<SocketAddr>) -> bool {
        let changed = {
            let mut last = self.last_discovered.lock().unwrap();
            let changed = addr.is_some() && *last != addr;
            if changed {
                *last = addr;
            }
            changed
        };
        if changed {
            self.reset.store(true, Ordering::SeqCst);
        }
        self.discovered.store(true, Ordering::SeqCst);
        self.notify.notify_one();
        changed
    }
}

/// 重连状态：每个受信设备一个任务（带专属信号），以及网络变化信号与计数。
pub(crate) struct RedialState {
    tasks: Mutex<HashMap<DeviceId, Arc<TaskSignals>>>,
    loops: AtomicU64,
    /// 网络变化（已去抖）：清零所有退避，立即重拨。
    net: watch::Sender<u64>,
    net_pending: AtomicBool,
    attempts: AtomicU64,
}

impl RedialState {
    pub(crate) fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            loops: AtomicU64::new(0),
            net: watch::channel(0).0,
            net_pending: AtomicBool::new(false),
            attempts: AtomicU64::new(0),
        }
    }

    pub(crate) fn attempts(&self) -> u64 {
        self.attempts.load(Ordering::SeqCst)
    }

    /// 重连任务循环迭代总数（测试用：检测空转）。
    pub(crate) fn loop_iterations(&self) -> u64 {
        self.loops.load(Ordering::SeqCst)
    }

    /// 已触发的（去抖后）网络变化次数。
    pub(crate) fn net_signals(&self) -> u64 {
        *self.net.borrow()
    }
}

/// 第 `attempt` 次失败后的等待时长：RETRY_MIN * 2^attempt，封顶 REDIAL_MAX，
/// 再乘以 [0.5, 1.0] 的抖动（`jitter` 取 0..=1），避免双方同步重拨。
pub(crate) fn backoff(attempt: u32, jitter: f64) -> Duration {
    let base = RETRY_MIN
        .checked_mul(1u32.checked_shl(attempt).unwrap_or(u32::MAX))
        .unwrap_or(REDIAL_MAX)
        .min(REDIAL_MAX);
    base.mul_f64(0.5 + 0.5 * jitter.clamp(0.0, 1.0))
}

/// 选拨号地址：发现表条目足够新（刚收到通告）就用它，否则用持久化地址；
/// 持久化地址也没有时，陈旧的发现条目好过没有。
pub(crate) fn pick_addr(
    discovered: Option<(SocketAddr, Duration)>,
    stored: Option<SocketAddr>,
) -> Option<SocketAddr> {
    match (discovered, stored) {
        (Some((addr, age)), _) if age <= FRESH_DISCOVERY => Some(addr),
        (_, Some(addr)) => Some(addr),
        (Some((addr, _)), None) => Some(addr),
        (None, None) => None,
    }
}

/// 连接断开：只唤醒该设备自己的任务（不存在则忽略，随后 ensure_task 会补建）。
pub(crate) fn wake(inner: &Inner, device: &DeviceId) {
    if let Some(sig) = inner.redial.tasks.lock().unwrap().get(device) {
        sig.notify.notify_one();
    }
}

/// 发现事件（须在写入发现表之后调用）：唤醒该设备的任务；地址变了才清零退避。
pub(crate) fn kick_device(inner: &Inner, device: &DeviceId) {
    let sig = inner.redial.tasks.lock().unwrap().get(device).cloned();
    if let Some(sig) = sig {
        let addr = inner.discovered.addr_with_age(device).map(|(a, _)| a);
        sig.on_discovered(addr);
    }
}

/// 平台通知网络变化：2 秒去抖后清零退避立即重拨，并重新通告一次。
pub(crate) fn network_changed(inner: &Arc<Inner>) {
    if inner.redial.net_pending.swap(true, Ordering::SeqCst) {
        return; // 窗口内已有待触发的通知，合并
    }
    crate::net::path::invalidate_ifaces_cache();
    let inner = inner.clone();
    tokio::spawn(async move {
        tokio::time::sleep(NET_DEBOUNCE).await;
        inner.redial.net_pending.store(false, Ordering::SeqCst);
        tracing::info!("network changed, redialing trusted peers now");
        inner.redial.net.send_modify(|g| *g += 1);
        inner.announce_tx.send_modify(|_| {});
    });
}

/// 设备是否受信：未吊销，且属于自己或已接受联系人。
fn is_trusted(inner: &Inner, device: &DeviceId) -> bool {
    let Ok(Some(row)) = inner.store.get_device(device) else {
        return false;
    };
    if row.revoked {
        return false;
    }
    let Some(self_user) = inner.state.read().unwrap().self_user else {
        return false;
    };
    if row.user == self_user {
        return true;
    }
    inner
        .store
        .get_contact(&row.user)
        .ok()
        .flatten()
        .is_some_and(|c| c.state == ContactState::Accepted)
}

/// 最近已知地址（见 [`pick_addr`]）。
fn known_addr(inner: &Inner, device: &DeviceId) -> Option<SocketAddr> {
    let stored = inner
        .store
        .get_device(device)
        .ok()
        .flatten()
        .and_then(|r| r.last_addr)
        .and_then(|a| a.parse().ok());
    pick_addr(inner.discovered.addr_with_age(device), stored)
}

/// 为受信设备确保有一个重连任务（已有则什么都不做）。
pub(crate) fn ensure_task(inner: &Arc<Inner>, device: DeviceId) {
    if !is_trusted(inner, &device) {
        return;
    }
    let signals = {
        let mut tasks = inner.redial.tasks.lock().unwrap();
        if tasks.contains_key(&device) {
            return;
        }
        let n = Arc::new(TaskSignals::new());
        tasks.insert(device, n.clone());
        n
    };
    tokio::spawn(redial_task(inner.clone(), device, signals));
}

/// 启动时为库里所有已知地址的受信设备恢复重连任务。
pub(crate) fn resume_all(inner: &Arc<Inner>) {
    let Some(self_user) = inner.state.read().unwrap().self_user else {
        return;
    };
    let self_device = inner.identity.read().unwrap().device_id();
    let mut users = vec![self_user];
    if let Ok(contacts) = inner.store.list_contacts(Some(ContactState::Accepted)) {
        users.extend(contacts.into_iter().map(|c| c.user));
    }
    for user in users {
        let Ok(devices) = inner.store.devices_of_user(&user) else {
            continue;
        };
        for d in devices {
            if d.device != self_device && d.last_addr.is_some() {
                ensure_task(inner, d.device);
            }
        }
    }
}

/// 此刻是否允许对该设备外联：未初始化不拨；隐身时只拨自己的设备（与发现拨号一致）。
fn may_dial(inner: &Inner, device: &DeviceId) -> bool {
    let st = inner.state.read().unwrap();
    let Some(self_user) = st.self_user else {
        return false;
    };
    if !matches!(st.status, Status::Invisible) {
        return true;
    }
    inner
        .store
        .get_device(device)
        .ok()
        .flatten()
        .is_some_and(|r| r.user == self_user)
}

/// 目标地址是否与本机某网卡同网段（含回环）。只用来调节节奏：不同网段（可能已换网络）
/// 不主动高频拨号，等网络变化或该设备的发现事件再试一次；身份由 TLS 握手内的
/// 期望设备校验保证，不依赖网段。
fn on_local_subnet(addr: &SocketAddr) -> bool {
    #[cfg(feature = "test-hooks")]
    if FORCE_OFF_SUBNET.load(Ordering::SeqCst) {
        return false;
    }
    ip_on_local_subnet(addr.ip(), &local_ifaces())
}

/// 测试钩子：让所有地址都被视为不在本机网段。
#[cfg(feature = "test-hooks")]
static FORCE_OFF_SUBNET: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "test-hooks")]
pub(crate) fn force_off_subnet_for_tests(on: bool) {
    FORCE_OFF_SUBNET.store(on, Ordering::SeqCst);
}

/// 注意这里用 `is_connected`（连接表里有）而不是 `is_connected_alive`：静默失效但仍登记的连接
/// 交给既有机制处理（对端拨入时按空闲阈值替换、空闲超时后自清理发 Down），重连任务只在
/// 连接真正消失（Down）或从未连上时介入，避免与静态对端 / 发现拨号抢着替换同一条连接。
async fn redial_task(inner: Arc<Inner>, device: DeviceId, signals: Arc<TaskSignals>) {
    let mut net = inner.redial.net.subscribe();
    // 已连续失败（或抖动）的次数，决定下一次等待时长。任务常在连接刚断开时才创建，
    // 从 1 开始：首次拨号前先退避 0.5–1s，让正在进行的替换 / 对端拨入 / 静态对端重拨先落地。
    let mut attempt: u32 = 1;
    let mut alive_since: Option<Instant> = None;
    // 不在同网段的地址：先等到网络变化 / 发现事件（或最长退避）再拨一次。
    let mut off_subnet_released = false;
    while !inner.conns.is_shut_down() {
        inner.redial.loops.fetch_add(1, Ordering::SeqCst);
        if !is_trusted(&inner, &device) {
            break;
        }
        if inner.conns.is_connected(&device) {
            alive_since.get_or_insert_with(Instant::now);
            tokio::select! {
                _ = tokio::time::sleep(IDLE_POLL) => {}
                _ = signals.notify.notified() => {}
                _ = net.changed() => attempt = 0,
                _ = inner.conns.wait_shutdown() => break,
            }
            continue;
        }
        if let Some(t) = alive_since.take() {
            // 刚丢了连接：先退避一小段（0.5–1s）再拨，让正在进行的替换 / 对端拨入 / 静态
            // 对端重拨先落地，避免我们和它们同时拨号造成重复的 Down / Up。
            attempt = if t.elapsed() >= STABLE_AFTER {
                1
            } else {
                attempt.max(1)
            };
        }
        if attempt > 0 {
            let wait = backoff(attempt - 1, rand::random::<f64>());
            tracing::debug!(peer = %device, attempt, wait_ms = wait.as_millis() as u64, "redial backing off");
            let sleep = tokio::time::sleep(wait);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    // 断开事件的唤醒不算；该设备再次被发现才提前再拨一次（地址没变不改退避，变了才清零）。
                    _ = signals.notify.notified() => {
                        if signals.discovered.swap(false, Ordering::SeqCst) {
                            if signals.reset.swap(false, Ordering::SeqCst) {
                                attempt = 0;
                            }
                            break;
                        }
                    }
                    _ = net.changed() => {
                        attempt = 0;
                        break;
                    }
                    _ = inner.conns.wait_shutdown() => break,
                }
            }
            // 退避期间设备可能被吊销 / 删除、连接可能已恢复：拨号前重新判断。
            if inner.conns.is_shut_down() || !is_trusted(&inner, &device) {
                break;
            }
            if inner.conns.is_connected(&device) {
                continue;
            }
        }
        // 暂不允许外联（隐身 / 未初始化）：长间隔等待，状态变化时由网络 / 发现事件唤醒。
        if !may_dial(&inner, &device) {
            attempt = 0;
            tokio::select! {
                _ = tokio::time::sleep(IDLE_POLL) => {}
                _ = signals.notify.notified() => {}
                _ = net.changed() => {}
                _ = inner.conns.wait_shutdown() => break,
            }
            continue;
        }
        let Some(addr) = known_addr(&inner, &device) else {
            // 还没有任何地址：只等发现事件或网络变化，不轮询。
            attempt = 0;
            tokio::select! {
                _ = signals.notify.notified() => {}
                _ = net.changed() => {}
                _ = inner.conns.wait_shutdown() => break,
            }
            continue;
        };
        if !on_local_subnet(&addr) && !off_subnet_released {
            // 不在同网段：用最长退避间隔，网络变化或该设备的发现事件到来时立即试一次。
            let sleep = tokio::time::sleep(REDIAL_MAX);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    // 断开事件的唤醒不算：只有该设备再次被发现才提前试一次。
                    _ = signals.notify.notified() => {
                        if signals.discovered.swap(false, Ordering::SeqCst) {
                            break;
                        }
                    }
                    _ = net.changed() => break,
                    _ = inner.conns.wait_shutdown() => break,
                }
            }
            off_subnet_released = true;
            continue;
        }
        off_subnet_released = false;
        // 与发现触发的拨号共用在途登记：同一地址不并发两次 TLS。
        if !inner.dialing.lock().unwrap().insert(addr) {
            attempt = attempt.saturating_add(1);
            continue;
        }
        let _guard = DialingGuard {
            inner: inner.clone(),
            addr,
        };
        inner.redial.attempts.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            tracing::info!(peer = %device, %addr, "redialing trusted peer");
        } else {
            tracing::debug!(peer = %device, %addr, attempt, "redial attempt");
        }
        let result = tokio::select! {
            r = inner.conns.connect_expecting(addr, Some(device)) => r,
            _ = inner.conns.wait_shutdown() => break,
        };
        match result {
            Ok(_) => tracing::info!(peer = %device, %addr, "redial succeeded"),
            Err(e) => tracing::debug!(peer = %device, %addr, error = %e, "redial failed"),
        }
        attempt = attempt.saturating_add(1);
    }
    // 登记与退出判定：先移除，再确认一次是否仍受信（退出判定后、移除前若有人
    // 调用 ensure_task 会因“已存在”直接返回，这里补建，避免漏掉）。
    inner.redial.tasks.lock().unwrap().remove(&device);
    if !inner.conns.is_shut_down() {
        ensure_task(&inner, device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn backoff_doubles_from_one_second_and_caps_at_five_minutes() {
        let full: Vec<u64> = (0..11).map(|n| backoff(n, 1.0).as_secs()).collect();
        assert_eq!(full, vec![1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300]);
        assert_eq!(backoff(1000, 1.0), REDIAL_MAX);
    }

    #[test]
    fn backoff_jitter_scales_between_half_and_full() {
        assert_eq!(backoff(3, 0.0), Duration::from_secs(4));
        assert_eq!(backoff(3, 1.0), Duration::from_secs(8));
        // 越界的抖动值被夹紧
        assert_eq!(backoff(3, 5.0), Duration::from_secs(8));
    }

    #[test]
    fn unchanged_discovery_address_does_not_reset_backoff() {
        let sig = TaskSignals::new();
        let a = Some(addr("10.0.0.2:7000"));
        assert!(sig.on_discovered(a), "首次见到地址：清零");
        assert!(!sig.on_discovered(a), "地址不变的通告：只唤醒，不清零");
        assert!(
            sig.on_discovered(Some(addr("10.0.0.3:7000"))),
            "地址变了：清零"
        );
        assert!(!sig.on_discovered(None), "没有地址信息：不清零");
    }

    #[test]
    fn fresh_discovery_beats_stored_address() {
        let got = pick_addr(
            Some((addr("10.0.0.9:7000"), Duration::from_secs(5))),
            Some(addr("10.0.0.2:7000")),
        );
        assert_eq!(got, Some(addr("10.0.0.9:7000")));
    }

    #[test]
    fn stale_discovery_loses_to_stored_address() {
        let got = pick_addr(
            Some((addr("10.0.0.9:7000"), Duration::from_secs(600))),
            Some(addr("10.0.0.2:7000")),
        );
        assert_eq!(got, Some(addr("10.0.0.2:7000")));
    }

    #[test]
    fn stale_discovery_is_better_than_nothing() {
        let got = pick_addr(
            Some((addr("10.0.0.9:7000"), Duration::from_secs(600))),
            None,
        );
        assert_eq!(got, Some(addr("10.0.0.9:7000")));
        assert_eq!(pick_addr(None, None), None);
    }
}
