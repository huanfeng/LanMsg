// 聊天主区：顶栏、消息列表（跟随底部 / 头部锚定 / 到顶加载更早）、底部输入区占位。
// 行数据由纯函数 `build_rows` 生成（时间分隔、作者间距），带单测。注释中文，标识符英文。

use lanmsg_api::types::*;
use windui::prelude::*;

use crate::event_text::conv_title;
use crate::model::{Model, Page};
use crate::strings;
use crate::style;
use crate::tz;
use crate::ui_state::UiState;
use crate::views::chats::{format_time, now_millis};
use crate::views::network::path_badge_text;
use crate::views::{composer, forward, message_row};

/// 超过此间隔插入时间分隔行（规范第 8 节）。
const SEPARATOR_GAP_MS: i64 = 5 * 60_000;
/// 顶栏高度。
const HEADER_H: i32 = 56;

/// 消息列表的一行：时间分隔或一条消息。
#[derive(Clone, PartialEq, Debug)]
pub enum ChatRow {
    Sep {
        key: String,
        label: String,
    },
    Msg {
        m: MessageView,
        show_author: bool,
        /// 与上一行的间距：同一作者连续 4，换作者 12，首行 0。
        gap: i32,
    },
    /// 列表尾部固定留白：stick_to_bottom 贴底时不计容器 padding，用占位元素保证与输入区的间距一致。
    Tail,
}

impl ChatRow {
    pub fn key(&self) -> String {
        match self {
            ChatRow::Sep { key, .. } => key.clone(),
            ChatRow::Msg { m, .. } => format!("m:{}", m.id),
            ChatRow::Tail => "tail".to_string(),
        }
    }
}

fn hhmm(millis: i64, tz_offset_secs: i32) -> String {
    let sod = (millis.div_euclid(1000) + tz_offset_secs as i64).rem_euclid(86_400);
    format!("{:02}:{:02}", sod / 3600, sod % 3600 / 60)
}

/// 时间分隔文字：今天 `HH:MM`、昨天 `昨天 HH:MM`、更早 `MM-DD HH:MM`。
pub fn separator_label(millis: i64, now_millis: i64, tz_offset_secs: i32) -> String {
    let day = format_time(millis, now_millis, tz_offset_secs);
    let t = hhmm(millis, tz_offset_secs);
    if day == t {
        t
    } else {
        format!("{day} {t}")
    }
}

/// 把消息窗口展开成列表行。`self_devices`：“我的设备”会话，每条都标来源设备名。
pub fn build_rows(
    messages: &[MessageView],
    self_devices: bool,
    now_millis: i64,
    tz_offset_secs: i32,
) -> Vec<ChatRow> {
    let mut rows = Vec::with_capacity(messages.len() + 4);
    let mut prev: Option<&MessageView> = None;
    for m in messages {
        let new_block = match prev {
            None => true,
            Some(p) => m.hlc_millis - p.hlc_millis > SEPARATOR_GAP_MS,
        };
        if new_block {
            rows.push(ChatRow::Sep {
                key: format!("s:{}", m.id),
                label: separator_label(m.hlc_millis, now_millis, tz_offset_secs),
            });
        }
        let gap = match prev {
            _ if new_block => 0,
            Some(p) if p.is_mine == m.is_mine && p.author_device == m.author_device => {
                style::SPACE_XS
            }
            _ => style::SPACE_MD,
        };
        // 缩略图与文字气泡等不同类型相邻：至少隔 8
        let gap = match prev {
            Some(p)
                if gap > 0
                    && message_row::is_thumb_message(p) != message_row::is_thumb_message(m) =>
            {
                gap.max(style::SPACE_SM)
            }
            _ => gap,
        };
        rows.push(ChatRow::Msg {
            m: m.clone(),
            show_author: self_devices,
            gap,
        });
        prev = Some(m);
    }
    rows
}

/// 顶栏展示数据。
#[derive(Clone, PartialEq, Debug)]
pub struct HeaderInfo {
    pub title: String,
    pub online: bool,
    /// 在线对端的设备名。
    pub devices: Vec<String>,
    /// 非局域网路径的角标文字。
    pub badge: Option<&'static str>,
}

fn path_rank(p: &PathClassDto) -> u8 {
    match p {
        PathClassDto::Lan => 0,
        PathClassDto::Vpn => 1,
        PathClassDto::Internet => 2,
        PathClassDto::Relay => 3,
    }
}

