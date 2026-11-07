// 受信对端的主动重连：已配对设备（自己的设备与已接受联系人）断开后，
// 不再只等组播通告，而是按最近已知地址拨号，失败按 1s..60s 指数退避（带抖动）。
// 双方同时拨号由 ConnManager 的重复连接裁决收敛（保留拨号方 DeviceId 较小的一条）。
// 注释中文，标识符英文。

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lanmsg_proto::consts::RETRY_MIN;
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::types::Status;
use tokio::sync::watch;

use super::Inner;
use crate::store::ContactState;

/// 受信重连退避上限。
const REDIAL_MAX: Duration = Duration::from_secs(60);

/// 连接稳定存活超过此时长才清零退避；更短视为抖动，继续沿用退避。
const STABLE_AFTER: Duration = Duration::from_secs(30);

/// 已连接时的兜底轮询间隔（Down 事件正常会直接唤醒）。
const CONNECTED_POLL: Duration = Duration::from_secs(10);

/// 重连状态：已有任务的设备、唤醒信号与尝试计数。
pub(crate) struct RedialState {
    tasks: Mutex<HashSet<DeviceId>>,
    /// 有连接断开：唤醒处于“已连接”等待中的任务。
    down: watch::Sender<u64>,
    /// 网络变化：清零所有退避，立即重拨。
    net: watch::Sender<u64>,
    attempts: AtomicU64,
}

impl RedialState {
    pub(crate) fn new() -> Self {
        Self {
            tasks: Mutex::new(HashSet::new()),
            down: watch::channel(0).0,
            net: watch::channel(0).0,
            attempts: AtomicU64::new(0),
        }
    }

    pub(crate) fn attempts(&self) -> u64 {
        self.attempts.load(Ordering::SeqCst)
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

/// 连接断开时唤醒各任务。
pub(crate) fn wake(inner: &Inner) {
    inner.redial.down.send_modify(|g| *g += 1);
}

/// 平台通知网络变化：清零退避立即重拨，并立刻重新通告一次。
pub(crate) fn network_changed(inner: &Inner) {
    tracing::info!("network changed, redialing trusted peers now");
    inner.redial.net.send_modify(|g| *g += 1);
    inner.announce_tx.send_modify(|_| {});
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

/// 最近已知地址：优先发现表（组播刚刷新），其次上次连接时持久化的地址。
fn known_addr(inner: &Inner, device: &DeviceId) -> Option<SocketAddr> {
    if let Some((_, addr)) = inner.discovered.get(device) {
        return Some(addr);
    }
    let row = inner.store.get_device(device).ok().flatten()?;
    row.last_addr?.parse().ok()
}

/// 为受信设备确保有一个重连任务（已有则什么都不做）。
pub(crate) fn ensure_task(inner: &Arc<Inner>, device: DeviceId) {
    if !is_trusted(inner, &device) {
        return;
    }
    if inner.redial.tasks.lock().unwrap().insert(device) {
        tokio::spawn(redial_task(inner.clone(), device));
    }
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

async fn redial_task(inner: Arc<Inner>, device: DeviceId) {
    let mut down = inner.redial.down.subscribe();
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
                _ = tokio::time::sleep(CONNECTED_POLL) => {}
                _ = down.changed() => {}
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
                _ = net.changed() => attempt = 0,
                _ = inner.conns.wait_shutdown() => break,
            }
            // 等待期间连接可能已恢复（对端拨入）或身份变化，回到循环顶部重新判断。
            if attempt > 0 && inner.conns.is_connected_alive(&device) {
                continue;
            }
        }
        if !may_dial(&inner, &device) {
            attempt = attempt.max(1);
            continue;
        }
        let Some(addr) = known_addr(&inner, &device) else {
            attempt = attempt.saturating_add(1);
            continue;
        };
        inner.redial.attempts.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            tracing::info!(peer = %device, %addr, "redialing trusted peer");
        } else {
            tracing::debug!(peer = %device, %addr, attempt, "redial attempt");
        }
        let result = tokio::select! {
            r = inner.conns.connect(addr, false) => r,
            _ = inner.conns.wait_shutdown() => break,
        };
        match result {
            Ok(_) => tracing::info!(peer = %device, %addr, "redial succeeded"),
            Err(e) => tracing::debug!(peer = %device, %addr, error = %e, "redial failed"),
        }
        attempt = attempt.saturating_add(1);
    }
    inner.redial.tasks.lock().unwrap().remove(&device);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_one_second_and_caps_at_sixty() {
        let full: Vec<u64> = (0..9).map(|n| backoff(n, 1.0).as_secs()).collect();
        assert_eq!(full, vec![1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(backoff(1000, 1.0), REDIAL_MAX);
    }

    #[test]
    fn backoff_jitter_scales_between_half_and_full() {
        assert_eq!(backoff(3, 0.0), Duration::from_secs(4));
        assert_eq!(backoff(3, 1.0), Duration::from_secs(8));
        // 越界的抖动值被夹紧
        assert_eq!(backoff(3, 5.0), Duration::from_secs(8));
    }
}
