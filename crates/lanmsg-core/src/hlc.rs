// P1 核心骨架：混合逻辑时钟（HLC）。
// HlcClock 实现标准 HLC 算法，保证逻辑时钟单调递增。
// 注释中文，标识符英文。

use lanmsg_proto::types::Hlc;

/// 混合逻辑时钟，记录最后观察到的 HLC 状态。
pub struct HlcClock {
    last: Hlc,
}

impl Default for HlcClock {
    fn default() -> Self {
        Self::new()
    }
}

impl HlcClock {
    /// 创建一个以零时间戳启动的 HLC 时钟。
    pub fn new() -> Self {
        Self {
            last: Hlc {
                millis: 0,
                counter: 0,
            },
        }
    }

    /// 生成当前时刻的 HLC 值。
    ///
    /// 算法：`millis = max(wall, last.millis)`；若等于 `last.millis` 则 `counter = last.counter + 1`，否则 `counter = 0`。
    pub fn now(&mut self, wall_millis: i64) -> Hlc {
        let millis = wall_millis.max(self.last.millis);
        if millis == self.last.millis {
            self.last.counter += 1;
        } else {
            self.last.millis = millis;
            self.last.counter = 0;
        }
        self.last.clone()
    }

    /// 观察远端 HLC，合并到本地状态。
    ///
    /// 算法：`millis = max(wall, last.millis, remote.millis)`，counter 按三者中取得最大值的来源 +1（相同则取较大 counter +1）。
    pub fn observe(&mut self, remote: Hlc, wall_millis: i64) {
        let millis = wall_millis.max(self.last.millis).max(remote.millis);

        let mut counter = 0u32;
        if self.last.millis == millis {
            counter = counter.max(self.last.counter);
        }
        if remote.millis == millis {
            counter = counter.max(remote.counter);
        }

        // wall 独占最大时（壁钟时间独占新的物理时刻），与 now() 保持一致：counter 从 0 开始。
        let wall_exclusive =
            wall_millis == millis && wall_millis > self.last.millis && wall_millis > remote.millis;
        let counter = if wall_exclusive { 0 } else { counter + 1 };

        self.last = Hlc { millis, counter };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hlc_monotonic_when_wall_clock_goes_back() {
        let mut c = HlcClock::new();
        let a = c.now(1000);
        let b = c.now(900);
        assert!(b > a);
    }

    #[test]
    fn hlc_observe_remote_ahead() {
        let mut c = HlcClock::new();
        c.observe(
            Hlc {
                millis: 5000,
                counter: 3,
            },
            1000,
        );
        let n = c.now(1000);
        assert!(
            n > Hlc {
                millis: 5000,
                counter: 3
            }
        );
    }
}
