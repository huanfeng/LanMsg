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
use lanmsg_proto::types::{PathClass, Status};
use tokio::sync::{watch, Notify};

use super::{DialingGuard, Inner};
use crate::net::path::{classify, local_ifaces};
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

/// 重连状态：每个受信设备一个任务（带专属唤醒信号），以及网络变化信号与尝试计数。
pub(crate) struct RedialState {
    tasks: Mutex<HashMap<DeviceId, Arc<Notify>>>,
    /// 网络变化（已去抖）：清零所有退避，立即重拨。
    net: watch::Sender<u64>,
    net_pending: AtomicBool,
    attempts: AtomicU64,
}

impl RedialState {
    pub(crate) fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            net: watch::channel(0).0,
            net_pending: AtomicBool::new(false),
            attempts: AtomicU64::new(0),
        }
    }

    pub(crate) fn attempts(&self) -> u64 {
        self.attempts.load(Ordering::SeqCst)
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
    if let Some(n) = inner.redial.tasks.lock().unwrap().get(device) {
        n.notify_one();
    }
}

/// 发现事件：该设备的任务若在退避，立即重试。
pub(crate) fn kick_device(inner: &Inner, device: &DeviceId) {
    wake(inner, device);
}

/// 平台通知网络变化：2 秒去抖后清零退避立即重拨，并重新通告一次。
pub(crate) fn network_changed(inner: &Arc<Inner>) {
    if inner.redial.net_pending.swap(true, Ordering::SeqCst) {
        return; // 窗口内已有待触发的通知，合并
    }
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
    let notify = {
        let mut tasks = inner.redial.tasks.lock().unwrap();
        if tasks.contains_key(&device) {
            return;
        }
        let n = Arc::new(Notify::new());
        tasks.insert(device, n.clone());
        n
    };
    tokio::spawn(redial_task(inner.clone(), device, notify));
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

/// 目标地址是否与本机某网卡同网段（含回环）。不同网段（换了 Wi-Fi）不拨，
/// 以免向旧地址上碰巧在监听的陌生主机发起 TLS。
fn on_local_subnet(addr: &SocketAddr) -> bool {
    classify(addr.ip(), &local_ifaces(), false) == PathClass::Lan
}

async fn redial_task(inner: Arc<Inner>, device: DeviceId, wake_me: Arc<Notify>) {
    let mut net = inner.redial.net.subscribe();
    // 已连续失败（或抖动）的次数，决定下一次等待时长。
    let mut attempt: u32 = 0;
    let mut alive_since: Option<Instant> = None;
    while !inner.conns.is_shut_down() {
        if !is_trusted(&inner, &device) {
            break;
        }
        if inner.conns.is_connected_alive(&device) {
            alive_since.get_or_insert_with(Instant::now);
            tokio::select! {
                _ = tokio::time::sleep(IDLE_POLL) => {}
                _ = wake_me.notified() => {}
                _ = net.changed() => attempt = 0,
                _ = inner.conns.wait_shutdown() => break,
            }
            continue;
        }
        if alive_since
            .take()
            .is_some_and(|t| t.elapsed() >= STABLE_AFTER)
        {
            attempt = 0;
        }
        if attempt > 0 {
            let wait = backoff(attempt - 1, rand::random::<f64>());
            tracing::debug!(peer = %device, attempt, wait_ms = wait.as_millis() as u64, "redial backing off");
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = wake_me.notified() => attempt = 0,
                _ = net.changed() => attempt = 0,
                _ = inner.conns.wait_shutdown() => break,
            }
            // 退避期间设备可能被吊销 / 删除、连接可能已恢复：拨号前重新判断。
            if inner.conns.is_shut_down() || !is_trusted(&inner, &device) {
                break;
            }
            if inner.conns.is_connected_alive(&device) {
                continue;
            }
        }
        let addr = known_addr(&inner, &device);
        let dialable = addr.is_some_and(|a| on_local_subnet(&a)) && may_dial(&inner, &device);
        let Some(addr) = addr.filter(|_| dialable) else {
            // 没有地址、不在同一网段或暂不允许外联：不算失败，长间隔等待状态变化。
            attempt = 0;
            tokio::select! {
                _ = tokio::time::sleep(IDLE_POLL) => {}
                _ = wake_me.notified() => {}
                _ = net.changed() => {}
                _ = inner.conns.wait_shutdown() => break,
            }
            continue;
        };
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
