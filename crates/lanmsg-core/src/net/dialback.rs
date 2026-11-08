// P5 NAT 下数据通道反向建立：等待者表。
// 请求方（直拨不通的一方）登记一次性令牌并经控制连接发 DataDialBack；
// 对方拨回后出示令牌，接受方据此把数据连接交给等待者。
// 令牌一次性、限时、绑定设备身份。注释中文，标识符英文。

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use lanmsg_proto::ids::DeviceId;
use rand::RngCore;
use tokio::sync::oneshot;

use crate::net::conn::DataStream;

/// 令牌有效期（与请求方等待时长一致）。
pub(crate) const DIAL_BACK_WAIT: Duration = Duration::from_secs(10);

/// 同时登记的等待者总量上限。
const MAX_PENDING: usize = 64;
/// 每个对端同时进行的拨回数上限。
const MAX_INFLIGHT_PER_DEVICE: usize = 4;
/// 全局同时进行的拨回数上限。
const MAX_INFLIGHT_GLOBAL: usize = 32;
/// 每个对端的拨回频率限制：窗口内最多次数。
const RATE_WINDOW: Duration = Duration::from_secs(10);
const RATE_MAX: usize = 8;
/// 同类日志的最小间隔（限速，避免刷屏）。
const LOG_INTERVAL: Duration = Duration::from_secs(10);
/// 记录“来源 IP → 上次日志时间”的条目上限。
const MAX_LOG_IPS: usize = 256;

/// 锁毒化时沿用内部数据继续工作（状态只是计数与表，不会处于不一致状态）。
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Waiter {
    device: DeviceId,
    expires: Instant,
    tx: oneshot::Sender<DataStream>,
}

/// 等待者表。std Mutex：锁内不 await。
#[derive(Default)]
pub(crate) struct DialBackTable {
    waiters: Mutex<HashMap<[u8; 16], Waiter>>,
    /// 非法令牌日志的来源 IP 限速表。
    invalid_log: Mutex<HashMap<IpAddr, Instant>>,
    /// 被请求方的拨回并发/频率限制。
    pub(crate) limiter: Arc<DialBackLimiter>,
}

#[derive(Default)]
struct DeviceState {
    inflight: usize,
    recent: VecDeque<Instant>,
}

#[derive(Default)]
struct LimiterState {
    per: HashMap<DeviceId, DeviceState>,
    global: usize,
    last_warn: Option<Instant>,
}

/// 被请求方的拨回限制：每设备并发 4、每设备 10 秒内 8 次、全局并发 32。
#[derive(Default)]
pub(crate) struct DialBackLimiter {
    state: Mutex<LimiterState>,
}

/// 拨回许可：持有期间占用并发名额，丢弃时释放。
pub(crate) struct DialBackPermit {
    limiter: Arc<DialBackLimiter>,
    device: DeviceId,
}

impl Drop for DialBackPermit {
    fn drop(&mut self) {
        let mut st = lock(&self.limiter.state);
        st.global = st.global.saturating_sub(1);
        if let Some(d) = st.per.get_mut(&self.device) {
            d.inflight = d.inflight.saturating_sub(1);
        }
    }
}

impl DialBackLimiter {
    /// 申请一次拨回许可；超出任一上限返回 None（调用方丢弃请求，不拨号）。
    pub(crate) fn try_acquire(
        self: &Arc<Self>,
        device: DeviceId,
        now: Instant,
    ) -> Option<DialBackPermit> {
        let mut st = lock(&self.state);
        if st.global >= MAX_INFLIGHT_GLOBAL {
            return None;
        }
        let d = st.per.entry(device).or_default();
        while d
            .recent
            .front()
            .is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW)
        {
            d.recent.pop_front();
        }
        if d.inflight >= MAX_INFLIGHT_PER_DEVICE || d.recent.len() >= RATE_MAX {
            return None;
        }
        d.inflight += 1;
        d.recent.push_back(now);
        st.global += 1;
        Some(DialBackPermit {
            limiter: self.clone(),
            device,
        })
    }

    /// 是否应该记一条“被限制”的告警（全局限速，避免刷屏）。
    pub(crate) fn should_warn(&self, now: Instant) -> bool {
        let mut st = lock(&self.state);
        if st
            .last_warn
            .is_some_and(|t| now.duration_since(t) < LOG_INTERVAL)
        {
            return false;
        }
        st.last_warn = Some(now);
        true
    }

    /// 对端断开：清掉它的频率记录（仍在进行的拨回照常释放名额）。
    pub(crate) fn forget(&self, device: &DeviceId) {
        let mut st = lock(&self.state);
        if let Some(d) = st.per.get_mut(device) {
            d.recent.clear();
            if d.inflight == 0 {
                st.per.remove(device);
            }
        }
    }
}

impl DialBackTable {
    /// 登记一个等待者：返回新令牌（操作系统随机源）与接收端。
    /// 登记总量达到上限时返回 `None`。
    pub(crate) fn register(
        &self,
        device: DeviceId,
        ttl: Duration,
    ) -> Option<([u8; 16], oneshot::Receiver<DataStream>)> {
        let mut token = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut token);
        let (tx, rx) = oneshot::channel();
        let now = Instant::now();
        let mut map = lock(&self.waiters);
        // 顺手清理已过期的条目。
        map.retain(|_, w| w.expires > now);
        if map.len() >= MAX_PENDING {
            return None;
        }
        map.insert(
            token,
            Waiter {
                device,
                expires: now + ttl,
                tx,
            },
        );
        Some((token, rx))
    }

    /// 取消登记（请求方超时或放弃时调用）。
    pub(crate) fn cancel(&self, token: &[u8; 16]) {
        lock(&self.waiters).remove(token);
    }

    /// 认领令牌：令牌存在、未过期且设备一致时取走等待者（一次性）。
    /// 过期的令牌立即作废；设备不一致不消耗令牌（防止第三方烧掉合法令牌）。
    pub(crate) fn claim(
        &self,
        token: &[u8; 16],
        device: &DeviceId,
    ) -> Option<oneshot::Sender<DataStream>> {
        let mut map = lock(&self.waiters);
        let w = map.get(token)?;
        if w.expires <= Instant::now() {
            map.remove(token);
            return None;
        }
        if w.device != *device {
            return None;
        }
        map.remove(token).map(|w| w.tx)
    }
}

