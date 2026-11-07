// 界面文案集中处（v1 不做 i18n）。注释中文，标识符英文。

/// 窗口标题。
pub const APP_TITLE: &str = "LanMsg";

/// 后台服务断开时的顶部提示条。
pub const DISCONNECTED_BANNER: &str = "未连接后台服务，正在重试…";
/// 后台服务被用户主动退出（托盘“退出”）后的提示条与按钮。
pub const DAEMON_QUIT_BANNER: &str = "后台服务已退出";
pub const DAEMON_START_BUTTON: &str = "启动后台服务";
/// 点击“启动后台服务”后、连接建立前的状态说明。
pub const DAEMON_STARTING: &str = "正在启动后台服务…";
/// 拉起 lanmsgd 失败的原因。
pub const LAUNCH_NOT_FOUND: &str = "找不到 lanmsgd，请确认它与 lanmsg 在同一目录";
pub const LAUNCH_FAILED_PREFIX: &str = "无法启动 lanmsgd：";

/// 侧栏页签名。
pub const NAV_CHATS: &str = "会话";
pub const NAV_CONTACTS: &str = "联系人";
pub const NAV_FILES: &str = "文件";
pub const NAV_SETTINGS: &str = "设置";

/// 页面标题。
pub const PAGE_FILES: &str = "文件";
pub const PAGE_SETTINGS: &str = "设置";

/// 传输失败原因文案（按 TransferFailReason）。
pub const DOWNLOAD_CANCELLED: &str = "已取消下载";
pub const DOWNLOAD_SOURCE_OFFLINE: &str = "下载失败：文件所在设备不在线";
pub const DOWNLOAD_VERIFY_FAILED: &str = "下载失败：文件校验不通过";
pub const DOWNLOAD_IO_FAILED: &str = "下载失败：无法写入文件";
pub const DOWNLOAD_FAILED: &str = "下载失败";
/// 错误码文案（按 ErrorCode）。
pub const ERR_NOT_INITIALIZED: &str = "请先完成初始设置";
pub const ERR_ALREADY_INITIALIZED: &str = "已经完成初始设置";
pub const ERR_NOT_FOUND: &str = "内容不存在或已被删除";
pub const ERR_INVALID_ARGUMENT: &str = "输入有误，请检查后重试";
pub const ERR_PEER_OFFLINE: &str = "对方不在线";
pub const ERR_REJECTED: &str = "操作被拒绝";
pub const ERR_IO: &str = "读写文件失败";
pub const ERR_INTERNAL: &str = "出错了，请重试";
pub const ERR_UNIMPLEMENTED: &str = "此功能暂不可用";
/// 转发成功提示。
pub const FORWARDED: &str = "已转发";

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
pub const WIZARD_BUTTONS_NEED_NAME: &str = "先填写上方的名字，下面两个按钮才能使用";
pub const WIZARD_JOIN_HINT: &str = "把这台电脑加入你已有的 LanMsg 身份，共享同一份联系人。";
pub const WIZARD_JOINING: &str =
    "已记录设备名。请在你已有的设备上进“我”→“我的设备”→“添加我的设备”完成配对。";
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
pub const PREVIEW_IMAGE: &str = "[图片]";
pub const PREVIEW_VIDEO: &str = "[视频]";
pub const PREVIEW_FILE: &str = "[文件]";
pub const PREVIEW_FOLDER: &str = "[文件夹]";
pub const PREVIEW_SYSTEM: &str = "[系统消息]";

