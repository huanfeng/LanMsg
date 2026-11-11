// 应用内更新的桌面端安装动作（P5 T23a）。
// core（lanmsgd）负责下载与校验；安装动作由界面进程执行：
// - Windows 安装器版：运行下载好的安装器 --silent（未提权时自弹 UAC），随后退出界面与 lanmsgd；
// - Windows scoop 版：弹一个新的命令行窗口执行 `scoop update lanmsg`（先等 2 秒让进程退出），
//   随后退出界面与 lanmsgd；
// - Linux 桌面渠道：后台执行 Release 中的 install.sh --version vX.Y.Z，完成后提示重启；
// - macOS：打开 dmg 提示手动拖装。
// 失败只记英文日志，不打断退出流程。注释中文，标识符英文。

use lanmsg_api::types::InstallChannel;

/// 安装动作的结果：启动更新后界面进程是否退出。
pub enum InstallOutcome {
    /// 拉起安装动作后退出自身与后台服务（安装器版与 scoop 版）。
    Exit,
    /// 留在当前进程（Linux/macOS：完成后提示重启或手动拖装）。
    Stay,
}

/// 执行安装动作。`path` 为下载完成的文件路径；scoop 渠道不需要文件（None）。
pub fn run_install(path: Option<&str>, channel: InstallChannel, version: &str) -> InstallOutcome {
    match channel {
        InstallChannel::WindowsScoop => {
            pop_scoop_console();
            InstallOutcome::Exit
        }
        InstallChannel::WindowsInstaller => match path {
            Some(p) => {
                spawn_silent_installer(p);
                InstallOutcome::Exit
            }
            None => {
                eprintln!("installer channel requires a downloaded installer path");
                InstallOutcome::Stay
            }
        },
        InstallChannel::LinuxSh => {
            if let Some(p) = path {
                run_install_sh(p, version);
            }
            InstallOutcome::Stay
        }
        InstallChannel::MacDmg => {
            if let Some(p) = path {
                open_dmg(p);
            }
            InstallOutcome::Stay
        }
        InstallChannel::Manual => InstallOutcome::Stay,
    }
}

/// Windows：运行下载好的安装器（静默安装；未提权时安装器自弹 UAC）。
#[cfg(windows)]
fn spawn_silent_installer(path: &str) {
    match std::process::Command::new(path).arg("--silent").spawn() {
        Ok(_) => {}
        Err(e) => tracing::error!("failed to launch installer: {e}"),
    }
}

/// 非 Windows 编译占位：安装器渠道只出现在 Windows（防御性日志）。
#[cfg(not(windows))]
fn spawn_silent_installer(path: &str) {
    eprintln!("installer channel is windows-only, cannot run: {path}");
}

/// Windows：弹新的命令行窗口执行 scoop 更新（等 2 秒让界面与 lanmsgd 先退出）。
#[cfg(windows)]
fn pop_scoop_console() {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let r = std::process::Command::new("cmd")
        .args(["/k", "timeout /t 2 /nobreak && scoop update lanmsg"])
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn();
    match r {
        Ok(_) => {}
        Err(e) => tracing::error!("failed to open scoop update console: {e}"),
    }
}

/// 非 Windows 编译占位。
#[cfg(not(windows))]
fn pop_scoop_console() {
    eprintln!("scoop channel is windows-only");
}

/// Linux：后台执行 install.sh（带 --version），完成后由用户重启。
#[cfg(target_os = "linux")]
fn run_install_sh(path: &str, version: &str) {
    match std::process::Command::new("sh")
        .arg(path)
        .arg("--version")
        .arg(format!("v{version}"))
        .spawn()
    {
        Ok(_) => {}
        Err(e) => tracing::error!("failed to run install.sh: {e}"),
    }
}

/// 非 Linux 编译占位。
#[cfg(not(target_os = "linux"))]
fn run_install_sh(path: &str, version: &str) {
    eprintln!("install.sh channel is linux-only: {path} v{version}");
}

/// macOS：打开 dmg 提示拖装。
#[cfg(target_os = "macos")]
fn open_dmg(path: &str) {
    match std::process::Command::new("open").arg(path).spawn() {
        Ok(_) => {}
        Err(e) => tracing::error!("failed to open dmg: {e}"),
    }
}

/// 非 macOS 编译占位。
#[cfg(not(target_os = "macos"))]
fn open_dmg(path: &str) {
    eprintln!("dmg channel is macos-only: {path}");
}
