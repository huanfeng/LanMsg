// P1 核心骨架：平台数据目录（C8）。
// default_data_dir 返回 LanMsg 的平台默认数据目录。
// 注释中文，标识符英文。

use directories::ProjectDirs;
use std::path::PathBuf;

/// LanMsg 应用标识，用于查找平台默认数据目录。
const QUALIFIER: &str = "com";
const ORGANIZATION: &str = "lanmsg";
const APPLICATION: &str = "LanMsg";

/// 返回平台默认数据目录。
///
/// - Windows: `%APPDATA%\LanMsg`
/// - macOS: `~/Library/Application Support/LanMsg`
/// - Linux: `$XDG_DATA_HOME/lanmsg`（或 `~/.local/share/lanmsg`）
pub fn default_data_dir() -> PathBuf {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("lanmsg"))
}