/// 聊天区。
pub const CHAT_ONLINE: &str = "在线";
pub const CHAT_OFFLINE: &str = "离线";
pub const PATH_VPN: &str = "VPN";
pub const PATH_INTERNET: &str = "公网";
pub const PATH_RELAY: &str = "中继";
pub const MSG_RECALLED: &str = "消息已撤回";
/// 系统消息文案（后接名字的为前缀）。
pub const SYS_DEVICE_ADDED: &str = "已添加设备：";
pub const SYS_DEVICE_REMOVED: &str = "已移除设备：";
pub const SYS_CONTACT_ADDED: &str = "已添加联系人：";
pub const SYS_CONTACT_UPDATED: &str = "联系人已更新：";
pub const SYS_NICKNAME_UPDATED: &str = "昵称已更新：";
pub const SYS_DELETED_PREFIX: &str = "已删除 ";
pub const SYS_DELETED_SUFFIX: &str = " 条消息";
pub const SYS_RECALLED_BY_ME: &str = "你撤回了一条消息";
pub const SYS_RECALLED_BY_PEER: &str = " 撤回了一条消息";
/// 文件来源文案：来自谁、存于哪里。
pub const SOURCE_FROM_ME: &str = "来自我";
pub const SOURCE_FROM_PREFIX: &str = "来自 ";
pub const SOURCE_DOWNLOADED: &str = "已下载";
pub const SOURCE_STORED_PREFIX: &str = "存于 ";
/// 请求预览文案（按 RequestPreviewView）。
pub const REQ_PREVIEW_FOLDER_PREFIX: &str = "文件夹 ";
pub const REQ_PREVIEW_FILES_ETC_MID: &str = " 等 ";
pub const REQ_PREVIEW_FILES_UNIT: &str = " 个文件";
/// 设备类型名称。
pub const DEVICE_TYPE_DESKTOP: &str = "台式机";
pub const DEVICE_TYPE_LAPTOP: &str = "笔记本";
pub const DEVICE_TYPE_PHONE: &str = "手机";
pub const DEVICE_TYPE_TABLET: &str = "平板";
/// 自设备会话的标题（core 给空串，界面按 kind 显示）。
pub const SELF_DEVICES_TITLE: &str = "我的设备";
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
/// 应用内预览。
pub const PREVIEW_OPEN_DEFAULT: &str = "用默认程序打开";
pub const PREVIEW_SHOW_IN_FOLDER: &str = "在文件夹中显示";
pub const PREVIEW_CLOSE: &str = "关闭";
pub const PREVIEW_LOADING: &str = "正在加载…";
pub const PREVIEW_TRUNCATED: &str = "仅显示前 1 MB";
pub const PREVIEW_BINARY: &str = "这个文件看起来不是文本，无法预览。";
pub const PREVIEW_FAILED: &str = "无法预览这个文件。";
pub const FILE_CLICK_TO_DOWNLOAD: &str = "点击下载";
pub const FILE_SOURCE_OFFLINE: &str = "源设备离线";
pub const FILE_PREPARING: &str = "准备中…";
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

/// 联系人页。
pub const TAB_CONTACTS: &str = "联系人";
pub const TAB_REQUESTS: &str = "新的请求";
pub const TAB_NEARBY: &str = "附近的人";
pub const TAB_DEVICES: &str = "我的设备";
pub const CONTACTS_EMPTY: &str = "还没有联系人";
pub const CONTACTS_EMPTY_HINT: &str = "在“附近的人”里找到对方，点“加为联系人”发出请求";
pub const REQUESTS_EMPTY: &str = "没有新的请求";
pub const REQUESTS_EMPTY_HINT: &str = "陌生人发来消息或联系人请求时，会出现在这里";
pub const NEARBY_EMPTY: &str = "附近没有发现其他人";
pub const NEARBY_EMPTY_HINT: &str = "确认对方在同一网络并已打开 LanMsg，或手动输入对方地址";
pub const CONTACT_FINGERPRINT: &str = "指纹";
pub const CONTACT_DEVICE_COUNT_UNIT: &str = "台设备";
pub const CONTACT_AUTO_ACCEPT: &str = "自动接收对方发来的文件";
pub const CONTACT_ASK_ABOVE: &str = "超过";
pub const CONTACT_ASK_ABOVE_UNIT: &str = "MB 仍询问";
pub const CONTACT_ASK_ABOVE_PLACEHOLDER: &str = "不限制";
pub const CONTACT_ASK_INVALID: &str = "请输入整数 MB，留空表示不限制";
pub const CONTACT_SAVE_POLICY: &str = "保存";
pub const CONTACT_SEND_MESSAGE: &str = "发消息";
pub const CONTACT_ADD: &str = "加为联系人";
pub const CONTACT_REQUEST_SENT: &str = "已发出联系人请求，等待对方同意";
pub const REQUEST_KIND_CONTACT: &str = "想加你为联系人";
pub const REQUEST_KIND_MESSAGE: &str = "发来消息";
pub const REQUEST_KIND_FILE: &str = "发来文件";
pub const REQUEST_ACCEPT_ONCE: &str = "接收一次";
pub const REQUEST_ADD_CONTACT: &str = "加为联系人";
pub const REQUEST_REJECT: &str = "拒绝";
pub const REQUEST_BLOCK: &str = "屏蔽";
pub const BLOCK_TITLE: &str = "屏蔽此人？";
pub const BLOCK_MESSAGE_SUFFIX: &str = "屏蔽后将不再收到对方的任何消息和请求。";
pub const CONFIRM_CANCEL: &str = "取消";
pub const MANUAL_PEER_TITLE: &str = "手动添加地址";
pub const MANUAL_PEER_PLACEHOLDER: &str = "例如 10.8.0.5 或 10.8.0.5:7000";
pub const MANUAL_PEER_INVALID: &str = "地址格式不对，请输入 IP 或主机名";
pub const MANUAL_PEER_PORT_INVALID: &str = "端口须在 1–65535 之间";
pub const MANUAL_PEER_EMPTY_HINT: &str = "先输入对方地址才能连接";
pub const NEARBY_FINGERPRINT: &str = "设备指纹";

