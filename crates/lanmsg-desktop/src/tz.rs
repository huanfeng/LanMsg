// 本地时区偏移（秒）。不引入时间库：Unix 问 `date +%z`，Windows 读系统时区信息。
// 偏移在进程生命周期内只取一次（夏令时切换后需重启才更新，对会话时间显示可接受）。
// 注释中文，标识符英文。

use std::sync::OnceLock;

/// 本地相对 UTC 的偏移秒数；取不到时按 UTC（0）。
pub fn local_offset_secs() -> i32 {
    static OFFSET: OnceLock<i32> = OnceLock::new();
    *OFFSET.get_or_init(query_offset)
}

/// 解析 `date +%z` 输出，如 `+0800`、`-0330`。
pub fn parse_utc_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let (sign, digits) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[..2].parse().ok()?;
    let m: i32 = digits[2..].parse().ok()?;
    Some(sign * (h * 3600 + m * 60))
}

#[cfg(not(windows))]
fn query_offset() -> i32 {
    std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|o| parse_utc_offset(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(0)
}

#[cfg(windows)]
fn query_offset() -> i32 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    // SAFETY: 只写入栈上的 TIME_ZONE_INFORMATION，无其它前置条件。
    let mut tz = TIME_ZONE_INFORMATION::default();
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    // 返回 2 表示当前处于夏令时
    let extra = if id == 2 {
        tz.DaylightBias
    } else {
        tz.StandardBias
    };
    -(tz.Bias + extra) * 60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_utc_offset_cases() {
        assert_eq!(parse_utc_offset("+0800\n"), Some(28_800));
        assert_eq!(parse_utc_offset("-0330"), Some(-12_600));
        assert_eq!(parse_utc_offset("+0000"), Some(0));
        assert_eq!(parse_utc_offset("08:00"), None);
        assert_eq!(parse_utc_offset(""), None);
    }
}
