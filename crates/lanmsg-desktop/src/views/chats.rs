// 会话列表与搜索。注释中文，标识符英文。
// 视图只经 UiState 读写；行数据先压成可比较的 Row，再交给 list_signal 做键控复用。

use lanmsg_api::types::{ConversationSummary, LastMessageKind, MessageBodyView, MessageView};
use windui::prelude::*;

use crate::event_text::conv_title;
use crate::strings;
use crate::style;
use crate::tz;
use crate::ui_state::UiState;
use crate::views::chat;

const SECS_PER_DAY: i64 = 86_400;

/// 从 1970-01-01 起的天数换算公历 (月, 日)。
pub(crate) fn civil_month_day(days: i64) -> (u32, u32) {
    // Howard Hinnant 的 civil_from_days
    let z = days + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (m, d)
}

/// 会话时间：今天 `HH:MM`、昨天“昨天”、更早 `MM-DD`（跨年也不带年份）。
pub fn format_time(millis: i64, now_millis: i64, tz_offset_secs: i32) -> String {
    let local = |ms: i64| ms.div_euclid(1000) + tz_offset_secs as i64;
    let day = local(millis).div_euclid(SECS_PER_DAY);
    let today = local(now_millis).div_euclid(SECS_PER_DAY);
    match today - day {
        d if d <= 0 => {
            let sod = local(millis).rem_euclid(SECS_PER_DAY);
            format!("{:02}:{:02}", sod / 3600, sod % 3600 / 60)
        }
        1 => strings::YESTERDAY.to_string(),
        _ => {
            let (m, d) = civil_month_day(day);
            format!("{m:02}-{d:02}")
        }
    }
}

/// 未读角标文字：0 不显示，超过 99 显示 `99+`。
pub fn unread_badge(n: u32) -> Option<String> {
    match n {
        0 => None,
        1..=99 => Some(n.to_string()),
        _ => Some("99+".to_string()),
    }
}

/// 头像里显示的首字素；空串返回 `?`。
///
/// 不引入字素库：把 ZWJ 连接的后续字符、变体选择符、肤色修饰符并入前一个字符。
pub fn avatar_initial(name: &str) -> String {
    const ZWJ: char = '\u{200d}';
    let is_modifier = |c: char| c == '\u{fe0f}' || ('\u{1f3fb}'..='\u{1f3ff}').contains(&c);
    let mut it = name.trim().chars().peekable();
    let Some(first) = it.next() else {
        return "?".to_string();
    };
    let mut out = String::from(first);
    while let Some(&c) = it.peek() {
        if c == ZWJ {
            out.push(c);
            it.next();
            if let Some(next) = it.next() {
                out.push(next);
            }
        } else if is_modifier(c) {
            out.push(c);
            it.next();
        } else {
            break;
        }
    }
    out
}