/// 当前会话的顶栏数据；没有选中会话时为 None。
/// 私聊按显示名匹配对端设备，“我的设备”取自己的其它设备。
pub fn header_info(m: &Model) -> Option<HeaderInfo> {
    let conv = m
        .current_conv
        .as_deref()
        .and_then(|id| m.convs.iter().find(|c| c.conv == id))?;
    let peers: Vec<&PeerView> = match conv.kind {
        ConvKind::SelfDevices => m.peers.iter().filter(|p| p.is_mine).collect(),
        ConvKind::Dm => m
            .peers
            .iter()
            .filter(|p| !p.is_mine && p.display_name == conv.title)
            .collect(),
        _ => Vec::new(),
    };
    let badge = if conv.peer_online {
        peers
            .iter()
            .map(|p| &p.path)
            .min_by_key(|p| path_rank(p))
            .and_then(path_badge_text)
    } else {
        None
    };
    Some(HeaderInfo {
        title: conv_title(conv),
        online: conv.peer_online,
        devices: if conv.peer_online {
            peers.iter().map(|p| p.device_name.clone()).collect()
        } else {
            Vec::new()
        },
        badge,
    })
}

fn header(info: Signal<Option<HeaderInfo>>) -> Element {
    let title = info.map(|i| i.as_ref().map(|i| i.title.clone()).unwrap_or_default());
    let subtitle = info.map(|i| match i {
        Some(i) if i.online && !i.devices.is_empty() => i.devices.join(strings::LIST_SEPARATOR),
        Some(i) if i.online => strings::CHAT_ONLINE.to_string(),
        _ => strings::CHAT_OFFLINE.to_string(),
    });
    let online = info.map(|i| i.as_ref().is_some_and(|i| i.online));
    let offline = online.map(|o| !*o);
    let badge_text = info.map(|i| {
        i.as_ref()
            .and_then(|i| i.badge)
            .unwrap_or_default()
            .to_string()
    });
    let has_badge = badge_text.map(|t| !t.is_empty());
    let dot = |on: Signal<bool>, role: Role| {
        Element::leaf()
            .size(8, 8)
            .corner(4.0)
            .bg_role(role)
            .visible_signal(on)
    };
    let sub_row = Element::row()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(dot(online, Role::Success))
        .child(dot(offline, Role::TextSubtle))
        .child(
            Element::label(subtitle)
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted)
                .max_lines(1)
                .truncate(Truncate::End),
        );
    let badge = Element::stack()
        .height(20)
        .padding_xy(style::SPACE_SM, 0)
        .corner(style::RADIUS_PILL)
        .bg_when(|| style::tokens().signal_soft)
        .visible_signal(has_badge)
        .child(
            Element::label(badge_text)
                .font_size(style::FONT_META)
                .fg_role(Role::AccentText)
                .align(Align::Center),
        );
    Element::row()
        .width_match()
        .height(HEADER_H)
        .padding_xy(style::SPACE_LG, 0)
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .bg_role(Role::Surface)
        .child(
            Element::col()
                .weight(1.0)
                .spacing(2)
                .child(
                    Element::label(title)
                        .font_size(style::FONT_HEADING)
                        .font_weight(style::WEIGHT_STRONG)
                        .fg_role(Role::Text)
                        .max_lines(1)
                        .truncate(Truncate::End),
                )
                .child(sub_row),
        )
        .child(badge)
}

fn separator_row(label: String) -> Element {
    Element::stack()
        .width_match()
        .padding_xy(0, style::SPACE_SM)
        .margin_edges(Insets {
            left: 0,
            top: style::SPACE_MD,
            right: 0,
            bottom: 0,
        })
        .child(
            Element::label(label)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle)
                .align(Align::Center),
        )
}

fn list_row(ui: &UiState, row: ChatRow) -> Element {
    match row {
        ChatRow::Sep { label, .. } => separator_row(label),
        ChatRow::Tail => Element::leaf().width_match().height(style::SPACE_MD),
        ChatRow::Msg {
            m,
            show_author,
            gap,
        } => Element::col()
            .width_match()
            .padding_xy(style::SPACE_LG, 0)
            .margin_edges(Insets {
                left: 0,
                top: gap,
                right: 0,
                bottom: 0,
            })
            .child(message_row::message_row(ui, &m, show_author)),
    }
}

