// 开机自启：注册的是与界面同目录的 lanmsgd。
// Windows 写 HKCU Run；Linux 写 ~/.config/autostart/lanmsgd.desktop；macOS 写 LaunchAgents。
// 注释中文，标识符英文。

use std::io;
use std::path::PathBuf;

/// 与当前可执行文件同目录的 lanmsgd 绝对路径。
fn daemon_exe() -> io::Result<PathBuf> {
    let name = if cfg!(windows) {
        "lanmsgd.exe"
    } else {
        "lanmsgd"
    };
    let dir = std::env::current_exe()?
        .parent()
        .map(|d| d.to_path_buf())
        .ok_or_else(|| io::Error::other("无法确定可执行文件目录"))?;
    Ok(dir.join(name))
}

/// 开启 / 关闭开机自启。
pub fn set_enabled(on: bool) -> io::Result<()> {
    imp::set_enabled(on, &daemon_exe()?)
}

/// 当前是否已开启开机自启。
pub fn is_enabled() -> bool {
    imp::is_enabled()
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::path::Path;

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
    };

    fn to_io(e: WIN32_ERROR) -> io::Error {
        io::Error::from_raw_os_error(e.0 as i32)
    }

    /// 打开 Run 键；调用方负责 RegCloseKey。
    fn open_run_key(access: REG_SAM_FLAGS) -> io::Result<HKEY> {
        let mut key = HKEY::default();
        // SAFETY: 子键名是 'static 宽字符串，key 指向有效的局部变量。
        let err = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
                0,
                access,
                &mut key,
            )
        };
        if err == ERROR_SUCCESS {
            Ok(key)
        } else {
            Err(to_io(err))
        }
    }

    pub fn set_enabled(on: bool, exe: &Path) -> io::Result<()> {
        let key = open_run_key(KEY_SET_VALUE)?;
        let result = if on {
            // 路径加引号，避免含空格时被拆开；REG_SZ 要求以 NUL 结尾的 UTF-16 字节
            let value = format!("\"{}\"", exe.display());
            let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes: Vec<u8> = wide.iter().flat_map(|u| u.to_le_bytes()).collect();
            // SAFETY: key 已成功打开；bytes 在调用期间有效。
            let err = unsafe { RegSetValueExW(key, w!("LanMsg"), 0, REG_SZ, Some(&bytes)) };
            if err == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(to_io(err))
            }
        } else {
            // SAFETY: key 已成功打开。
            let err = unsafe { RegDeleteValueW(key, w!("LanMsg")) };
            // 本来就没有该值也算关闭成功
            if err == ERROR_SUCCESS || err == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(to_io(err))
            }
        };
        // SAFETY: key 由 open_run_key 打开且尚未关闭。
        let _ = unsafe { RegCloseKey(key) };
        result
    }

    pub fn is_enabled() -> bool {
        let Ok(key) = open_run_key(KEY_QUERY_VALUE) else {
            return false;
        };
        // SAFETY: key 已成功打开；不取数据，只探测值是否存在。
        let err = unsafe { RegQueryValueExW(key, PCWSTR(w!("LanMsg").0), None, None, None, None) };
        // SAFETY: key 由 open_run_key 打开且尚未关闭。
        let _ = unsafe { RegCloseKey(key) };
        err == ERROR_SUCCESS
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    fn home_dir() -> io::Result<PathBuf> {
        std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("未设置 HOME"))
    }

    /// 自启项文件路径。
    pub(super) fn desktop_path(home: &Path) -> PathBuf {
        home.join(".config/autostart/lanmsgd.desktop")
    }

    pub(super) fn desktop_entry(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=LanMsg\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\nTerminal=false\n",
            exe.display()
        )
    }

    pub(super) fn set_enabled_in(home: &Path, on: bool, exe: &Path) -> io::Result<()> {
        let path = desktop_path(home);
        if on {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, desktop_entry(exe))
        } else {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            }
        }
    }

    pub(super) fn is_enabled_in(home: &Path) -> bool {
        desktop_path(home).is_file()
    }

    pub fn set_enabled(on: bool, exe: &Path) -> io::Result<()> {
        set_enabled_in(&home_dir()?, on, exe)
    }

    pub fn is_enabled() -> bool {
        home_dir().map(|h| is_enabled_in(&h)).unwrap_or(false)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    fn plist_path() -> io::Result<PathBuf> {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("未设置 HOME"))?;
        Ok(home.join("Library/LaunchAgents/app.lanmsg.daemon.plist"))
    }

    fn plist(exe: &Path) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n\
<key>Label</key><string>app.lanmsg.daemon</string>\n\
<key>ProgramArguments</key><array><string>{}</string></array>\n\
<key>RunAtLoad</key><true/>\n\
</dict>\n</plist>\n",
            exe.display()
        )
    }

    pub fn set_enabled(on: bool, exe: &Path) -> io::Result<()> {
        let path = plist_path()?;
        if on {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, plist(exe))
        } else {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            }
        }
    }

    pub fn is_enabled() -> bool {
        plist_path().map(|p| p.is_file()).unwrap_or(false)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    use std::io;
    use std::path::Path;

    pub fn set_enabled(_on: bool, _exe: &Path) -> io::Result<()> {
        Err(io::Error::other("当前平台不支持开机自启"))
    }

    pub fn is_enabled() -> bool {
        false
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::imp::*;
    use std::path::Path;

    #[test]
    fn enable_writes_desktop_entry_and_disable_removes_it() {
        let home = tempfile::tempdir().expect("创建临时 HOME");
        let exe = Path::new("/opt/lanmsg/lanmsgd");
        assert!(!is_enabled_in(home.path()));
        set_enabled_in(home.path(), true, exe).expect("开启应成功");
        assert!(is_enabled_in(home.path()));
        let text = std::fs::read_to_string(desktop_path(home.path())).expect("读取自启项");
        assert!(text.contains("Exec=\"/opt/lanmsg/lanmsgd\""));
        assert!(text.contains("X-GNOME-Autostart-enabled=true"));
        set_enabled_in(home.path(), false, exe).expect("关闭应成功");
        assert!(!is_enabled_in(home.path()));
    }

    #[test]
    fn disable_when_not_enabled_is_ok() {
        let home = tempfile::tempdir().expect("创建临时 HOME");
        set_enabled_in(home.path(), false, Path::new("/x")).expect("本就未开启时关闭应成功");
    }
}