/// 搜索结果的文本片段：以首个命中（不区分大小写）为中心，前后各取至多 `radius` 个字符，
/// 被截断的一侧加“…”；未命中返回前 `2*radius` 个字符。
pub fn snippet(text: &str, query: &str, radius: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let q: Vec<char> = query.chars().collect();
    let eq = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    let hit = if q.is_empty() || q.len() > chars.len() {
        None
    } else {
        (0..=chars.len() - q.len())
            .find(|&i| q.iter().enumerate().all(|(j, &qc)| eq(chars[i + j], qc)))
    };
    let (start, end) = match hit {
        Some(i) => (
            i.saturating_sub(radius),
            (i + q.len() + radius).min(chars.len()),
        ),
        None => (0, (2 * radius).min(chars.len())),
    };
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 头像（在线点的描边环取列表底色 `surface`）。
pub(crate) fn avatar(id: &str, name: &str, online: bool) -> Element {
    avatar_in_row(id, name, online, false)
}

/// 头像；`selected` 为真时在线点的外圈取 `signal_soft`，与选中行底色无缝衔接。
fn avatar_in_row(id: &str, name: &str, online: bool, selected: bool) -> Element {
    let circle = Element::stack()
        .size(style::AVATAR_SIZE, style::AVATAR_SIZE)
        .corner(style::AVATAR_SIZE as f32 / 2.0)
        .bg(style::avatar_color(id))
        .child(
            Element::label(avatar_initial(name))
                .font_size(style::AVATAR_FONT)
                .fg_role(Role::OnAccent)
                .align(Align::Center),
        );
    // 在线小圆点叠在头像右下角：外圈 14 用行底色留出缝（不用描边，描边在选中行上会露出非底色的白缝），内圆 10 居中
    let dot = Element::stack()
        .size(style::ONLINE_RING, style::ONLINE_RING)
        .corner(style::ONLINE_RING as f32 / 2.0)
        .bg_when(move || {
            if selected {
                style::tokens().signal_soft
            } else {
                style::tokens().surface
            }
        })
        .child(
            Element::leaf()
                .size(style::ONLINE_DOT, style::ONLINE_DOT)
                .corner(style::ONLINE_DOT as f32 / 2.0)
                .bg_role(Role::Success)
                .align(Align::Center),
        )
        .align_xy(Align::End, Align::End)
        .visible(online);
    Element::stack()
        .size(style::AVATAR_SIZE, style::AVATAR_SIZE)
        .child(circle)
        .child(dot)
}

/// 会话列表一行的展示数据（可比较，供 list_signal 判定行是否需要重建）。
#[derive(Clone, PartialEq)]
struct ConvRow {
    conv: String,
    title: String,
    preview: String,
    time: String,
    unread: Option<String>,
    online: bool,
    selected: bool,
}

/// 会话行第二行预览：文本取 `last_text`，其余按 `last_kind` 拼文案（core 不生成展示文本）。
fn conv_preview(c: &ConversationSummary) -> String {
    match &c.last_kind {
        None => String::new(),
        Some(LastMessageKind::Text) => c.last_text.clone().unwrap_or_default(),
        Some(LastMessageKind::Image { .. }) => strings::PREVIEW_IMAGE.into(),
        Some(LastMessageKind::Video { .. }) => strings::PREVIEW_VIDEO.into(),
        Some(LastMessageKind::File { count, .. }) if *count > 1 => {
            format!("[{} {}]", count, strings::FILE_COUNT_UNIT)
        }
        Some(LastMessageKind::File { name, .. }) if name.is_empty() => strings::PREVIEW_FILE.into(),
        Some(LastMessageKind::File { name, .. }) => format!("{} {name}", strings::PREVIEW_FILE),
        Some(LastMessageKind::Folder { name }) => format!("{} {name}", strings::PREVIEW_FOLDER),
        Some(LastMessageKind::System) => strings::PREVIEW_SYSTEM.into(),
        Some(LastMessageKind::Recalled) => strings::MSG_RECALLED.into(),
    }
}

fn conv_row(c: &ConversationSummary, selected: bool, now: i64, tz_offset: i32) -> ConvRow {
    ConvRow {
        conv: c.conv.clone(),
        title: conv_title(c),
        // 预览压成单行，避免换行把行撑高
        preview: conv_preview(c).replace(['\n', '\r'], " "),
        time: c
            .last_hlc_millis
            .map(|t| format_time(t, now, tz_offset))
            .unwrap_or_default(),
        unread: unread_badge(c.unread),
        online: c.peer_online,
        selected,
    }
}

fn conv_item(ui: &UiState, r: ConvRow) -> Element {
    let conv = r.conv.clone();
    let ui_click = ui.clone();
    let unread = r.unread.is_some();
    let mut bottom = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(
            Element::label(r.preview)
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted)
                .max_lines(1)
                .truncate(Truncate::End)
                .weight(1.0),
        );
    if let Some(n) = r.unread {
        bottom = bottom.child(style::badge(n));
    }
    let title = Element::label(r.title.clone())
        .font_size(style::FONT_BODY)
        .fg_role(Role::Text)
        .max_lines(1)
        .truncate(Truncate::End)
        .weight(1.0);
    // 未读时标题加粗（规范第 6 节）
    let title = if unread {
        title.font_weight(style::WEIGHT_STRONG)
    } else {
        title
    };
    // 标题与时间之间留 12，加粗标题截断时也不会贴住时间
    let top = Element::row()
        .width_match()
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .child(title)
        .child(
            Element::label(r.time)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle),
        );
    let selected = r.selected;
    Element::row()
        .width_match()
        .height(style::CONV_ROW_H)
        .padding_xy(style::SPACE_MD, 0)
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        // 选中行 signal_soft 底；悬停由 clickable 的半透明叠层提供
        .bg_when(move || {
            if selected {
                style::tokens().signal_soft
            } else {
                Color::TRANSPARENT
            }
        })
        .clickable()
        .on_click(move |_| {
            let c = conv.clone();
            ui_click.act(|m| m.open_conv(c));
        })
        .child(avatar_in_row(&r.conv, &r.title, r.online, selected))
        .child(
            Element::col()
                .weight(1.0)
                .spacing(style::SPACE_XS)
                .child(top)
                .child(bottom),
        )
}

