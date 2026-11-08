// lanmsgd 的界面文案（托盘与系统通知）。这里是 lanmsgd 里唯一允许出现中文字符串
// 字面量的地方（scripts/check-no-cjk.py 的白名单）；日志与命令行输出一律英文。

/// 通知里的文件预览。
pub const NOTIFY_IMAGE: &str = "[图片]";
pub const NOTIFY_VIDEO: &str = "[视频]";
pub const NOTIFY_FILE_PREFIX: &str = "[文件] ";
pub const NOTIFY_FOLDER_PREFIX: &str = "[文件夹] ";

/// 多个文件的预览：`[N 个文件]`。
pub fn notify_files(count: usize) -> String {
    format!("[{count} 个文件]")
}

/// 自设备会话的通知标题（core 给空串，这里按会话类型给出名称）。
pub const NOTIFY_SELF_DEVICES_TITLE: &str = "我的设备";

/// 新请求通知。
pub const NOTIFY_REQUEST_TITLE: &str = "新的请求";
pub const NOTIFY_REQUEST_CONTACT: &str = "加你为联系人";
pub const NOTIFY_REQUEST_MESSAGE: &str = "给你发消息";
pub const NOTIFY_REQUEST_FILE: &str = "给你发文件";

/// 请求通知正文：`{名字} 想要{动作}`。
pub fn notify_request_body(from_name: &str, what: &str) -> String {
    format!("{from_name} 想要{what}")
}

/// 托盘菜单。
pub const TRAY_OPEN: &str = "打开 LanMsg";
pub const TRAY_WEB: &str = "开启网页传输";
pub const TRAY_ONLINE: &str = "在线";
pub const TRAY_INVISIBLE: &str = "隐身";
pub const TRAY_BUSY: &str = "忙碌";
pub const TRAY_QUIT: &str = "退出";