fn message_list(ui: &UiState) -> Element {
    let rows = ui.derive(|m| {
        let self_devices = m
            .current_conv
            .as_deref()
            .and_then(|id| m.convs.iter().find(|c| c.conv == id))
            .is_some_and(|c| c.kind == ConvKind::SelfDevices);
        let mut rows = build_rows(
            &m.messages,
            self_devices,
            now_millis(),
            tz::local_offset_secs(),
        );
        rows.push(ChatRow::Tail);
        rows
    });
    let ui_row = ui.clone();
    let ui_edge = ui.clone();
    Element::list_signal(rows, |r: &ChatRow| r.key(), move |r| list_row(&ui_row, r))
        .stick_to_bottom(true)
        .preserve_on_prepend(true)
        // 到顶且还有更早消息时加载；Model 内部防重复请求。
        // 初始页是满页（50 条）时内容必然超出一屏，用户滚到顶才会触发
        .on_scroll_edge(move |_ctx, edge| {
            match edge {
                ScrollEdge::Top => ui_edge.act(|m| m.load_older()),
                // 窗口缺最新消息时，滚到底部重新拉取最新一页
                ScrollEdge::Bottom => ui_edge.act(|m| m.reload_latest()),
            }
        })
        // 首尾留白，最后一条不贴着输入区
        .padding_xy(0, style::SPACE_MD)
        .weight(1.0)
        .width_match()
}

/// 窗口缺最新消息时的提示条：点击重新拉取最新一页并回到底部。
/// 不依赖滚动边缘回调，覆盖“已在底部、不会再触发 Bottom”的情形。
fn new_messages_banner(ui: &UiState) -> Element {
    let has_newer = ui.derive(|m| m.has_newer);
    let ui = ui.clone();
    Element::stack()
        .width_match()
        .padding_xy(0, style::SPACE_SM)
        .visible_signal(has_newer)
        .child(
            Element::stack()
                .height(28)
                .padding_xy(style::SPACE_MD, 0)
                .corner(style::RADIUS_PILL)
                .bg_role(Role::Surface)
                .border_role(Role::Border, 1)
                .align(Align::Center)
                .clickable()
                .on_click(move |_| {
                    ui.act(|m| m.reload_latest());
                    ui.jump_to_bottom();
                })
                .child(
                    Element::label(strings::NEW_MESSAGES_BANNER)
                        .font_size(style::FONT_META)
                        .fg_role(Role::AccentText)
                        .align(Align::Center),
                ),
        )
}

fn conversation(ui: &UiState, conv: String) -> Element {
    let info = ui.derive(header_info);
    let ui_drop = ui.clone();
    // 列表按“回到底部”序号重建，首次布局即滚到底
    let ui_list = ui.clone();
    let list = Element::switcher(ui.bottom_epoch, move |_| message_list(&ui_list)).weight(1.0);
    Element::stack()
        .fill()
        .bg_role(Role::Bg)
        .child(
            Element::col()
                .fill()
                .child(header(info))
                .child(Element::divider())
                .child(list)
                .child(new_messages_banner(ui))
                .child(composer::composer(ui, conv)),
        )
        .child(forward::forward_host(ui))
        .child(composer::paste_host(ui))
        // 整个聊天区接收拖入的文件
        .on_drop_files(move |_ctx, paths| composer::on_drop(&ui_drop, paths))
}

