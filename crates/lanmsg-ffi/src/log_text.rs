// 日志文本转 C 字符串（与平台无关，便于在宿主机上测试）。
// 注释中文，标识符英文。

use std::ffi::{CString, NulError};

/// 日志行转 C 字符串；含 NUL 的字节替换成 '?'，不丢整行。
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn to_c_text(line: &str) -> Result<CString, NulError> {
    CString::new(line.replace('\0', "?"))
}

#[cfg(test)]
mod tests {
    use super::to_c_text;

    #[test]
    fn nul_bytes_are_replaced_not_dropped() {
        let c = to_c_text("a\0b").unwrap();
        assert_eq!(c.to_str().unwrap(), "a?b");
    }
}
