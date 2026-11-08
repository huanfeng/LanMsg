// Windows 防火墙入站规则：检测与添加（经 netsh advfirewall）。
// 规则名与参数和安装包清单（packaging/windows/app.toml 的 [[firewall]]）一致；
// 纯函数（参数拼装、输出判断）跨平台编译以便单测，netsh 调用与提权只在 Windows 编译。
// 注释中文，标识符英文。

use std::path::Path;

/// 防火墙规则名；与 `packaging/windows/app.toml` 中 `[[firewall]].name` 一致。
pub const FIREWALL_RULE_NAME: &str = "LanMsg";

/// `show rule` 的参数。不带 `program=`（该子命令不接受），按名字查 verbose 输出。
pub fn show_rule_args() -> Vec<String> {
    vec![
        "advfirewall".into(),
        "firewall".into(),
        "show".into(),
        "rule".into(),
        format!("name={FIREWALL_RULE_NAME}"),
        "verbose".into(),
    ]
}

/// `add rule` 的参数：入站放行 `program`，私有与域网络，任意协议（与安装包一致）。
pub fn add_rule_args(program: &Path) -> Vec<String> {
    vec![
        "advfirewall".into(),
        "firewall".into(),
        "add".into(),
        "rule".into(),
        format!("name={FIREWALL_RULE_NAME}"),
        "dir=in".into(),
        "action=allow".into(),
        format!("program={}", program.display()),
        "enable=yes".into(),
        "profile=private,domain".into(),
        "protocol=any".into(),
    ]
}

/// `show rule ... verbose` 的（已解码）输出里是否有一条规则放行 `program`：
/// 按 `-----` 分隔的块判断，同一块内须含程序路径（不区分大小写的字面量），且
/// 已启用（Enabled: Yes / 已启用: 是）、动作为允许（Action: Allow / 操作: 允许）、
/// 方向为入站（Direction: In / 方向: 入站）。被禁用、拦截或出站的同名规则不算。
/// 输出文案随系统语言变化，只认中英文标签与取值；调用方须先按系统 OEM 代码页解码，
/// 否则含非 ASCII 的路径无法比较。
pub fn rule_allows_program(output: &str, program: &Path) -> bool {
    let program = program.to_string_lossy().to_lowercase();
    output.split("-----").any(|block| {
        // 分隔线之后残留的横线字符不影响判断；块内逐行取“标签: 取值”
        let lower = block.to_lowercase();
        lower.contains(&program)
            && field_is(block, &["enabled", "已启用"], &["yes", "是"])
            && field_is(block, &["action", "操作"], &["allow", "允许"])
            && field_is(block, &["direction", "方向"], &["in", "入站"])
    })
}

/// 块内标签（任一）对应的取值是否为 `accepted` 之一（忽略大小写与首尾空白）。
fn field_is(block: &str, keys: &[&str], accepted: &[&str]) -> bool {
    block.lines().any(|line| {
        // 英文冒号或全角冒号分隔标签与取值
        let Some((key, value)) = line.split_once([':', '：']) else {
            return false;
        };
        let key = key.trim().to_lowercase();
        keys.iter().any(|k| key == *k) && {
            let value = value.trim().to_lowercase();
            accepted.iter().any(|a| value == *a)
        }
    })
}

#[cfg(windows)]
pub use win::{add_rule, elevate_add_rule, firewall_rule_present};

#[cfg(windows)]
mod win {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use windows::core::{w, HRESULT, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::Globalization::{
        GetOEMCP, MultiByteToWideChar, MULTI_BYTE_TO_WIDE_CHAR_FLAGS,
    };
    use windows::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    use super::{add_rule_args, rule_allows_program, show_rule_args};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// 用户在 UAC 对话框里取消。
    const ERROR_CANCELLED: u32 = 1223;

    /// 用 System32 下的绝对路径，避免被当前目录或 PATH 里同名 netsh.exe 劫持。
    fn netsh_path() -> PathBuf {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        PathBuf::from(root).join("System32").join("netsh.exe")
    }

    /// 按系统 OEM 代码页解码 netsh 的输出（中文系统是 GBK 等，UTF-8 解码会乱码，
    /// 含非 ASCII 的程序路径就无法比较）。转换失败时退回 UTF-8 宽松解码。
    fn decode_oem(bytes: &[u8]) -> String {
        if bytes.is_empty() {
            return String::new();
        }
        // SAFETY: 两次调用都只读 bytes；第二次的输出缓冲区长度取自第一次的返回值。
        unsafe {
            let cp = GetOEMCP();
            let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
            let len = MultiByteToWideChar(cp, flags, bytes, None);
            if len <= 0 {
                return String::from_utf8_lossy(bytes).to_string();
            }
            let mut buf = vec![0u16; len as usize];
            let written = MultiByteToWideChar(cp, flags, bytes, Some(&mut buf));
            if written <= 0 {
                return String::from_utf8_lossy(bytes).to_string();
            }
            String::from_utf16_lossy(&buf[..written as usize])
        }
    }