/// 手动添加地址的说明；默认端口取协议常量。
pub fn manual_peer_hint() -> String {
    format!(
        "跨网段或 VPN 时，输入对方的 IP 或主机名，可带端口（默认 {}）",
        lanmsg_proto::consts::SERVICE_PORT
    )
}
pub const MANUAL_PEER_BUTTON: &str = "连接";
pub const MANUAL_PEER_EMPTY: &str = "请先输入对方地址";
pub const MANUAL_PEER_SENT: &str = "已连接，对方出现在“附近的人”后即可添加";

/// 我的设备与配对。
pub const DEVICES_THIS: &str = "本机";
pub const DEVICES_ONLINE: &str = "在线";
pub const DEVICES_OFFLINE: &str = "离线";
pub const DEVICES_REMOVE: &str = "移除";
pub const DEVICES_ADD: &str = "添加我的设备";
pub const DEVICES_ADD_BUSY: &str = "正在配对，完成或取消后才能再添加";
pub const DEVICES_ADD_SAS: &str = "用数字比对添加";
pub const DEVICES_ADD_HINT: &str = "扫描二维码添加手机或另一台电脑";
pub const DEVICES_ADD_SAS_HINT: &str = "无法扫码时，在两台设备上核对 6 位数字";
pub const DEVICES_REMOVE_TITLE: &str = "移除这台设备？";
pub const DEVICES_REMOVE_MESSAGE: &str = "移除后该设备将无法再收发你的消息";
pub const CANNOT_REMOVE_SELF: &str = "不能移除本机";
pub const PAIR_TITLE: &str = "添加设备";
pub const PAIR_STARTING: &str = "正在准备…";
pub const PAIR_QR_HINT: &str = "用手机 LanMsg 扫描此二维码";
pub const PAIR_QR_WAITING: &str = "等待对方扫描…";
pub const PAIR_QR_GUIDE: &str = "手机上打开 LanMsg：首次使用选“加入我已有的设备”；已在使用的，进“我”→“我的设备”→“加入其他设备”，再扫码。没有相机时，选“改用数字比对”并选中这台电脑。";
pub const PAIR_SAS_WAITING: &str = "等待对方设备加入…";
pub const PAIR_SAS_WAITING_HINT: &str = "手机上同样进入“加入其他设备”（首次使用选“加入我已有的设备”），选“改用数字比对”，再选中这台电脑。";
pub const PAIR_SAS_HINT: &str = "确认两台设备上显示的数字完全一致";
pub const PAIR_SAS_PEER: &str = "对方自称";
pub const PAIR_SAS_GO: &str = "请到该设备上核对数字";
pub const PAIR_SAS_MATCH: &str = "一致";
pub const PAIR_SAS_MISMATCH_BUTTON: &str = "不一致";
/// 配对失败原因文案（按 PairFailReason）。
pub const PAIR_FAIL_TIMEOUT: &str = "配对超时，请重新开始";
pub const PAIR_FAIL_CANCELLED: &str = "配对已取消";
pub const PAIR_FAIL_NOT_OPEN: &str = "对方没有开启配对，请在对方设备上重新打开";
pub const PAIR_FAIL_WRONG_SECRET: &str = "配对口令不正确，请重新扫描二维码";
pub const PAIR_FAIL_BUSY: &str = "对方正在与其他设备配对，请稍后再试";
pub const PAIR_FAIL_CERT_MISMATCH: &str = "对方发来的证书不属于这台设备，请重试";
pub const PAIR_FAIL_BAD_SIGNATURE: &str = "对方发来的证书验证失败，请重试";
pub const PAIR_FAIL_IDENTITY_MISMATCH: &str = "对方的身份信息不一致，请重试";
pub const PAIR_FAIL_INVALID_NAME: &str = "对方的名字不合法，请改名后重试";
pub const PAIR_FAIL_SAS_MISMATCH: &str = "两台设备上的数字不一致，已取消配对";
pub const PAIR_FAIL_UNKNOWN: &str = "配对失败，请重试";
pub const PAIR_VERIFYING: &str = "正在验证…";
pub const PAIR_DONE: &str = "已添加";
pub const PAIR_FAILED: &str = "配对失败";
pub const PAIR_RETRY: &str = "重试";
pub const PAIR_CLOSE: &str = "关闭";
pub const PAIR_CANCEL: &str = "取消";
pub const PAIR_JOIN_PICK: &str = "选择要加入的设备";
pub const PAIR_JOIN_NONE: &str = "还没有发现可加入的设备";
pub const PAIR_JOIN_NONE_HINT: &str = "在手机上打开 LanMsg，进“我”→“我的设备”→“添加我的设备”，并保持 LanMsg 在前台，然后点“刷新”。也可以粘贴配对链接。";
pub const PAIR_JOIN_REFRESH: &str = "刷新";
pub const PAIR_JOIN_LINK_LABEL: &str = "或粘贴配对链接";
pub const PAIR_JOIN_LINK_PLACEHOLDER: &str = "lanmsg://pair?…";
pub const PAIR_JOIN_LINK_INVALID: &str = "请粘贴以 lanmsg:// 开头的配对链接";
pub const PAIR_JOIN_LINK_EMPTY_HINT: &str = "先粘贴配对链接才能使用";
pub const PAIR_JOIN_LINK_BUTTON: &str = "使用链接";
pub const PAIR_JOIN_ACTION: &str = "加入";

