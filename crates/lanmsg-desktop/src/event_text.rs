// 结构化数据到界面文案的映射：系统消息与会话标题。
// core 只给结构（SystemEventView、ConvKind），文案只在 strings.rs。

use lanmsg_api::types::{
    BlobState, ConvKind, ConversationSummary, FileItemView, FileView, SystemEventView,
};

use crate::strings;

/// 系统消息的界面文案。
pub fn system_event_text(event: &SystemEventView) -> String {
    match event {
        SystemEventView::DeviceAdded { name } => format!("{}{name}", strings::SYS_DEVICE_ADDED),
        SystemEventView::DeviceRemoved { name } => {
            format!("{}{name}", strings::SYS_DEVICE_REMOVED)
        }
        SystemEventView::ContactAdded { name } => format!("{}{name}", strings::SYS_CONTACT_ADDED),
        SystemEventView::ContactUpdated { name } => {
            format!("{}{name}", strings::SYS_CONTACT_UPDATED)
        }
        SystemEventView::NicknameUpdated { name } => {
            format!("{}{name}", strings::SYS_NICKNAME_UPDATED)
        }
        SystemEventView::MessagesDeleted { count } => {
            format!(
                "{}{count}{}",
                strings::SYS_DELETED_PREFIX,
                strings::SYS_DELETED_SUFFIX
            )
        }
        SystemEventView::Recalled { by_me: true, .. } => strings::SYS_RECALLED_BY_ME.to_string(),
        SystemEventView::Recalled {
            author_name,
            by_me: false,
        } => {
            format!("{author_name}{}", strings::SYS_RECALLED_BY_PEER)
        }
    }
}

/// 会话标题：自设备会话由 core 给空串，这里按 kind 显示本地化名称。
pub fn conv_title(c: &ConversationSummary) -> String {
    match c.kind {
        ConvKind::SelfDevices => strings::SELF_DEVICES_TITLE.to_string(),
        _ => c.title.clone(),
    }
}

/// 文件来源的“来自谁”：自己发出的显示“来自我”，其余“来自 {名字}”。
pub fn source_from(item: &FileItemView) -> String {
    if item.source_is_me {
        strings::SOURCE_FROM_ME.to_string()
    } else {
        format!("{}{}", strings::SOURCE_FROM_PREFIX, item.source_name)
    }
}

/// 文件来源的状态：本机已有 → “已下载”；否则 “存于 {持有者}”；两者都没有则空。
pub fn source_status(item: &FileItemView) -> String {
    if item.state == BlobState::Local {
        strings::SOURCE_DOWNLOADED.to_string()
    } else {
        match &item.holder_name {
            Some(h) => format!("{}{h}", strings::SOURCE_STORED_PREFIX),
            None => String::new(),
        }
    }
}

/// 聊天里文件卡片的单行来源：“来自 X · 已下载”。
pub fn source_line(item: &FileItemView) -> String {
    let status = source_status(item);
    if status.is_empty() {
        source_from(item)
    } else {
        format!("{} · {status}", source_from(item))
    }
}

/// 文件视图所属会话的标题：自设备会话按 kind 本地化。
pub fn file_conv_title(f: &FileView) -> String {
    match f.conv_kind {
        ConvKind::SelfDevices => strings::SELF_DEVICES_TITLE.to_string(),
        _ => f.conv_title.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_system_event_renders_non_empty_text_with_name() {
        let n = "小李".to_string();
        let cases = [
            (
                SystemEventView::DeviceAdded { name: n.clone() },
                "已添加设备：小李",
            ),
            (
                SystemEventView::DeviceRemoved { name: n.clone() },
                "已移除设备：小李",
            ),
            (
                SystemEventView::ContactAdded { name: n.clone() },
                "已添加联系人：小李",
            ),
            (
                SystemEventView::ContactUpdated { name: n.clone() },
                "联系人已更新：小李",
            ),
            (
                SystemEventView::NicknameUpdated { name: n.clone() },
                "昵称已更新：小李",
            ),
            (
                SystemEventView::MessagesDeleted { count: 3 },
                "已删除 3 条消息",
            ),
            (
                SystemEventView::Recalled {
                    author_name: n.clone(),
                    by_me: false,
                },
                "小李 撤回了一条消息",
            ),
            (
                SystemEventView::Recalled {
                    author_name: n,
                    by_me: true,
                },
                "你撤回了一条消息",
            ),
        ];
        for (event, expect) in cases {
            assert_eq!(system_event_text(&event), expect, "{event:?}");
        }
    }

    fn summary(kind: ConvKind, title: &str) -> ConversationSummary {
        ConversationSummary {
            conv: "c".into(),
            kind,
            title: title.into(),
            last_text: None,
            last_kind: None,
            last_hlc_millis: None,
            unread: 0,
            peer_online: false,
        }
    }

    #[test]
    fn self_devices_title_is_localized_by_kind() {
        assert_eq!(conv_title(&summary(ConvKind::SelfDevices, "")), "我的设备");
        assert_eq!(conv_title(&summary(ConvKind::Dm, "小王")), "小王");
    }

    fn item(state: BlobState, is_me: bool, holder: Option<&str>) -> FileItemView {
        FileItemView {
            hash: "h".into(),
            name: "a.bin".into(),
            size: 1,
            mime: "application/octet-stream".into(),
            width: None,
            height: None,
            duration_ms: None,
            thumb_path: None,
            local_path: None,
            state,
            source_name: "张三".into(),
            source_is_me: is_me,
            holder_name: holder.map(String::from),
        }
    }

    #[test]
    fn file_source_texts_by_fields() {
        let downloaded = item(BlobState::Local, true, None);
        assert_eq!(source_line(&downloaded), "来自我 · 已下载");
        let remote = item(BlobState::Remote, false, Some("张三的笔记本"));
        assert_eq!(source_line(&remote), "来自 张三 · 存于 张三的笔记本");
        assert_eq!(source_from(&remote), "来自 张三");
        assert_eq!(source_status(&remote), "存于 张三的笔记本");
        let nowhere = item(BlobState::SourceOffline, false, None);
        assert_eq!(source_line(&nowhere), "来自 张三");
    }
}
