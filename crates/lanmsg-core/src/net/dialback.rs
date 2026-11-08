// P5 NAT 下数据通道反向建立：等待者表。
// 请求方（直拨不通的一方）登记一次性令牌并经控制连接发 DataDialBack；
// 对方拨回后出示令牌，接受方据此把数据连接交给等待者。
// 令牌一次性、限时、绑定设备身份。注释中文，标识符英文。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lanmsg_proto::ids::DeviceId;
use rand::RngCore;
use tokio::sync::oneshot;

use crate::net::conn::DataStream;

/// 令牌有效期（与请求方等待时长一致）。
pub(crate) const DIAL_BACK_WAIT: Duration = Duration::from_secs(10);

struct Waiter {
    device: DeviceId,
    expires: Instant,
    tx: oneshot::Sender<DataStream>,
}

/// 等待者表。std Mutex：锁内不 await。
#[derive(Default)]
pub(crate) struct DialBackTable {
    waiters: Mutex<HashMap<[u8; 16], Waiter>>,
}

impl DialBackTable {
    /// 登记一个等待者：返回新令牌（操作系统随机源）与接收端。
    pub(crate) fn register(
        &self,
        device: DeviceId,
        ttl: Duration,
    ) -> ([u8; 16], oneshot::Receiver<DataStream>) {
        let mut token = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut token);
        let (tx, rx) = oneshot::channel();
        let now = Instant::now();
        let mut map = self.waiters.lock().unwrap();
        // 顺手清理已过期的条目。
        map.retain(|_, w| w.expires > now);
        map.insert(
            token,
            Waiter {
                device,
                expires: now + ttl,
                tx,
            },
        );
        (token, rx)
    }

    /// 取消登记（请求方超时或放弃时调用）。
    pub(crate) fn cancel(&self, token: &[u8; 16]) {
        self.waiters.lock().unwrap().remove(token);
    }

    /// 认领令牌：令牌存在、未过期且设备一致时取走等待者（一次性）。
    /// 过期的令牌立即作废；设备不一致不消耗令牌（防止第三方烧掉合法令牌）。
    pub(crate) fn claim(
        &self,
        token: &[u8; 16],
        device: &DeviceId,
    ) -> Option<oneshot::Sender<DataStream>> {
        let mut map = self.waiters.lock().unwrap();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_single_use() {
        let t = DialBackTable::default();
        let d = DeviceId([1; 32]);
        let (token, _rx) = t.register(d, Duration::from_secs(5));
        assert!(t.claim(&token, &d).is_some());
        assert!(t.claim(&token, &d).is_none());
    }

    #[test]
    fn wrong_device_neither_claims_nor_burns_token() {
        let t = DialBackTable::default();
        let d = DeviceId([1; 32]);
        let (token, _rx) = t.register(d, Duration::from_secs(5));
        assert!(t.claim(&token, &DeviceId([2; 32])).is_none());
        assert!(t.claim(&token, &d).is_some());
    }

    #[test]
    fn unknown_and_expired_tokens_are_rejected() {
        let t = DialBackTable::default();
        let d = DeviceId([1; 32]);
        assert!(t.claim(&[0; 16], &d).is_none());
        let (token, _rx) = t.register(d, Duration::ZERO);
        assert!(t.claim(&token, &d).is_none());
    }

    #[test]
    fn tokens_are_unique() {
        let t = DialBackTable::default();
        let d = DeviceId([1; 32]);
        let (a, _ra) = t.register(d, Duration::from_secs(5));
        let (b, _rb) = t.register(d, Duration::from_secs(5));
        assert_ne!(a, b);
    }
}
