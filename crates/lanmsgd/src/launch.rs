// 拉起界面进程（P3 Task 9）：`lanmsg --data-dir <dir> [--open-conv <conv>]`。
// 已在运行的界面由界面自身的单实例机制激活（Task 10）。
// 注释中文，标识符英文。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 界面可执行文件：与当前可执行文件同目录的 `lanmsg(.exe)`。
pub fn gui_exe() -> PathBuf {
    let name = if cfg!(windows) {
        "lanmsg.exe"
    } else {
        "lanmsg"
    };
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// 通知点击带回的 tag → 要定位的会话。
/// None 或空串（Windows 上气泡超时 / 被顶掉后的点击）都视为“只激活界面”。
pub fn conv_from_tag(tag: Option<String>) -> Option<String> {
    tag.filter(|t| !t.is_empty())
}

fn gui_command(exe: &Path, data_dir: &Path, open_conv: Option<&str>) -> Command {
    let mut cmd = Command::new(exe);
    cmd.arg("--data-dir").arg(data_dir);
    if let Some(conv) = open_conv {
        cmd.arg("--open-conv").arg(conv);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组，不随守护进程的终端信号退出。
        cmd.process_group(0);
    }
    cmd
}

/// 分离启动界面进程；不等待其退出（子进程由系统回收，退出后无需 wait）。
pub fn open_gui(data_dir: &Path, open_conv: Option<&str>) -> std::io::Result<()> {
    let mut child = gui_command(&gui_exe(), data_dir, open_conv).spawn()?;
    // 后台回收，避免 Unix 下留僵尸进程。
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn args(cmd: &Command) -> Vec<&OsStr> {
        cmd.get_args().collect()
    }

    #[test]
    fn gui_command_args_without_conv() {
        let cmd = gui_command(Path::new("lanmsg"), Path::new("/d"), None);
        assert_eq!(args(&cmd), ["--data-dir", "/d"]);
    }

    #[test]
    fn gui_command_args_with_conv() {
        let cmd = gui_command(Path::new("lanmsg"), Path::new("/d"), Some("abc"));
        assert_eq!(args(&cmd), ["--data-dir", "/d", "--open-conv", "abc"]);
    }

    #[test]
    fn empty_or_missing_tag_means_no_conv() {
        assert_eq!(conv_from_tag(None), None);
        assert_eq!(conv_from_tag(Some(String::new())), None);
        assert_eq!(conv_from_tag(Some("c1".into())), Some("c1".into()));
    }

    #[test]
    fn gui_exe_is_sibling_named_lanmsg() {
        let exe = gui_exe();
        assert!(exe
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("lanmsg"));
        assert_eq!(exe.parent(), std::env::current_exe().unwrap().parent());
    }
}
