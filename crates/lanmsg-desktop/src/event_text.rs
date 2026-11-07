// 结构化数据到界面文案的映射：系统消息与会话标题。
// core 只给结构（SystemEventView、ConvKind），文案只在 strings.rs。

use lanmsg_api::types::{ConvKind, ConversationSummary, SystemEventView};

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
}