/// 搜索结果一行的展示数据。
#[derive(Clone, PartialEq)]
struct HitRow {
    msg: String,
    conv: String,
    title: String,
    author: String,
    snippet: String,
    time: String,
}

fn hit_text(m: &MessageView) -> String {
    match &m.body {
        MessageBodyView::Text { text } => text.replace(['\n', '\r'], " "),
        _ => String::new(),
    }
}

fn hit_item(ui: &UiState, r: HitRow) -> Element {
    let conv = r.conv.clone();
    let ui_click = ui.clone();
    let head = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(
            Element::label(r.title)
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End)
                .weight(1.0),
        )
        .child(
            Element::label(r.time)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle),
        );
    Element::col()
        .width_match()
        .padding_xy(style::SPACE_MD, style::SPACE_SM)
        .spacing(style::SPACE_XS)
        .clickable()
        .on_click(move |_| ui_click.open_from_search(conv.clone()))
        .child(head)
        .child(
            Element::label(strings::with_colon(&r.author, &r.snippet))
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted)
                .max_lines(2)
                .truncate(Truncate::End),
        )
}

/// 搜索框：36 高，左侧 16px 放大镜。
/// windui 的 `leading_icon` 只收字形字符，画不了 SVG；这里用空格字形占住图标区，再把 SVG 叠在其上。
/// SVG 图标节点命中不透明，会吃掉落在其上的按下事件（事件只沿祖先链冒泡，到不了同级的输入框），
/// 所以图标包进可点击容器：点击经 `focus_when` 把焦点交给输入框，点放大镜等同点输入框。
fn search_box(ui: &UiState) -> Element {
    let focus_input = signal(0u64);
    let input = Element::text_input(ui.search_text, strings::SEARCH_PLACEHOLDER)
        .leading_icon(' ')
        .width_match()
        .height(style::SEARCH_H)
        .focus_when(focus_input);
    // 输入框内部左内边距 12，图标左缘与之对齐；容器与图标同大，悬停叠层只落在图标上
    let magnifier = Element::stack()
        .size(style::ICON_SM, style::ICON_SM)
        .corner(style::RADIUS_CONTROL / 2.0)
        .focusable(false)
        .clickable()
        .on_click(move |_| focus_input.update(|n| *n += 1))
        .child(style::icon(
            style::ICON_SEARCH,
            style::ICON_SM,
            Role::TextSubtle,
        ))
        .align_xy(Align::Start, Align::Center)
        .margin_edges(Insets {
            left: style::SPACE_MD,
            top: 0,
            right: 0,
            bottom: 0,
        });
    Element::stack()
        .width_match()
        .height(style::SEARCH_H)
        .child(input)
        .child(magnifier)
}

