// 通知逻辑（P3 Task 9，纯逻辑，不依赖 windui）：事件 → 是否弹通知、弹什么。
// 注释中文，标识符英文。

use std::collections::HashMap;

use crate::strings;

use lanmsg_api::{
    ConvKind, Event, FileItemView, MessageBodyView, MessageView, RequestKind, StatusDto,
};

/// 一条待弹出的系统通知。`conv` 是点击后定位用的标签（会话 id）。
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub conv: Option<String>,
}

/// 会话标题缓存：会话 id → (标题, 类型)，由 `ConversationUpdated` 事件填充；
/// 同时记录本机设备 id，用于只排除本机发出的消息（同用户其他设备的消息仍通知）。
#[derive(Debug, Default)]
pub struct TitleCache {
    titles: HashMap<String, (String, ConvKind)>,
    self_device: String,
}

impl TitleCache {
    /// 记录本机设备 id（空串表示未知，此时不排除任何消息）。
    pub fn set_self_device(&mut self, device: String) {
        self.self_device = device;
    }

    /// 观察一个事件，记录会话标题。
    pub fn observe(&mut self, ev: &Event) {
        if let Event::ConversationUpdated { summary } = ev {
            self.titles.insert(
                summary.conv.clone(),
                (summary.title.clone(), summary.kind.clone()),
            );
        }
    }

    pub fn get(&self, conv: &str) -> Option<&(String, ConvKind)> {
        self.titles.get(conv)
    }
}

/// 文字预览最多保留的字符数。
const PREVIEW_CHARS: usize = 60;