    /// 运行 netsh，返回（退出码，按 OEM 代码页解码的 stdout）。
    fn run_netsh(args: &[String]) -> io::Result<(i32, String)> {
        let out = Command::new(netsh_path())
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;
        Ok((out.status.code().unwrap_or(-1), decode_oem(&out.stdout)))
    }

    /// 规则是否已放行 `program`：`show rule` 非零退出码表示该名下没有规则；命令无法执行视为不存在。
    pub fn firewall_rule_present(program: &Path) -> bool {
        match run_netsh(&show_rule_args()) {
            Ok((0, out)) => rule_allows_program(&out, program),
            _ => false,
        }
    }

    /// 添加放行规则（需要管理员权限）；已存在则不重复添加。
    pub fn add_rule(program: &Path) -> Result<(), String> {
        if firewall_rule_present(program) {
            return Ok(());
        }
        let (code, out) =
            run_netsh(&add_rule_args(program)).map_err(|e| format!("failed to run netsh: {e}"))?;
        if code == 0 {
            Ok(())
        } else {
            // netsh 把错误写在 stdout
            Err(format!("netsh failed ({code}): {}", out.trim()))
        }
    }

    /// 以管理员身份（`runas`）运行 `exe --add-firewall-rule` 并等待结束：
    /// 退出码 0 返回 `Ok(true)`；用户取消 UAC 返回 `Ok(false)`。会阻塞到 UAC 与命令完成，
    /// 调用方应放在后台线程。
    pub fn elevate_add_rule(exe: &Path) -> io::Result<bool> {
        let file: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let params: Vec<u16> = "--add-firewall-rule\0".encode_utf16().collect();
        // 工作目录设为 exe 所在目录，避免提权进程继承到不可信的当前目录
        let dir: Vec<u16> = exe
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            lpDirectory: PCWSTR(dir.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: info 与其指向的宽字符串在调用期间有效；hProcess 由本函数负责关闭；
        // CoInitializeEx 成功（含 S_FALSE）才配对 CoUninitialize。
        unsafe {
            // ShellExecuteEx 要求调用线程已初始化 COM；线程若已用别的模式初始化则不重复初始化
            let com_inited =
                CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
            let result = shell_execute_and_wait(&mut info);
            if com_inited {
                CoUninitialize();
            }
            result
        }
    }

    /// 执行 `info` 并等待子进程结束，退出码 0 为 `true`；UAC 取消为 `false`。
    ///
    /// # Safety
    /// `info` 指向的字符串必须在调用期间有效。
    unsafe fn shell_execute_and_wait(info: &mut SHELLEXECUTEINFOW) -> io::Result<bool> {
        unsafe {
            if let Err(e) = ShellExecuteExW(info) {
                if e.code() == HRESULT::from_win32(ERROR_CANCELLED) {
                    return Ok(false);
                }
                return Err(io::Error::other(e));
            }
            if WaitForSingleObject(info.hProcess, INFINITE) != WAIT_OBJECT_0 {
                let _ = CloseHandle(info.hProcess);
                return Err(io::Error::last_os_error());
            }
            let mut code = 1u32;
            let got = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            got.map_err(io::Error::other)?;
            Ok(code == 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const PROGRAM: &str = r"C:\Program Files\LanMsg\lanmsgd.exe";

    const EN_OUTPUT: &str = "\r\nRule Name:                            LanMsg\r\n\
----------------------------------------------------------------------\r\n\
Enabled:                              Yes\r\n\
Direction:                            In\r\n\
Profiles:                             Domain,Private\r\n\
Grouping:\r\n\
Program:                              C:\\Program Files\\LanMsg\\lanmsgd.exe\r\n\
Action:                               Allow\r\n";

    const ZH_OUTPUT: &str = "\r\n规则名称:                            LanMsg\r\n\
----------------------------------------------------------------------\r\n\
已启用:                               是\r\n\
方向:                                 入站\r\n\
配置文件:                             域,专用\r\n\
程序:                                 c:\\program files\\lanmsg\\LANMSGD.EXE\r\n\
操作:                                 允许\r\n";

    /// 拼一条英文规则块（含分隔线）。
    fn en_rule(enabled: &str, direction: &str, action: &str, program: &str) -> String {
        format!(
            "\r\nRule Name:                            LanMsg\r\n\
-----------------------------------------------------------------\r\n\
Enabled:                              {enabled}\r\n\
Direction:                            {direction}\r\n\
Profiles:                             Domain,Private\r\n\
Program:                              {program}\r\n\
Action:                               {action}\r\n"
        )
    }

    /// 拼一条中文规则块。
    fn zh_rule(enabled: &str, direction: &str, action: &str, program: &str) -> String {
        format!(
            "\r\n规则名称:                            LanMsg\r\n\
-----------------------------------------------------------------\r\n\
已启用:                               {enabled}\r\n\
方向:                                 {direction}\r\n\
配置文件:                             域,专用\r\n\
程序:                                 {program}\r\n\
操作:                                 {action}\r\n"
        )
    }

    #[test]
    fn rule_allows_program_matches_english_and_chinese_labels() {
        assert!(rule_allows_program(EN_OUTPUT, Path::new(PROGRAM)));
        // 中文系统输出，路径大小写不同
        assert!(rule_allows_program(ZH_OUTPUT, Path::new(PROGRAM)));
    }

    #[test]
    fn rule_allows_program_rejects_other_program() {
        assert!(!rule_allows_program(
            EN_OUTPUT,
            Path::new(r"D:\Other\lanmsgd.exe")
        ));
        assert!(!rule_allows_program("", Path::new(PROGRAM)));
    }

    #[test]
    fn rule_allows_program_compares_non_ascii_paths_literally() {
        let program = r"C:\程序文件\LanMsg\lanmsgd.exe";
        let out = zh_rule("是", "入站", "允许", program);
        assert!(rule_allows_program(&out, Path::new(program)));
        // 另一个中文路径不能蒙混过关
        assert!(!rule_allows_program(
            &out,
            Path::new(r"C:\其他目录\LanMsg\lanmsgd.exe")
        ));
    }

    #[test]
    fn rule_allows_program_ignores_disabled_blocking_or_outbound_rules() {
        assert!(!rule_allows_program(
            &en_rule("No", "In", "Allow", PROGRAM),
            Path::new(PROGRAM)
        ));
        assert!(!rule_allows_program(
            &en_rule("Yes", "In", "Block", PROGRAM),
            Path::new(PROGRAM)
        ));
        assert!(!rule_allows_program(
            &en_rule("Yes", "Out", "Allow", PROGRAM),
            Path::new(PROGRAM)
        ));
        assert!(!rule_allows_program(
            &zh_rule("否", "入站", "允许", PROGRAM),
            Path::new(PROGRAM)
        ));
        assert!(!rule_allows_program(
            &zh_rule("是", "入站", "阻止", PROGRAM),
            Path::new(PROGRAM)
        ));
        assert!(!rule_allows_program(
            &zh_rule("是", "出站", "允许", PROGRAM),
            Path::new(PROGRAM)
        ));
    }

    #[test]
    fn rule_allows_program_needs_all_fields_in_the_same_block() {
        // 第一块禁用、第二块是别的程序：属性不能跨块拼凑
        let out = format!(
            "{}{}",
            en_rule("No", "In", "Allow", PROGRAM),
            en_rule("Yes", "In", "Allow", r"D:\Other\x.exe")
        );
        assert!(!rule_allows_program(&out, Path::new(PROGRAM)));
        // 同名规则有多条时，任一条有效即可
        let out = format!(
            "{}{}",
            en_rule("No", "In", "Allow", PROGRAM),
            en_rule("Yes", "In", "Allow", PROGRAM)
        );
        assert!(rule_allows_program(&out, Path::new(PROGRAM)));
    }

    #[test]
    fn rule_args_match_installer_manifest() {
        assert_eq!(FIREWALL_RULE_NAME, "LanMsg");
        assert_eq!(
            show_rule_args(),
            [
                "advfirewall",
                "firewall",
                "show",
                "rule",
                "name=LanMsg",
                "verbose"
            ]
        );
        assert_eq!(
            add_rule_args(Path::new(PROGRAM)),
            [
                "advfirewall",
                "firewall",
                "add",
                "rule",
                "name=LanMsg",
                "dir=in",
                "action=allow",
                r"program=C:\Program Files\LanMsg\lanmsgd.exe",
                "enable=yes",
                "profile=private,domain",
                "protocol=any",
            ]
        );
    }
}