/// 右侧聊天区：未选会话显示空状态，选中后按会话重建（滚动位置自然回到底部）。
pub fn chat_pane(ui: &UiState) -> Element {
    let current = ui.derive(|m| m.current_conv.clone());
    let ui = ui.clone();
    Element::switcher(current, move |c| match c {
        None => {
            let ui_go = ui.clone();
            let go = Element::link(strings::CHAT_PANE_EMPTY_ACTION)
                .on_click(move |_| ui_go.act(|m| m.set_page(Page::Contacts)));
            style::empty_state(
                strings::CHAT_PANE_EMPTY,
                strings::CHAT_PANE_EMPTY_HINT,
                Some(go),
            )
        }
        Some(conv) => conversation(&ui, conv),
    })
    .fill()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn msg(id: &str, mine: bool, device: &str, t: i64) -> MessageView {
        MessageView {
            id: id.into(),
            conv: "c".into(),
            author_user: "u".into(),
            author_device: device.into(),
            author_device_name: device.into(),
            is_mine: mine,
            hlc_millis: t,
            body: MessageBodyView::Text { text: "x".into() },
            state: MessageState::Sent,
        }
    }

    fn gaps(rows: &[ChatRow]) -> Vec<i32> {
        rows.iter()
            .filter_map(|r| match r {
                ChatRow::Msg { gap, .. } => Some(*gap),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn separator_inserted_after_five_minutes_and_at_start() {
        let ms = [
            msg("a", false, "d1", 0),
            msg("b", false, "d1", 5 * MIN),
            msg("c", false, "d1", 10 * MIN + 1),
        ];
        let rows = build_rows(&ms, false, 10 * MIN, 0);
        let kinds: Vec<_> = rows
            .iter()
            .map(|r| matches!(r, ChatRow::Sep { .. }))
            .collect();
        // 开头一个分隔；a-b 恰 5 分钟不插；b-c 超过 5 分钟插
        assert_eq!(kinds, [true, false, false, true, false]);
    }

    #[test]
    fn gap_is_4_for_same_author_and_12_for_new_author() {
        let ms = [
            msg("a", false, "d1", 0),
            msg("b", false, "d1", MIN),
            msg("c", true, "d2", 2 * MIN),
            msg("d", true, "d3", 3 * MIN),
        ];
        let rows = build_rows(&ms, false, 3 * MIN, 0);
        assert_eq!(gaps(&rows), [0, 4, 12, 12]);
    }

    #[test]
    fn gap_resets_after_separator() {
        let ms = [msg("a", false, "d1", 0), msg("b", false, "d1", 20 * MIN)];
        assert_eq!(gaps(&build_rows(&ms, false, 20 * MIN, 0)), [0, 0]);
    }

    #[test]
    fn self_devices_marks_every_message_with_author() {
        let ms = [msg("a", true, "d1", 0), msg("b", true, "d1", MIN)];
        let rows = build_rows(&ms, true, MIN, 0);
        assert!(rows.iter().all(|r| match r {
            ChatRow::Msg { show_author, .. } => *show_author,
            _ => true,
        }));
        let rows = build_rows(&ms, false, MIN, 0);
        assert!(rows.iter().all(|r| match r {
            ChatRow::Msg { show_author, .. } => !*show_author,
            _ => true,
        }));
    }

    #[test]
    fn row_keys_are_unique_and_stable() {
        let ms = [msg("a", false, "d", 0), msg("b", false, "d", 20 * MIN)];
        let keys: Vec<_> = build_rows(&ms, false, 0, 0)
            .iter()
            .map(|r| r.key())
            .collect();
        assert_eq!(keys, ["s:a", "m:a", "s:b", "m:b"]);
    }

    #[test]
    fn separator_label_forms() {
        // 2026-03-15 09:05:00 UTC
        let now = 1_773_565_500_000;
        assert_eq!(separator_label(now - 60_000, now, 0), "09:04");
        assert_eq!(separator_label(now - 86_400_000, now, 0), "昨天 09:05");
        assert_eq!(separator_label(now - 2 * 86_400_000, now, 0), "03-13 09:05");
    }

    fn peer(device: &str, name: &str, mine: bool, path: PathClassDto) -> PeerView {
        PeerView {
            device_id: device.into(),
            device_short: "ab".into(),
            user_id: "u".into(),
            display_name: name.into(),
            device_name: format!("{device}-dev"),
            device_type: "laptop".into(),
            addr: "1.2.3.4:5".into(),
            path,
            is_contact: true,
            is_mine: mine,
        }
    }

    fn model_with(kind: ConvKind, online: bool, peers: Vec<PeerView>) -> Model {
        let mut m = Model::new();
        m.convs = vec![ConversationSummary {
            conv: "c".into(),
            kind,
            title: "小王".into(),
            last_text: None,
            last_kind: None,
            last_hlc_millis: None,
            unread: 0,
            peer_online: online,
        }];
        m.current_conv = Some("c".into());
        m.peers = peers;
        m
    }

    #[test]
    fn header_matches_dm_peers_and_picks_best_path_badge() {
        let m = model_with(
            ConvKind::Dm,
            true,
            vec![
                peer("d1", "小王", false, PathClassDto::Relay),
                peer("d2", "小王", false, PathClassDto::Vpn),
                peer("d3", "别人", false, PathClassDto::Lan),
            ],
        );
        let h = header_info(&m).expect("有当前会话");
        assert_eq!(h.devices, ["d1-dev", "d2-dev"]);
        assert_eq!(h.badge, Some("VPN"));
    }

    #[test]
    fn header_lan_has_no_badge_and_offline_hides_devices() {
        let m = model_with(
            ConvKind::Dm,
            true,
            vec![peer("d1", "小王", false, PathClassDto::Lan)],
        );
        assert_eq!(header_info(&m).and_then(|h| h.badge), None);
        let m = model_with(
            ConvKind::Dm,
            false,
            vec![peer("d1", "小王", false, PathClassDto::Relay)],
        );
        let h = header_info(&m).expect("有当前会话");
        assert!(!h.online && h.devices.is_empty() && h.badge.is_none());
    }

    #[test]
    fn header_self_devices_uses_own_peers_and_none_without_conv() {
        let m = model_with(
            ConvKind::SelfDevices,
            true,
            vec![
                peer("d1", "小李", true, PathClassDto::Lan),
                peer("d2", "小王", false, PathClassDto::Lan),
            ],
        );
        assert_eq!(
            header_info(&m).map(|h| h.devices),
            Some(vec!["d1-dev".to_string()])
        );
        assert!(header_info(&Model::new()).is_none());
    }
}
