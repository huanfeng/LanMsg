// 连接保活时序：心跳间隔、空闲超时、重连替换阈值。
// 生产值取自 lanmsg-proto/consts.rs；仅 `test-hooks` 特性（集成测试）可整体缩短。
// 注释中文，标识符英文。

use std::time::Duration;

use lanmsg_proto::consts::{
    ACCEPT_HANDSHAKE_TIMEOUT, DIAL_TIMEOUT, PING_INTERVAL, PING_TIMEOUT, RECONNECT_REPLACE_IDLE,
};

/// 一组保活时序（连接建立时取快照）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct NetTiming {
    /// 无其他出帧时发送 Ping 的间隔。
    pub heartbeat: Duration,
    /// 读侧超过该时长没收到任何帧即判对端消失。
    pub idle_timeout: Duration,
    /// 新连接到达时，旧连接空闲超过该时长即被替换。
    pub replace_idle: Duration,
    /// 出站拨号总时限。
    pub dial: Duration,
    /// 入站握手总时限。
    pub accept_handshake: Duration,
}

const DEFAULT: NetTiming = NetTiming {
    heartbeat: PING_INTERVAL,
    idle_timeout: PING_TIMEOUT,
    replace_idle: RECONNECT_REPLACE_IDLE,
    dial: DIAL_TIMEOUT,
    accept_handshake: ACCEPT_HANDSHAKE_TIMEOUT,
};

#[cfg(feature = "test-hooks")]
static OVERRIDE: std::sync::Mutex<Option<NetTiming>> = std::sync::Mutex::new(None);

/// 当前生效的时序。
pub(crate) fn current() -> NetTiming {
    #[cfg(feature = "test-hooks")]
    if let Some(t) = *OVERRIDE.lock().unwrap() {
        return t;
    }
    DEFAULT
}

/// 测试专用：整进程缩短保活时序（每个集成测试文件是独立进程，互不影响）。
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn set_timing_for_tests(
    heartbeat: Duration,
    idle_timeout: Duration,
    replace_idle: Duration,
    dial: Duration,
    accept_handshake: Duration,
) {
    *OVERRIDE.lock().unwrap() = Some(NetTiming {
        heartbeat,
        idle_timeout,
        replace_idle,
        dial,
        accept_handshake,
    });
}
