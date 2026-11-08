// 防火墙放行（P5 Task 18）：启动检查、`--add-firewall-rule` 命令行与 IPC 补发状态。
// 检测与 netsh 调用在 lanmsg-core 的 firewall 模块；这里只在 Windows 上编排。
// 注释中文，标识符英文。

use std::sync::atomic::{AtomicBool, Ordering};

/// 启动检查发现放行规则缺失（只有 Windows 会置位）；IPC 新连接据此补发 `FirewallMissing`。
static FIREWALL_MISSING: AtomicBool = AtomicBool::new(false);

/// 规则缺失标记。
pub(crate) fn firewall_missing() -> bool {
    FIREWALL_MISSING.load(Ordering::SeqCst)
}

/// 检查放行规则是否缺失并更新标记（阻塞，会运行 netsh；调用方放在后台线程）。
/// 非 Windows 恒为 false。
#[cfg(windows)]
pub(crate) fn check_missing() -> bool {
    let missing = match std::env::current_exe() {
        Ok(exe) => !lanmsg_core::firewall::firewall_rule_present(&exe),
        Err(e) => {
            tracing::warn!("cannot locate own executable for firewall check: {e}");
            false
        }
    };
    FIREWALL_MISSING.store(missing, Ordering::SeqCst);
    missing
}

#[cfg(not(windows))]
pub(crate) fn check_missing() -> bool {
    false
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
