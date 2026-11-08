// 防火墙放行（P5 Task 18）：后台检查与 `--add-firewall-rule` 命令行。
// 检测与 netsh 调用在 lanmsg-core 的 firewall 模块；这里只在 Windows 上编排。
// 缺失状态写入运行时共享的 watch（见 `runtime.rs`），IPC 连接据此补发与推送 `FirewallMissing`。
// 注释中文，标识符英文。

/// 托盘是否要弹“防火墙可能阻止发现”通知：只在由“已放行”变为“缺失”时提示一次。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn should_notify(was_missing: bool, missing: bool) -> bool {
    missing && !was_missing
}

/// 检查放行规则是否缺失（阻塞，会运行 netsh；调用方放在后台线程）。
#[cfg(windows)]
pub(crate) fn check_missing() -> bool {
    match std::env::current_exe() {
        Ok(exe) => !lanmsg_core::firewall::firewall_rule_present(&exe),
        Err(e) => {
            tracing::warn!("cannot locate own executable for firewall check: {e}");
            false
        }
    }
}

/// 复查间隔：规则缺失期间定期重查，用户在别处（界面提权、手工）放行后状态能自行恢复。
#[cfg(windows)]
const RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// 后台线程：检查规则并把结果写入 `tx`；缺失时每 30 秒复查，直到放行后退出。
/// 状态变化（缺失 ↔ 放行）时调用 `on_change(missing)`；启动时已放行则不调用。
#[cfg(windows)]
pub(crate) fn spawn_monitor(
    tx: std::sync::Arc<tokio::sync::watch::Sender<bool>>,
    on_change: impl Fn(bool) + Send + 'static,
) {
    std::thread::spawn(move || loop {
        let missing = check_missing();
        if tx.send_replace(missing) != missing {
            on_change(missing);
        }
        if !missing {
            break;
        }
        std::thread::sleep(RECHECK_INTERVAL);
    });
}

/// `lanmsgd --add-firewall-rule`：不启动服务，为本程序添加入站放行规则（需要管理员权限）。
/// 返回进程退出码：成功 0，失败 1；非 Windows 打印说明并返回 0。
#[cfg(windows)]
pub(crate) fn run_add_rule() -> i32 {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cannot locate own executable: {e}");
            return 1;
        }
    };
    match lanmsg_core::firewall::add_rule(&exe) {
        Ok(()) => {
            println!("Firewall rule added");
            0
        }
        Err(e) => {
            eprintln!("failed to add firewall rule: {e}");
            1
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn run_add_rule() -> i32 {
    println!("The firewall rule is only needed on Windows");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_only_on_transition_to_missing() {
        assert!(should_notify(false, true));
        // 重复上报缺失、恢复放行、一直放行都不提示
        assert!(!should_notify(true, true));
        assert!(!should_notify(true, false));
        assert!(!should_notify(false, false));
    }
}
