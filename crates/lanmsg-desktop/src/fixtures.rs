// 演示夹具：固定假数据，供 `--demo <页>` 与截图测试使用，不连后台服务。
// 注释中文，标识符英文。

use lanmsg_api::types::*;

use crate::model::{ConnState, Model, Page, PairUi};

/// 演示页面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoPage {
    Wizard,
    Chats,
    Chat,
    Contacts,
    Devices,
    Pair,
    Files,
    Settings,
}

impl DemoPage {
    /// 解析 `--demo` 参数值。
    pub fn parse(s: &str) -> Option<DemoPage> {
        Some(match s {
            "wizard" => DemoPage::Wizard,
            "chats" => DemoPage::Chats,
            "chat" => DemoPage::Chat,
            "contacts" => DemoPage::Contacts,
            "devices" => DemoPage::Devices,
            "pair" => DemoPage::Pair,
            "files" => DemoPage::Files,
            "settings" => DemoPage::Settings,
            _ => return None,
        })
    }
}

const NOW: i64 = 1_790_000_000_000;
const MIN: i64 = 60_000;

fn conv(
    id: &str,
    kind: ConvKind,
    title: &str,
    text: &str,
    ago: i64,
    unread: u32,
) -> ConversationSummary {
    ConversationSummary {
        conv: id.into(),
        kind,
        title: title.into(),
        last_text: Some(text.into()),
        last_hlc_millis: Some(NOW - ago * MIN),
        unread,
        peer_online: true,
    }
}

fn file_item(name: &str, mime: &str, size: u64, state: BlobState) -> FileItemView {
    let image = mime.starts_with("image/");
    let video = mime.starts_with("video/");
    FileItemView {
        hash: format!("hash-{name}"),
        name: name.into(),
        size,
        mime: mime.into(),
        width: (image || video).then_some(1920),
        height: (image || video).then_some(1080),
        duration_ms: video.then_some(83_000),
        thumb_path: None,
        local_path: (state == BlobState::Local).then(|| format!("/tmp/{name}")),
        state,
        source_label: "小王的笔记本".into(),
    }
}

fn message(i: usize, mine: bool, body: MessageBodyView) -> MessageView {
    MessageView {
        id: format!("m{i}"),
        conv: "c-dm".into(),
        author_user: if mine { "me" } else { "peer" }.into(),
        author_device: if mine { "dev-me" } else { "dev-peer" }.into(),
        author_device_name: if mine {
            "我的电脑"
        } else {
            "小王的笔记本"
        }
        .into(),
        is_mine: mine,
        hlc_millis: NOW - (30 - i as i64) * MIN,
        body,
        state: if mine {
            MessageState::Delivered
        } else {
            MessageState::Read
        },
    }
}

fn text(i: usize, mine: bool, t: &str) -> MessageView {
    message(i, mine, MessageBodyView::Text { text: t.into() })
}

fn files(i: usize, mine: bool, items: Vec<FileItemView>) -> MessageView {
    message(i, mine, MessageBodyView::Files { items })
}

/// 20 条混合消息：文本、图片、视频与三种文件状态。
fn demo_messages() -> Vec<MessageView> {
    let mut v = vec![
        text(0, false, "晚上好，下周的方案文档发你了"),
        text(1, true, "收到，我看一下"),
        files(
            2,
            false,
            vec![file_item(
                "方案-v3.pdf",
                "application/pdf",
                2_400_000,
                BlobState::Local,
            )],
        ),
        text(3, true, "第三页的预算表格有点问题"),
        text(4, false, "哪里？我改一下"),
        files(
            5,
            false,
            vec![file_item(
                "现场照片.jpg",
                "image/jpeg",
                3_200_000,
                BlobState::Local,
            )],
        ),
        text(6, true, "这张拍得很清楚"),
        files(
            7,
            false,
            vec![file_item(
                "演示录屏.mp4",
                "video/mp4",
                48_000_000,
                BlobState::Remote,
            )],
        ),
        text(8, false, "录屏比较大，需要的话点下载"),
        files(
            9,
            true,
            vec![file_item(
                "会议纪要.docx",
                "application/vnd.openxmlformats",
                120_000,
                BlobState::Local,
            )],
        ),
        files(
            10,
            false,
            vec![file_item(
                "设计稿.zip",
                "application/zip",
                96_000_000,
                BlobState::Downloading,
            )],
        ),
        text(11, true, "下载中，稍等"),
        files(
            12,
            false,
            vec![file_item(
                "旧版本.rar",
                "application/x-rar",
                12_000_000,
                BlobState::SourceOffline,
            )],
        ),
        text(13, false, "我这边先下线了，明天再传"),
        text(14, true, "好的，不急"),
    ];
    v.push(files(
        15,
        true,
        vec![
            file_item("截图1.png", "image/png", 400_000, BlobState::Local),
            file_item("截图2.png", "image/png", 380_000, BlobState::Local),
            file_item("截图3.png", "image/png", 410_000, BlobState::Local),
        ],
    ));
    v.push(message(
        16,
        false,
        MessageBodyView::Folder {
            name: "项目资料".into(),
            items: vec![file_item(
                "readme.txt",
                "text/plain",
                2_000,
                BlobState::Remote,
            )],
        },
    ));
    let mut recalled = text(17, false, "");
    recalled.state = MessageState::Recalled;
    v.push(recalled);
    v.push(text(18, true, "明天见"));
    let mut failed = text(19, true, "最后一条发送失败的消息");
    failed.state = MessageState::Failed;
    v.push(failed);
    v
}

