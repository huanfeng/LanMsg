// 开机自启：注册的是与界面同目录的 lanmsgd。
// Windows 写 HKCU Run；Linux 写 ~/.config/autostart/lanmsgd.desktop；macOS 写 LaunchAgents。
// 注释中文，标识符英文。

use std::io;
use std::path::{Path, PathBuf};

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

/// 开启 / 关闭开机自启。`data_dir` 非默认目录时，自启项会带上 `--data-dir`。
pub fn set_enabled(on: bool, data_dir: &Path) -> io::Result<()> {
    let extra = (data_dir != lanmsg_core::paths::default_data_dir()).then_some(data_dir);
    imp::set_enabled(on, &daemon_exe()?, extra)
}

/// desktop entry 的 Exec 参数转义。规范里 Exec 值先受通用字符串转义（`\\`→`\`），
/// 再受 Exec 引号内转义，所以字面 `"`、`` ` ``、`$`、`\` 在文件里要写成「两个反斜杠 + 字符」
/// （字面反斜杠即四个反斜杠）；`%` 写作 `%%`（否则会被当作字段代码）。
#[cfg(any(target_os = "linux", test))]
fn escape_exec_arg(arg: &str) -> String {
    let mut out = String::from("\"");
    for ch in arg.chars() {
        match ch {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(ch);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Linux 自启项内容。
#[cfg(any(target_os = "linux", test))]
fn desktop_entry(exe: &Path, data_dir: Option<&Path>) -> String {
    let mut exec = escape_exec_arg(&exe.to_string_lossy());
    if let Some(d) = data_dir {
        exec.push_str(" --data-dir ");
        exec.push_str(&escape_exec_arg(&d.to_string_lossy()));
    }
    format!(
        "[Desktop Entry]\nType=Application\nName=LanMsg\nExec={exec}\nX-GNOME-Autostart-enabled=true\nTerminal=false\n"
    )
}

/// XML 文本转义（& < > " '）。
#[cfg(any(target_os = "macos", test))]
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// macOS LaunchAgent plist 内容。
#[cfg(any(target_os = "macos", test))]
fn plist_content(exe: &Path, data_dir: Option<&Path>) -> String {
    let mut args = format!("<string>{}</string>", xml_escape(&exe.to_string_lossy()));
    if let Some(d) = data_dir {
        args.push_str(&format!(
            "<string>--data-dir</string><string>{}</string>",
            xml_escape(&d.to_string_lossy())
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n\
<key>Label</key><string>app.lanmsg.daemon</string>\n\
<key>ProgramArguments</key><array>{args}</array>\n\
<key>RunAtLoad</key><true/>\n\
</dict>\n</plist>\n"
    )
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

    pub fn set_enabled(on: bool, exe: &Path, data_dir: Option<&Path>) -> io::Result<()> {
        let key = open_run_key(KEY_SET_VALUE)?;
        let result = if on {
            // 路径加引号，避免含空格时被拆开；REG_SZ 要求以 NUL 结尾的 UTF-16 字节
            let mut value = format!("\"{}\"", exe.display());
            if let Some(d) = data_dir {
                value.push_str(&format!(" --data-dir \"{}\"", d.display()));
            }
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

    pub(super) fn set_enabled_in(
        home: &Path,
        on: bool,
        exe: &Path,
        data_dir: Option<&Path>,
    ) -> io::Result<()> {
        let path = desktop_path(home);
        if on {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, super::desktop_entry(exe, data_dir))
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

    pub fn set_enabled(on: bool, exe: &Path, data_dir: Option<&Path>) -> io::Result<()> {
        set_enabled_in(&home_dir()?, on, exe, data_dir)
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

    pub fn set_enabled(on: bool, exe: &Path, data_dir: Option<&Path>) -> io::Result<()> {
        let path = plist_path()?;
        if on {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, super::plist_content(exe, data_dir))
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

    pub fn set_enabled(_on: bool, _exe: &Path, _data_dir: Option<&Path>) -> io::Result<()> {
        Err(io::Error::other("当前平台不支持开机自启"))
    }

    pub fn is_enabled() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按 desktop entry 规范反向解析：先通用字符串转义（`\\`→`\`），
    /// 再去掉引号并还原 Exec 引号内转义（`\"` `\`` `\$` `\\`）与 `%%`。
    fn unescape_exec_arg(text: &str) -> String {
        let mut stage1 = String::new();
        let mut it = text.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                stage1.push(it.next().expect("悬空反斜杠"));
            } else {
                stage1.push(c);
            }
        }
        let inner = stage1
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .expect("应被双引号包裹");
        let mut out = String::new();
        let mut it = inner.chars();
        while let Some(c) = it.next() {
            match c {
                '\\' => out.push(it.next().expect("悬空反斜杠")),
                '%' => {
                    assert_eq!(it.next(), Some('%'), "% 必须写作 %%");
                    out.push('%');
                }
                _ => out.push(c),
            }
        }
        out
    }

    #[test]
    fn escape_exec_arg_handles_special_chars() {
        assert_eq!(escape_exec_arg("/opt/a b/x"), "\"/opt/a b/x\"");
        assert_eq!(escape_exec_arg(r#"a"b"#), r#""a\\"b""#);
        assert_eq!(escape_exec_arg("a$b`c"), r#""a\\$b\\`c""#);
        assert_eq!(escape_exec_arg(r"a\b"), r#""a\\\\b""#);
        assert_eq!(escape_exec_arg("100%"), "\"100%%\"");
    }

    #[test]
    fn escape_exec_arg_roundtrips_through_spec_unescape() {
        for path in [
            "/opt/a b/x",
            r"/we\ird/pa th",
            r#"/q"uote/x"#,
            "/d$ollar/`tick`/100%/x y",
            r#"\"$`% mix"#,
        ] {
            assert_eq!(unescape_exec_arg(&escape_exec_arg(path)), path);
        }
    }

    #[test]
    fn desktop_entry_includes_data_dir_when_given() {
        let t = desktop_entry(Path::new("/opt/lan msg/lanmsgd"), None);
        assert!(t.contains("Exec=\"/opt/lan msg/lanmsgd\"\n"));
        let t = desktop_entry(Path::new("/opt/lanmsgd"), Some(Path::new("/data/my dir")));
        assert!(t.contains("Exec=\"/opt/lanmsgd\" --data-dir \"/data/my dir\"\n"));
    }

    #[test]
    fn xml_escape_covers_five_chars() {
        assert_eq!(
            xml_escape("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
    }

    #[test]
    fn plist_escapes_path_and_includes_data_dir() {
        let t = plist_content(Path::new("/Apps/R&D/lanmsgd"), Some(Path::new("/d/<x>")));
        assert!(t.contains("<string>/Apps/R&amp;D/lanmsgd</string>"));
        assert!(t.contains("<string>--data-dir</string><string>/d/&lt;x&gt;</string>"));
        let t = plist_content(Path::new("/a/lanmsgd"), None);
        assert!(!t.contains("--data-dir"));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::imp::*;
    use std::path::Path;

    #[test]
    fn enable_writes_desktop_entry_and_disable_removes_it() {
        let home = tempfile::tempdir().expect("创建临时 HOME");
        let exe = Path::new("/opt/lanmsg/lanmsgd");
        assert!(!is_enabled_in(home.path()));
        set_enabled_in(home.path(), true, exe, None).expect("开启应成功");
        assert!(is_enabled_in(home.path()));
        let text = std::fs::read_to_string(desktop_path(home.path())).expect("读取自启项");
        assert!(text.contains("Exec=\"/opt/lanmsg/lanmsgd\""));
        assert!(text.contains("X-GNOME-Autostart-enabled=true"));
        set_enabled_in(home.path(), false, exe, None).expect("关闭应成功");
        assert!(!is_enabled_in(home.path()));
    }

    #[test]
    fn disable_when_not_enabled_is_ok() {
        let home = tempfile::tempdir().expect("创建临时 HOME");
        set_enabled_in(home.path(), false, Path::new("/x"), None).expect("本就未开启时关闭应成功");
    }
}
