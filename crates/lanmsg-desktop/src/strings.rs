// 界面文案集中处（v1 不做 i18n）。注释中文，标识符英文。

/// 窗口标题。
pub const APP_TITLE: &str = "LanMsg";

/// 后台服务断开时的顶部提示条。
pub const DISCONNECTED_BANNER: &str = "未连接后台服务，正在重试…";

/// 侧栏页签名。
pub const NAV_CHATS: &str = "会话";
pub const NAV_CONTACTS: &str = "联系人";
pub const NAV_FILES: &str = "文件";
pub const NAV_SETTINGS: &str = "设置";

/// 各页占位标题（后续任务替换为真实内容）。
pub const PAGE_WIZARD: &str = "欢迎使用 LanMsg";
pub const PAGE_CHATS: &str = "会话";
pub const PAGE_CONTACTS: &str = "联系人";
pub const PAGE_FILES: &str = "文件";
pub const PAGE_SETTINGS: &str = "设置";

/// 传输失败提示前缀，后接错误原因。
pub const DOWNLOAD_FAILED_PREFIX: &str = "下载失败：";
/// 传输失败但无原因时的兜底文字。
pub const UNKNOWN_ERROR: &str = "未知错误";
/// 转发成功提示。
pub const FORWARDED: &str = "已转发";
pub const NOT_CONNECTED: &str = "未连接后台服务";