/// 文字预览：前 60 个字符，超出加"…"。
fn text_preview(text: &str) -> String {
    let mut it = text.chars();
    let head: String = it.by_ref().take(PREVIEW_CHARS).collect();
    if it.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// 单个文件的预览：图片 / 视频 / 其它。
fn file_preview(item: &FileItemView) -> String {
    if item.mime.starts_with("image/") {
        strings::NOTIFY_IMAGE.to_string()
    } else if item.mime.starts_with("video/") {
        strings::NOTIFY_VIDEO.to_string()
    } else {
        format!("{}{}", strings::NOTIFY_FILE_PREFIX, item.name)
    }
}

/// 消息体预览；`System`/`Unsupported` 不弹通知，返回 None。
fn body_preview(body: &MessageBodyView) -> Option<String> {
    match body {
        MessageBodyView::Text { text } => Some(text_preview(text)),
        MessageBodyView::Files { items } => match items.as_slice() {
            [] => None,
            [one] => Some(file_preview(one)),
            many => Some(strings::notify_files(many.len())),
        },
        MessageBodyView::Folder { name, .. } => {
            Some(format!("{}{name}", strings::NOTIFY_FOLDER_PREFIX))
        }
        MessageBodyView::System { .. } | MessageBodyView::Unsupported => None,
    }
}

fn message_notice(msg: &MessageView, focus: Option<&str>, titles: &TitleCache) -> Option<Notice> {
    let from_this_device =
        !titles.self_device.is_empty() && msg.author_device == titles.self_device;
    if from_this_device || focus == Some(msg.conv.as_str()) {
        return None;
    }
    let preview = body_preview(&msg.body)?;
    let cached = titles.get(&msg.conv);
    // 自设备会话的标题由 core 留空，这里按 kind 给出名称。
    let title = match cached {
        Some((_, ConvKind::SelfDevices)) => strings::NOTIFY_SELF_DEVICES_TITLE.to_string(),
        // 网页访客会话的标题同样由 core 留空
        Some((t, ConvKind::Guest)) if t.is_empty() => strings::NOTIFY_GUEST_TITLE.to_string(),
        Some((t, _)) => t.clone(),
        None => msg.author_device_name.clone(),
    };
    let body = match cached {
        Some((_, ConvKind::SelfDevices)) => format!("{}: {preview}", msg.author_device_name),
        _ => preview,
    };
    Some(Notice {
        title,
        body,
        conv: Some(msg.conv.clone()),
    })
}

/// 事件是否需要弹通知。调用方应先对同一事件 `titles.observe`。
/// 忙碌状态下一律不弹（仍计未读，设计 §5.5）。
pub fn notice_for(
    ev: &Event,
    focus: Option<&str>,
    my_status: &StatusDto,
    titles: &TitleCache,
) -> Option<Notice> {
    if *my_status == StatusDto::Busy {
        return None;
    }
    match ev {
        Event::MessageAdded { msg } => message_notice(msg, focus, titles),
        Event::IncomingRequest { request } => {
            let what = match request.kind {
                RequestKind::Contact => strings::NOTIFY_REQUEST_CONTACT,
                RequestKind::Message => strings::NOTIFY_REQUEST_MESSAGE,
                RequestKind::File => strings::NOTIFY_REQUEST_FILE,
            };
            Some(Notice {
                title: strings::NOTIFY_REQUEST_TITLE.to_string(),
                body: strings::notify_request_body(&request.from_name, what),
                conv: None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanmsg_api::{
        BlobState, ConversationSummary, MessageState, RequestPreviewView, RequestView,
        SystemEventView,
    };

    fn msg(conv: &str, body: MessageBodyView) -> MessageView {
        MessageView {
            id: "m1".into(),
            conv: conv.into(),
            author_user: "u".into(),
            author_device: "d".into(),
            author_device_name: "手机".into(),
            is_mine: false,
            hlc_millis: 0,
            body,
            state: MessageState::Delivered,
        }
    }

    fn text(t: &str) -> MessageBodyView {
        MessageBodyView::Text { text: t.into() }
    }

    fn file(name: &str, mime: &str) -> FileItemView {
        FileItemView {
            hash: "h".into(),
            name: name.into(),
            size: 1,
            mime: mime.into(),
            width: None,
            height: None,
            duration_ms: None,
            thumb_path: None,
            local_path: None,
            state: BlobState::Remote,
            source_name: String::new(),
            source_is_me: false,
            holder_name: None,
        }
    }

    fn added(m: MessageView) -> Event {
        Event::MessageAdded { msg: m }
    }

    fn conv_event(conv: &str, title: &str, kind: ConvKind) -> Event {
        Event::ConversationUpdated {
            summary: ConversationSummary {
                conv: conv.into(),
                kind,
                title: title.into(),
                last_text: None,
                last_kind: None,
                last_hlc_millis: None,
                unread: 1,
                peer_online: true,
            },
        }
    }

    fn request(kind: RequestKind) -> Event {
        Event::IncomingRequest {
            request: RequestView {
                request: "r".into(),
                kind,
                from_user: "u".into(),
                from_name: "张三".into(),
                from_short: "abcd".into(),
                preview: RequestPreviewView::None,
                created_at: 0,
                source_addr: None,
                addr_changed: false,
            },
        }
    }

    #[test]
    fn no_notice_for_message_from_this_device() {
        let mut titles = TitleCache::default();
        titles.set_self_device("d".into());
        let mut m = msg("c1", text("hi"));
        m.is_mine = true;
        let n = notice_for(&added(m), None, &StatusDto::Online, &titles);
        assert_eq!(n, None);
    }

    #[test]
    fn notice_for_own_user_other_device_in_self_devices() {
        let mut titles = TitleCache::default();
        titles.set_self_device("local".into());
        titles.observe(&conv_event("c1", "", ConvKind::SelfDevices));
        let mut m = msg("c1", text("hi"));
        m.is_mine = true; // 作者用户是自己，但来自另一台设备（"d"）
        let n = notice_for(&added(m), None, &StatusDto::Online, &titles).unwrap();
        assert_eq!(n.title, "我的设备");
        assert_eq!(n.body, "手机: hi");
    }

    #[test]
    fn guest_conv_with_empty_title_uses_guest_title() {
        let mut titles = TitleCache::default();
        titles.observe(&conv_event("c1", "", ConvKind::Guest));
        let ev = added(msg("c1", text("hi")));
        let n = notice_for(&ev, None, &StatusDto::Online, &titles).unwrap();
        assert_eq!(n.title, "网页访客");
        assert_eq!(n.body, "hi");
    }

    #[test]
    fn no_notice_when_busy() {
        let titles = TitleCache::default();
        let ev = added(msg("c1", text("hi")));
        assert_eq!(notice_for(&ev, None, &StatusDto::Busy, &titles), None);
        assert_eq!(
            notice_for(
                &request(RequestKind::Contact),
                None,
                &StatusDto::Busy,
                &titles
            ),
            None
        );
        assert!(notice_for(&ev, None, &StatusDto::Online, &titles).is_some());
    }

    #[test]
    fn self_devices_body_prefixed_with_device_name() {
        let mut titles = TitleCache::default();
        titles.observe(&conv_event("c1", "", ConvKind::SelfDevices));
        let ev = added(msg("c1", text("hi")));
        let n = notice_for(&ev, None, &StatusDto::Online, &titles).unwrap();
        assert_eq!(n.title, "我的设备");
        assert_eq!(n.body, "手机: hi");
        assert_eq!(n.conv.as_deref(), Some("c1"));
    }

    #[test]
    fn no_notice_when_conv_focused() {
        let ev = added(msg("c1", text("hi")));
        let n = notice_for(&ev, Some("c1"), &StatusDto::Online, &TitleCache::default());
        assert_eq!(n, None);
    }

    #[test]
    fn text_preview_truncates_at_60_chars() {
        let long = "字".repeat(100);
        let ev = added(msg("c1", text(&long)));
        let n = notice_for(&ev, None, &StatusDto::Online, &TitleCache::default()).unwrap();
        assert_eq!(n.body, format!("{}…", "字".repeat(60)));
        // 恰好 60 个字符不加省略号。
        let exact = added(msg("c1", text(&"字".repeat(60))));
        let n = notice_for(&exact, None, &StatusDto::Online, &TitleCache::default()).unwrap();
        assert_eq!(n.body, "字".repeat(60));
    }

    #[test]
    fn file_message_bodies() {
        let body_of = |b: MessageBodyView| {
            notice_for(
                &added(msg("c1", b)),
                None,
                &StatusDto::Online,
                &TitleCache::default(),
            )
            .unwrap()
            .body
        };
        let files = |items| MessageBodyView::Files { items };
        assert_eq!(body_of(files(vec![file("a.png", "image/png")])), "[图片]");
        assert_eq!(body_of(files(vec![file("a.mp4", "video/mp4")])), "[视频]");
        assert_eq!(
            body_of(files(vec![file("a.pdf", "application/pdf")])),
            "[文件] a.pdf"
        );
        let three = vec![file("a", "x/y"), file("b", "x/y"), file("c", "x/y")];
        assert_eq!(body_of(files(three)), "[3 个文件]");
        let folder = MessageBodyView::Folder {
            name: "docs".into(),
            items: vec![],
        };
        assert_eq!(body_of(folder), "[文件夹] docs");
    }

    #[test]
    fn system_and_unsupported_messages_do_not_notify() {
        let titles = TitleCache::default();
        for b in [
            MessageBodyView::System {
                event: SystemEventView::MessagesDeleted { count: 1 },
            },
            MessageBodyView::Unsupported,
        ] {
            let ev = added(msg("c1", b));
            assert_eq!(notice_for(&ev, None, &StatusDto::Online, &titles), None);
        }
    }

    #[test]
    fn incoming_contact_request_notice() {
        let n = notice_for(
            &request(RequestKind::Contact),
            None,
            &StatusDto::Online,
            &TitleCache::default(),
        )
        .unwrap();
        assert_eq!(n.title, "新的请求");
        assert_eq!(n.body, "张三 想要加你为联系人");
        assert_eq!(n.conv, None);
    }

    #[test]
    fn title_falls_back_to_device_name_without_cache() {
        let ev = added(msg("c1", text("hi")));
        let n = notice_for(&ev, None, &StatusDto::Online, &TitleCache::default()).unwrap();
        assert_eq!(n.title, "手机");
        assert_eq!(n.body, "hi");
    }
}
