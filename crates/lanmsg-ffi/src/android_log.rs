// 把 core 的 tracing 事件写入 Android logcat（tag 固定为 "lanmsg-core"）。
// 只依赖 liblog 的 __android_log_write，不引入额外 crate。
// 查看：adb logcat -s lanmsg-core
// 注释中文，标识符英文。

use std::ffi::{c_char, c_int, CString};
use std::fmt::Write as _;
use std::sync::Once;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use crate::log_text::to_c_text;

const TAG: &str = "lanmsg-core";

// android/log.h 的优先级常量
const ANDROID_LOG_DEBUG: c_int = 3;
const ANDROID_LOG_INFO: c_int = 4;
const ANDROID_LOG_WARN: c_int = 5;
const ANDROID_LOG_ERROR: c_int = 6;

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// 把事件的 message 与其余字段拼成一行文本。
struct LineVisitor(String);

impl Visit for LineVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let sep = if self.0.is_empty() { "" } else { " " };
        if field.name() == "message" {
            let _ = write!(self.0, "{sep}{value:?}");
        } else {
            let _ = write!(self.0, "{sep}{}={value:?}", field.name());
        }
    }
}

struct AndroidLayer;

impl<S: Subscriber> Layer<S> for AndroidLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = *event.metadata().level();
        let prio = match level {
            Level::ERROR => ANDROID_LOG_ERROR,
            Level::WARN => ANDROID_LOG_WARN,
            Level::INFO => ANDROID_LOG_INFO,
            _ => ANDROID_LOG_DEBUG,
        };
        let mut v = LineVisitor(String::new());
        event.record(&mut v);
        let line = format!("{}: {}", event.metadata().target(), v.0);
        if let (Ok(tag), Ok(text)) = (CString::new(TAG), to_c_text(&line)) {
            // SAFETY: 两个指针都指向以 NUL 结尾且在调用期间有效的 CString。
            unsafe {
                __android_log_write(prio, tag.as_ptr(), text.as_ptr());
            }
        }
    }
}

/// 安装全局 subscriber；重复调用或已有 subscriber 时静默跳过。
pub fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // 在 callsite 就过滤掉 debug / trace（过于嘈杂），只写 info 及以上
        let subscriber =
            tracing_subscriber::registry().with(AndroidLayer.with_filter(LevelFilter::INFO));
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}
