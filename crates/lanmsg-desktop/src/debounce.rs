// 输入防抖：文本停止变化满一段时间后才放行一次。纯逻辑，时间由调用方传入。
// 注释中文，标识符英文。

/// 防抖器。初始“已放行文本”为空串，故空输入不会触发。
pub struct Debouncer {
    delay_ms: u64,
    last_seen: String,
    changed_at: u64,
    emitted: String,
}

impl Debouncer {
    pub fn new(delay_ms: u64) -> Debouncer {
        Debouncer {
            delay_ms,
            last_seen: String::new(),
            changed_at: 0,
            emitted: String::new(),
        }
    }

    /// 喂入当前文本与时间；文本稳定满 `delay_ms` 且与上次放行不同时返回它。
    pub fn feed(&mut self, text: &str, now_ms: u64) -> Option<String> {
        if text != self.last_seen {
            self.last_seen = text.to_string();
            self.changed_at = now_ms;
        }
        if self.last_seen != self.emitted && now_ms.saturating_sub(self.changed_at) >= self.delay_ms
        {
            self.emitted = self.last_seen.clone();
            return Some(self.emitted.clone());
        }
        None
    }

    /// 文本为空且此前从未放行过非空内容：喂入不会产生任何结果，调用方可以直接跳过。
    pub fn is_idle(&self, text: &str) -> bool {
        text.is_empty() && self.last_seen.is_empty() && self.emitted.is_empty()
    }

    /// 外部直接设置了搜索状态（如点击结果后清空）：同步内部记录，避免重复触发。
    pub fn reset_to(&mut self, text: &str, now_ms: u64) {
        self.last_seen = text.to_string();
        self.emitted = text.to_string();
        self.changed_at = now_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_only_while_nothing_typed_or_pending() {
        let mut d = Debouncer::new(300);
        assert!(d.is_idle(""));
        assert!(!d.is_idle("a"));
        assert_eq!(d.feed("a", 0), None);
        assert_eq!(d.feed("a", 400), Some("a".into()));
        // 已放行过非空内容：框被清空后仍要再喂，让“空查询”放行
        assert_eq!(d.feed("", 500), None);
        assert!(!d.is_idle(""));
        assert_eq!(d.feed("", 900), Some(String::new()));
        assert!(d.is_idle(""));
    }

    #[test]
    fn emits_only_after_text_is_stable() {
        let mut d = Debouncer::new(300);
        assert_eq!(d.feed("a", 0), None);
        assert_eq!(d.feed("ab", 200), None, "文本又变了，重新计时");
        assert_eq!(d.feed("ab", 400), None);
        assert_eq!(d.feed("ab", 500), Some("ab".to_string()));
        assert_eq!(d.feed("ab", 900), None, "同一文本不重复放行");
    }

    #[test]
    fn clearing_text_is_emitted_so_search_resets() {
        let mut d = Debouncer::new(300);
        d.feed("x", 0);
        assert_eq!(d.feed("x", 300), Some("x".to_string()));
        assert_eq!(d.feed("", 400), None);
        assert_eq!(d.feed("", 700), Some(String::new()));
    }

    #[test]
    fn reset_to_suppresses_stale_emission() {
        let mut d = Debouncer::new(300);
        d.feed("x", 0);
        d.reset_to("", 100);
        assert_eq!(d.feed("", 1000), None);
    }
}