impl DialBackTable {
    /// 非法令牌日志的来源 IP 限速：同一 IP 每 10 秒最多记一条。
    pub(crate) fn should_log_invalid(&self, ip: IpAddr, now: Instant) -> bool {
        let mut map = lock(&self.invalid_log);
        if map.len() >= MAX_LOG_IPS {
            map.retain(|_, t| now.duration_since(*t) < LOG_INTERVAL);
            if map.len() >= MAX_LOG_IPS {
                return false;
            }
        }
        match map.get(&ip) {
            Some(t) if now.duration_since(*t) < LOG_INTERVAL => false,
            _ => {
                map.insert(ip, now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_single_use() {
        let t = DialBackTable::default();
        let d = DeviceId([1; 32]);
        let (token, _rx) = t.register(d, Duration::from_secs(5)).unwrap();
        assert!(t.claim(&token, &d).is_some());
        assert!(t.claim(&token, &d).is_none());
    }

    #[test]
    fn wrong_device_neither_claims_nor_burns_token() {
        let t = DialBackTable::default();
        let d = DeviceId([1; 32]);
        let (token, _rx) = t.register(d, Duration::from_secs(5)).unwrap();
        assert!(t.claim(&token, &DeviceId([2; 32])).is_none());
        assert!(t.claim(&token, &d).is_some());
    }

    #[test]
    fn unknown_and_expired_tokens_are_rejected() {
        let t = DialBackTable::default();
        let d = DeviceId([1; 32]);
        assert!(t.claim(&[0; 16], &d).is_none());
        let (token, _rx) = t.register(d, Duration::ZERO).unwrap();
        assert!(t.claim(&token, &d).is_none());
    }

    #[test]
    fn tokens_are_unique() {
        let t = DialBackTable::default();
        let d = DeviceId([1; 32]);
        let (a, _ra) = t.register(d, Duration::from_secs(5)).unwrap();
        let (b, _rb) = t.register(d, Duration::from_secs(5)).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn pending_registrations_are_capped() {
        let t = DialBackTable::default();
        let d = DeviceId([1; 32]);
        let mut keep = Vec::new();
        for _ in 0..MAX_PENDING {
            keep.push(t.register(d, Duration::from_secs(5)).unwrap());
        }
        assert!(t.register(d, Duration::from_secs(5)).is_none());
        // 取消一个后又能登记。
        t.cancel(&keep[0].0);
        assert!(t.register(d, Duration::from_secs(5)).is_some());
    }

    #[test]
    fn limiter_caps_inflight_per_device_and_globally() {
        let l = Arc::new(DialBackLimiter::default());
        let now = Instant::now();
        let d = DeviceId([1; 32]);
        let permits: Vec<_> = (0..MAX_INFLIGHT_PER_DEVICE)
            .map(|_| l.try_acquire(d, now).unwrap())
            .collect();
        assert!(l.try_acquire(d, now).is_none(), "per-device inflight cap");
        drop(permits);
        // 释放后并发名额回来，但频率窗口里已有 4 次。
        assert!(l.try_acquire(d, now).is_some());

        // 全局上限：每个设备各占 1 个并发，直到 32。
        let l = Arc::new(DialBackLimiter::default());
        let held: Vec<_> = (0..MAX_INFLIGHT_GLOBAL as u8)
            .map(|i| l.try_acquire(DeviceId([i; 32]), now).unwrap())
            .collect();
        assert!(l.try_acquire(DeviceId([200; 32]), now).is_none());
        drop(held);
        assert!(l.try_acquire(DeviceId([200; 32]), now).is_some());
    }

    #[test]
    fn limiter_rate_window_and_forget() {
        let l = Arc::new(DialBackLimiter::default());
        let t0 = Instant::now();
        let d = DeviceId([1; 32]);
        for _ in 0..RATE_MAX {
            drop(l.try_acquire(d, t0).unwrap());
        }
        assert!(l.try_acquire(d, t0).is_none(), "rate cap");
        assert!(
            l.try_acquire(d, t0 + RATE_WINDOW).is_some(),
            "window slides"
        );
        // 断开后清理频率记录。
        let l2 = Arc::new(DialBackLimiter::default());
        for _ in 0..RATE_MAX {
            drop(l2.try_acquire(d, t0).unwrap());
        }
        l2.forget(&d);
        assert!(l2.try_acquire(d, t0).is_some());
    }

    #[test]
    fn warn_and_invalid_logs_are_rate_limited() {
        let l = DialBackLimiter::default();
        let t0 = Instant::now();
        assert!(l.should_warn(t0));
        assert!(!l.should_warn(t0));
        assert!(l.should_warn(t0 + LOG_INTERVAL));
        let t = DialBackTable::default();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(t.should_log_invalid(ip, t0));
        assert!(!t.should_log_invalid(ip, t0));
        assert!(t.should_log_invalid("10.0.0.2".parse().unwrap(), t0));
    }
}
