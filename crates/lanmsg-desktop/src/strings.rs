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
pub const WIZARD_NAME_REQUIRED_HINT: &str = "填写名字后继续";
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
pub const CHATS_EMPTY_LIST_HINT: &str = "添加联系人后，会话会出现在这里";
pub const CHAT_PANE_EMPTY: &str = "选择一个会话开始聊天";
pub const CHAT_PANE_EMPTY_HINT: &str = "从左侧选择一个会话，或在“联系人”里添加设备";
pub const CHAT_PANE_EMPTY_ACTION: &str = "去添加设备";
pub const YESTERDAY: &str = "昨天";
pub const UNKNOWN_CONV: &str = "未知会话";

/// 聊天区。
pub const CHAT_ONLINE: &str = "在线";
pub const CHAT_OFFLINE: &str = "离线";
pub const PATH_VPN: &str = "VPN";
pub const PATH_INTERNET: &str = "公网";
pub const PATH_RELAY: &str = "中继";
pub const MSG_RECALLED: &str = "消息已撤回";
pub const MSG_UNSUPPORTED: &str = "不支持的消息类型，请升级";
pub const MSG_SENDING: &str = "发送中";
pub const MSG_FAILED: &str = "发送失败";
pub const MENU_COPY: &str = "复制";
pub const MENU_RECALL: &str = "撤回";
pub const MENU_DELETE: &str = "删除";
pub const MENU_FORWARD: &str = "转发…";
pub const FILE_DOWNLOAD: &str = "下载";
pub const FILE_CANCEL: &str = "取消";
pub const FILE_OPEN: &str = "打开";
pub const FILE_OPEN_FOLDER: &str = "打开所在文件夹";
pub const FILE_CLICK_TO_DOWNLOAD: &str = "点击下载";
pub const FILE_SOURCE_OFFLINE: &str = "源设备离线";
pub const FILE_PREPARING: &str = "准备中…";
pub const FILE_DOWNLOADING: &str = "下载中";
pub const FILE_COUNT_UNIT: &str = "个文件";
pub const COMPOSER_PLACEHOLDER: &str = "输入消息，Enter 发送，Shift+Enter 换行";
pub const COMPOSER_SEND: &str = "发送";
pub const COMPOSER_EMPTY_HINT: &str = "输入内容后才能发送";
pub const COMPOSER_FILE: &str = "文件";
pub const COMPOSER_FOLDER: &str = "文件夹";
pub const PICK_FILES_TITLE: &str = "选择要发送的文件";
pub const PICK_FOLDER_TITLE: &str = "选择要发送的文件夹";
pub const PICK_UNAVAILABLE: &str = "无法打开选择框，可把文件拖入窗口发送";
pub const PASTE_TITLE: &str = "发送这张图片？";
pub const PASTE_SEND: &str = "发送";
pub const PASTE_CANCEL: &str = "取消";
pub const PASTE_SAVE_FAILED: &str = "保存粘贴的图片失败：";
pub const PASTE_NAME_EXHAUSTED: &str = "同一时刻粘贴的图片过多";
pub const NEW_MESSAGES_BANNER: &str = "有新消息，点击查看最新";

/// 转发对话框。
pub const FORWARD_TITLE: &str = "转发到";
pub const FORWARD_SEND: &str = "发送";
pub const FORWARD_CANCEL: &str = "取消";
pub const FORWARD_PICK_HINT: &str = "选择一个会话后才能发送";
pub const FORWARD_EMPTY: &str = "还没有可转发的会话";
