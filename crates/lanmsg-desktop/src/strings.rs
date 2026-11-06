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

/// 首次向导。
pub const WIZARD_TITLE: &str = "欢迎使用 LanMsg";
pub const WIZARD_SUBTITLE: &str = "在局域网内直接收发消息和文件，无需账号与服务器。";
pub const WIZARD_NAME_LABEL: &str = "你的名字";
pub const WIZARD_NAME_PLACEHOLDER: &str = "显示给对方看，1–32 个字符";
pub const WIZARD_DEVICE_LABEL: &str = "这台设备的名称";
pub const WIZARD_DEVICE_PLACEHOLDER: &str = "例如：办公室电脑";
pub const WIZARD_FIRST_DEVICE: &str = "这是我的第一台设备";
pub const WIZARD_FIRST_DEVICE_HINT: &str = "创建新的身份，之后可以把其它设备加进来。";
pub const WIZARD_JOIN_EXISTING: &str = "加入我已有的设备";
pub const WIZARD_JOIN_HINT: &str = "把这台电脑加入你已有的 LanMsg 身份，共享同一份联系人。";
pub const WIZARD_JOINING: &str = "已记录设备名，请在你已有的设备上选择“添加我的设备”完成配对。";
pub const WIZARD_NAME_INVALID: &str = "名字需要 1–32 个字符";
pub const WIZARD_DEVICE_INVALID: &str = "设备名称需要 1–32 个字符";
pub const WIZARD_BACK: &str = "返回";
pub const AUTOSTART_FAILED: &str = "设置开机自启失败：";
/// 设备名输入为空且取不到主机名时的兜底。
pub const DEFAULT_DEVICE_NAME: &str = "我的电脑";

/// 会话列表。
pub const SEARCH_PLACEHOLDER: &str = "搜索聊天记录";
pub const SEARCH_EMPTY: &str = "没有找到相关聊天记录";
pub const CHATS_EMPTY_LIST: &str = "还没有会话";
pub const CHAT_PANE_EMPTY: &str = "选择一个会话开始聊天";
pub const YESTERDAY: &str = "昨天";
pub const UNKNOWN_CONV: &str = "未知会话";