/// 文件页。
pub const FILES_KIND_ALL: &str = "全部";
pub const FILES_KIND_IMAGE: &str = "图片";
pub const FILES_KIND_VIDEO: &str = "视频";
pub const FILES_KIND_DOCUMENT: &str = "文档";
pub const FILES_KIND_ARCHIVE: &str = "压缩包";
pub const FILES_KIND_OTHER: &str = "其他";
pub const FILES_ALL_CONVS: &str = "全部会话";
pub const FILES_STATE_ALL: &str = "全部状态";
pub const FILES_STATE_LOCAL: &str = "已下载";
pub const FILES_STATE_REMOTE: &str = "未下载";
pub const FILES_STATE_OFFLINE: &str = "源离线";
pub const FILES_SEARCH_PLACEHOLDER: &str = "按文件名搜索";
pub const FILES_EMPTY: &str = "还没有文件";
pub const FILES_EMPTY_HINT: &str = "别人发来或你发出的文件会汇总在这里";
pub const FILES_EMPTY_FILTERED: &str = "没有符合条件的文件";
pub const FILES_EMPTY_FILTERED_HINT: &str = "换一个类型、会话或关键词试试";
pub const FILES_LOADING: &str = "正在加载文件…";
pub const FILES_LOADING_MORE: &str = "正在加载更多…";
pub const FILES_MENU_OPEN: &str = "打开";
pub const FILES_MENU_OPEN_FOLDER: &str = "打开所在文件夹";
pub const FILES_MENU_DOWNLOAD: &str = "下载";
pub const FILES_MENU_FORWARD: &str = "转发到会话…";
pub const FILES_MENU_LOCATE: &str = "定位到会话";
pub const FILES_MENU_DELETE: &str = "删除本地副本…";
pub const FILES_DELETE_TITLE: &str = "删除本地副本？";
pub const FILES_DELETE_MESSAGE: &str = "会从本机的聊天记录和文件列表中移除，对方的记录不受影响。";
pub const FILES_DELETE_ACTION: &str = "删除";