fn device(id: &str, name: &str, ty: &str, online: bool, is_self: bool) -> DeviceView {
    DeviceView {
        device_id: format!("dev-{id}"),
        device_short: id.into(),
        name: name.into(),
        device_type: ty.into(),
        online,
        path: online.then_some(PathClassDto::Lan),
        is_self,
    }
}

fn demo_files() -> Vec<FileView> {
    (0..15)
        .map(|i| {
            let (name, mime) = match i % 5 {
                0 => (format!("照片{i}.jpg"), "image/jpeg"),
                1 => (format!("文档{i}.pdf"), "application/pdf"),
                2 => (format!("录像{i}.mp4"), "video/mp4"),
                3 => (format!("压缩包{i}.zip"), "application/zip"),
                _ => (format!("笔记{i}.txt"), "text/plain"),
            };
            FileView {
                item: file_item(&name, mime, 100_000 * (i as u64 + 1), BlobState::Local),
                conv: "c-dm".into(),
                conv_title: "小王".into(),
                msg: format!("m{i}"),
                hlc_millis: NOW - i as i64 * 60 * MIN,
            }
        })
        .collect()
}

/// 构造某个演示页面的 Model。
pub fn demo_model(p: DemoPage) -> Model {
    let mut m = Model::new();
    m.conn = ConnState::Connected;
    m.me = Some(SelfInfo {
        initialized: p != DemoPage::Wizard,
        user_id: Some("user-me".into()),
        user_short: Some("A1B2".into()),
        display_name: Some("小李".into()),
        device_id: "dev-me".into(),
        device_name: Some("我的电脑".into()),
        status: StatusDto::Online,
    });
    m.convs = vec![
        conv("c-dm", ConvKind::Dm, "小王", "明天见", 2, 2),
        conv(
            "c-self",
            ConvKind::SelfDevices,
            "我的设备",
            "手机: 备忘已同步",
            45,
            0,
        ),
        conv(
            "c-guest",
            ConvKind::Guest,
            "网页访客 192.168.1.8",
            "[图片]",
            300,
            1,
        ),
    ];
    m.contacts = vec![
        ContactView {
            user_id: "u-wang".into(),
            user_short: "C3D4".into(),
            display_name: "小王".into(),
            online: true,
            devices: vec![device("w1", "小王的笔记本", "laptop", true, false)],
            auto_accept: true,
            ask_above_bytes: Some(100_000_000),
        },
        ContactView {
            user_id: "u-zhao".into(),
            user_short: "E5F6".into(),
            display_name: "赵经理".into(),
            online: false,
            devices: vec![device("z1", "赵的手机", "phone", false, false)],
            auto_accept: false,
            ask_above_bytes: None,
        },
    ];
    m.requests = vec![RequestView {
        request: "r1".into(),
        kind: RequestKind::Contact,
        from_user: "u-chen".into(),
        from_name: "陈同学".into(),
        from_short: "G7H8".into(),
        preview: "你好，我是隔壁组的".into(),
        created_at: NOW - 5 * MIN,
    }];
    m.devices = vec![
        device("a1", "我的电脑", "desktop", true, true),
        device("a2", "我的手机", "phone", true, false),
    ];
    m.files = demo_files();
    m.settings = Some(SettingsDto {
        device_name: "我的电脑".into(),
        download_dir: "/home/demo/Downloads/LanMsg".into(),
        auto_image_max: 10_000_000,
        auto_file_max: 50_000_000,
        auto_file_wifi_only: false,
        localsend_enabled: true,
        unicast_targets: vec!["192.168.1.20".into()],
        background_mode: BackgroundMode::Realtime,
        localsend_listening: Some(true),
    });
    match p {
        DemoPage::Wizard => {
            m.page = Page::Wizard;
        }
        DemoPage::Chats => {
            m.page = Page::Chats;
        }
        DemoPage::Chat => {
            m.page = Page::Chats;
            m.current_conv = Some("c-dm".into());
            m.messages = demo_messages();
        }
        DemoPage::Contacts | DemoPage::Devices => {
            m.page = Page::Contacts;
        }
        DemoPage::Pair => {
            m.page = Page::Contacts;
            m.pair = Some(PairUi::Showing {
                session: "s1".into(),
                qr_uri: Some("lanmsg://pair?s=demo".into()),
                state: PairStateDto::WaitingPeer,
            });
        }
        DemoPage::Files => {
            m.page = Page::Files;
        }
        DemoPage::Settings => {
            m.page = Page::Settings;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_chat_has_twenty_mixed_messages() {
        let m = demo_model(DemoPage::Chat);
        assert_eq!(m.messages.len(), 20);
        assert_eq!(m.convs.len(), 3);
        assert_eq!(m.files.len(), 15);
        assert_eq!(m.contacts.len(), 2);
        assert_eq!(m.requests.len(), 1);
        assert_eq!(m.devices.len(), 2);
    }

    #[test]
    fn demo_names_round_trip() {
        for n in [
            "wizard", "chats", "chat", "contacts", "devices", "pair", "files", "settings",
        ] {
            assert!(DemoPage::parse(n).is_some(), "{n}");
        }
        assert!(DemoPage::parse("nope").is_none());
    }
}