/// 会话列表（含顶部搜索框）。
pub fn chat_list(ui: &UiState) -> Element {
    let search = search_box(ui);
    let searching = ui.derive(|m| !m.search_query.trim().is_empty());

    let rows = ui.derive(|m| {
        let (now, off) = (now_millis(), tz::local_offset_secs());
        m.ordered_convs()
            .into_iter()
            .map(|c| {
                conv_row(
                    c,
                    m.current_conv.as_deref() == Some(c.conv.as_str()),
                    now,
                    off,
                )
            })
            .collect::<Vec<_>>()
    });
    let hits = ui.derive(|m| {
        let (now, off) = (now_millis(), tz::local_offset_secs());
        let q = m.search_query.trim().to_string();
        m.search_results
            .iter()
            .map(|h| HitRow {
                msg: h.id.clone(),
                conv: h.conv.clone(),
                title: m
                    .convs
                    .iter()
                    .find(|c| c.conv == h.conv)
                    .map(conv_title)
                    .unwrap_or_else(|| strings::UNKNOWN_CONV.to_string()),
                author: h.author_device_name.clone(),
                snippet: snippet(&hit_text(h), &q, 20),
                time: format_time(h.hlc_millis, now, off),
            })
            .collect::<Vec<_>>()
    });
    let no_hits = hits.map(|h| h.is_empty());
    let no_convs = rows.map(|r| r.is_empty());

    let ui_conv = ui.clone();
    let ui_hit = ui.clone();
    let body = Element::switcher(searching, move |on| {
        if on {
            let ui = ui_hit.clone();
            Element::col()
                .fill()
                .child(
                    Element::label(strings::SEARCH_EMPTY)
                        .font_size(style::FONT_BODY)
                        .fg_role(Role::TextMuted)
                        .padding(style::SPACE_LG)
                        .visible_signal(no_hits),
                )
                .child(
                    Element::list_signal(
                        hits,
                        |h: &HitRow| h.msg.clone(),
                        move |h| hit_item(&ui, h),
                    )
                    .weight(1.0),
                )
        } else {
            let ui = ui_conv.clone();
            Element::col()
                .fill()
                .child(
                    style::empty_state(
                        strings::CHATS_EMPTY_LIST,
                        strings::CHATS_EMPTY_LIST_HINT,
                        None,
                    )
                    .visible_signal(no_convs),
                )
                .child(
                    Element::list_signal(
                        rows,
                        |r: &ConvRow| r.conv.clone(),
                        move |r| conv_item(&ui, r),
                    )
                    .weight(1.0),
                )
        }
    })
    .weight(1.0);

    let list = Element::col()
        .fill()
        .bg_role(Role::Surface)
        .child(
            Element::col()
                .width_match()
                .padding(style::SPACE_MD)
                .child(search),
        )
        .child(body);
    // 列表与内容区之间的 1px line 由分栏条自带；左侧由图标栏的右分隔线承担
    list
}