/// 设置页。
pub const SETTINGS_SAVED: &str = "设置已保存";
pub const SETTINGS_SAVE: &str = "保存";
pub const SETTINGS_SAVE_BLOCKED: &str = "有内容不符合要求，改正后才能保存";
pub const SETTINGS_SAVING: &str = "正在保存";
/// 自动下载上限超出可填范围（如 Android 写入的“不限”）时输入框里显示的文字。
pub const SETTINGS_MB_UNLIMITED: &str = "不限";
pub const SETTINGS_SAVE_CLEAN: &str = "没有修改";
pub const SETTINGS_LOADING: &str = "正在读取设置…";
pub const SETTINGS_GROUP_PERSONAL: &str = "个人";
pub const SETTINGS_GROUP_DEVICES: &str = "我的设备";
pub const SETTINGS_GROUP_STORAGE: &str = "下载与存储";
pub const SETTINGS_GROUP_NETWORK: &str = "网络";
pub const SETTINGS_GROUP_GENERAL: &str = "通用";
pub const SETTINGS_GROUP_ABOUT: &str = "关于";
pub const SETTINGS_DISPLAY_NAME: &str = "显示名";
pub const SETTINGS_DISPLAY_NAME_HINT: &str = "对方看到的名字，1–32 个字符";
pub const SETTINGS_DISPLAY_NAME_INVALID: &str = "显示名需要 1–32 个字符";
pub const SETTINGS_DEVICE_NAME: &str = "设备名";
pub const SETTINGS_DEVICE_NAME_HINT: &str = "这台电脑在你的设备列表里的名字，1–32 个字符";
pub const SETTINGS_DEVICE_NAME_INVALID: &str = "设备名需要 1–32 个字符";
pub const SETTINGS_DOWNLOAD_DIR: &str = "下载目录";
pub const SETTINGS_DOWNLOAD_DIR_INVALID: &str = "下载目录不能为空";
pub const SETTINGS_PICK_DIR: &str = "选择…";
pub const SETTINGS_PICK_DIR_TITLE: &str = "选择下载目录";
pub const SETTINGS_PICK_DIR_FAILED: &str = "无法打开选择框，可直接输入目录路径";
pub const SETTINGS_AUTO_IMAGE: &str = "图片自动下载上限";
pub const SETTINGS_AUTO_FILE: &str = "文件自动下载上限";
pub const SETTINGS_MB: &str = "MB";
pub const SETTINGS_MB_INVALID: &str = "请输入 0–10240 之间的整数";
pub const SETTINGS_AUTO_HINT: &str =
    "联系人发来的内容不超过上限时自动下载，超过则等你点击下载；填 0 表示不自动下载";
pub const SETTINGS_WIFI_ONLY: &str = "仅在 Wi-Fi 或有线网络下自动下载文件";
pub const SETTINGS_WIFI_ONLY_HINT: &str =
    "使用流量计费的网络时同样不会自动下载文件，图片不受此开关影响";
pub const SETTINGS_LOCALSEND: &str = "兼容 LocalSend";
pub const SETTINGS_LOCALSEND_HINT: &str = "开启后可与 LocalSend 应用互传文件";
pub const SETTINGS_TARGETS: &str = "单播通告目标";
pub const SETTINGS_TARGETS_HINT: &str =
    "每行一个 IP、IP:端口 或网段（如 10.8.0.0/24），用于组播到不了的网络";
pub const SETTINGS_TARGETS_PLACEHOLDER: &str = "例如 10.8.0.5";
pub const SETTINGS_AUTOSTART: &str = "开机自动启动";
pub const SETTINGS_AUTOSTART_HINT: &str =
    "登录系统后在后台运行，随时接收消息；开关立即生效，无需保存";
pub const SETTINGS_AUTOSTART_FAILED: &str = "修改开机自启失败：";
pub const SETTINGS_ABOUT_VERSION: &str = "版本";
pub const SETTINGS_ABOUT_LICENSE: &str = "许可证";
pub const SETTINGS_ABOUT_LICENSE_VALUE: &str = "GPL-3.0-or-later";
pub const SETTINGS_ABOUT_USER_FINGERPRINT: &str = "用户指纹";
pub const SETTINGS_ABOUT_DEVICE_FINGERPRINT: &str = "本机指纹";

/// 设置页地址行校验错误，`line` 为 1 起的行号。
pub fn settings_target_invalid(line: usize, text: &str) -> String {
    format!("第 {line} 行无效：{text}")
}

/// 文件列表信息行里的会话名。
pub fn files_in_conv(title: &str) -> String {
    format!("会话 {title}")
}

/// 删除确认里追加的说明：被删的消息里不止这一个文件。
pub fn files_delete_siblings(n: usize) -> String {
    format!("这条消息共有 {n} 个文件，会一并移除。")
}