/// 会话页：左列表 + 右聊天区，可拖动分栏。
pub fn chats_page(ui: &UiState) -> Element {
    Element::split(
        Axis::Horizontal,
        chat_list(ui),
        chat::chat_pane(ui),
        signal(0.3f32),
    )
    .fill()
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;
    const D: i64 = 24 * H;

    fn summary(last_text: Option<&str>, kind: Option<LastMessageKind>) -> ConversationSummary {
        ConversationSummary {
            conv: "c".into(),
            kind: lanmsg_api::types::ConvKind::Dm,
            title: "t".into(),
            last_text: last_text.map(Into::into),
            last_kind: kind,
            last_hlc_millis: None,
            unread: 0,
            peer_online: false,
        }
    }

    #[test]
    fn conv_preview_per_kind() {
        let p = |t, k| conv_preview(&summary(t, k));
        assert_eq!(p(None, None), "");
        assert_eq!(p(Some("你好"), Some(LastMessageKind::Text)), "你好");
        assert_eq!(p(None, Some(LastMessageKind::Image { count: 2 })), "[图片]");
        assert_eq!(p(None, Some(LastMessageKind::Video { count: 1 })), "[视频]");
        let file = |count| LastMessageKind::File {
            name: "a.pdf".into(),
            count,
        };
        assert_eq!(p(None, Some(file(1))), "[文件] a.pdf");
        assert_eq!(p(None, Some(file(3))), "[3 个文件]");
        let nameless = LastMessageKind::File {
            name: String::new(),
            count: 0,
        };
        assert_eq!(p(None, Some(nameless)), "[文件]");
        assert_eq!(
            p(
                None,
                Some(LastMessageKind::Folder {
                    name: "照片".into()
                })
            ),
            "[文件夹] 照片"
        );
        assert_eq!(p(None, Some(LastMessageKind::System)), "[系统消息]");
        assert_eq!(p(None, Some(LastMessageKind::Recalled)), "消息已撤回");
    }

    #[test]
    fn format_time_cases() {
        // 2026-03-15 09:05:00 UTC，时区 +0 与 +8 各一例
        let now = 1_773_565_500_000;
        assert_eq!(format_time(now - 60_000, now, 0), "09:04");
        assert_eq!(format_time(now, now, 0), "09:05");
        assert_eq!(format_time(now - D, now, 0), "昨天");
        assert_eq!(format_time(now - 2 * D, now, 0), "03-13");
        assert_eq!(format_time(now - 365 * D, now, 0), "03-15");
        // +8 区：本地 17:05，前一日同刻为昨天；UTC 23:00 在 +8 区已是次日
        assert_eq!(format_time(now, now, 8 * 3600), "17:05");
        assert_eq!(format_time(now - 10 * H, now, 8 * 3600), "07:05");
        assert_eq!(format_time(now - 18 * H, now, 8 * 3600), "昨天");
    }

    #[test]
    fn format_time_day_boundaries() {
        // 2026-03-15 00:00:00 UTC
        let midnight = 1_773_532_800_000;
        // 当天 00:00 与 23:59 仍是今天
        assert_eq!(format_time(midnight, midnight + 12 * H, 0), "00:00");
        assert_eq!(
            format_time(midnight + D - 60_000, midnight + D - 1, 0),
            "23:59"
        );
        // 前 1 毫秒属于昨天，次日 00:00 起今天变昨天
        assert_eq!(format_time(midnight - 1, midnight + 12 * H, 0), "昨天");
        assert_eq!(format_time(midnight + D - 1, midnight + D, 0), "昨天");
        assert_eq!(format_time(midnight + D - 1, midnight + D - 1, 0), "23:59");
        // 负时区（UTC-5）：UTC 03:00 在本地是前一日 22:00
        let tz = -5 * 3600;
        assert_eq!(format_time(midnight + 3 * H, midnight + 3 * H, tz), "22:00");
        assert_eq!(format_time(midnight + 3 * H, midnight + 6 * H, tz), "昨天");
        // 跨年：12-31 相对 01-01 为昨天，更早为 MM-DD
        let jan1 = 1_767_225_600_000; // 2026-01-01 00:00:00 UTC
        assert_eq!(format_time(jan1 - 1, jan1, 0), "昨天");
        assert_eq!(format_time(jan1 - D - 1, jan1, 0), "12-30");
    }

    #[test]
    fn unread_badge_cases() {
        assert_eq!(unread_badge(0), None);
        assert_eq!(unread_badge(5), Some("5".to_string()));
        assert_eq!(unread_badge(99), Some("99".to_string()));
        assert_eq!(unread_badge(120), Some("99+".to_string()));
    }

    #[test]
    fn snippet_centers_on_match() {
        assert_eq!(
            snippet("今天下午三点开会讨论年度报告", "年度", 3),
            "…会讨论年度报告"
        );
        assert_eq!(
            snippet("今天下午三点开会讨论年度报告", "不存在", 3),
            "今天下午三点…"
        );
        assert_eq!(snippet("短文", "不存在", 3), "短文");
        assert_eq!(snippet("Hello World", "WORLD", 20), "Hello World");
    }

    #[test]
    fn avatar_initial_handles_cjk_and_emoji() {
        assert_eq!(avatar_initial("张三"), "张");
        assert_eq!(
            avatar_initial("👨\u{200d}👩\u{200d}👧 家"),
            "👨\u{200d}👩\u{200d}👧"
        );
        assert_eq!(avatar_initial("👍🏽 好"), "👍🏽");
        assert_eq!(avatar_initial(""), "?");
        assert_eq!(avatar_initial("  "), "?");
        assert_eq!(avatar_initial("abc"), "a");
    }
}
