// 会话列表与搜索。注释中文，标识符英文。
// 视图只经 UiState 读写；行数据先压成可比较的 Row，再交给 list_signal 做键控复用。

use lanmsg_api::types::{ConversationSummary, MessageBodyView, MessageView};
use windui::prelude::*;

use crate::strings;
use crate::tz;
use crate::ui_state::UiState;

const SECS_PER_DAY: i64 = 86_400;

/// 从 1970-01-01 起的天数换算公历 (月, 日)。
fn civil_month_day(days: i64) -> (u32, u32) {
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

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 头像底色：按 id 哈希在若干强调色角色里选一个（同一会话颜色稳定）。
fn avatar_role(id: &str) -> Role {
    const ROLES: [Role; 5] = [
        Role::Accent,
        Role::Success,
        Role::Warning,
        Role::AccentActive,
        Role::SurfaceInverse,
    ];
    let h = id
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32));
    ROLES[h as usize % ROLES.len()]
}

fn avatar(id: &str, name: &str, online: bool) -> Element {
    let circle = Element::stack()
        .size(40, 40)
        .corner(20.0)
        .bg_role(avatar_role(id))
        .child(
            Element::label(avatar_initial(name))
                .font_size(17.0)
                .fg_role(if avatar_role(id) == Role::SurfaceInverse {
                    Role::OnSurfaceInverse
                } else {
                    Role::OnAccent
                })
                .align(Align::Center),
        );
    // 在线小圆点叠在头像右下角
    let dot = Element::leaf()
        .size(11, 11)
        .corner(5.5)
        .bg_role(Role::Success)
        .border_role(Role::Bg, 2)
        .align_xy(Align::End, Align::End)
        .visible(online);
    Element::stack().size(40, 40).child(circle).child(dot)
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

fn conv_row(c: &ConversationSummary, selected: bool, now: i64, tz_offset: i32) -> ConvRow {
    ConvRow {
        conv: c.conv.clone(),
        title: c.title.clone(),
        // 预览压成单行，避免换行把行撑高
        preview: c
            .last_text
            .as_deref()
            .unwrap_or("")
            .replace(['\n', '\r'], " "),
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
    let mut bottom = Element::row()
        .width_match()
        .spacing(6)
        .cross(Align::Center)
        .child(
            Element::label(r.preview)
                .font_size(13.0)
                .fg_role(Role::TextMuted)
                .max_lines(1)
                .truncate(Truncate::End)
                .weight(1.0),
        );
    if let Some(n) = r.unread {
        bottom = bottom.child(Element::badge(n));
    }
    let top = Element::row()
        .width_match()
        .spacing(6)
        .cross(Align::Center)
        .child(
            Element::label(r.title.clone())
                .font_size(15.0)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End)
                .weight(1.0),
        )
        .child(
            Element::label(r.time)
                .font_size(12.0)
                .fg_role(Role::TextSubtle),
        );
    let mut item = Element::row()
        .width_match()
        .padding_xy(12, 10)
        .spacing(10)
        .cross(Align::Center)
        .clickable()
        .on_click(move |_| {
            let c = conv.clone();
            ui_click.act(|m| m.open_conv(c));
        })
        .child(avatar(&r.conv, &r.title, r.online))
        .child(
            Element::col()
                .weight(1.0)
                .spacing(3)
                .child(top)
                .child(bottom),
        );
    if r.selected {
        item = item.bg_role(Role::SurfaceAlt);
    }
    item
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
        .spacing(6)
        .child(
            Element::label(format!("{} · {}", r.title, r.author))
                .font_size(14.0)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End)
                .weight(1.0),
        )
        .child(
            Element::label(r.time)
                .font_size(12.0)
                .fg_role(Role::TextSubtle),
        );
    Element::col()
        .width_match()
        .padding_xy(12, 10)
        .spacing(3)
        .clickable()
        .on_click(move |_| ui_click.open_from_search(conv.clone()))
        .child(head)
        .child(
            Element::label(r.snippet)
                .font_size(13.0)
                .fg_role(Role::TextMuted)
                .max_lines(2)
                .truncate(Truncate::End),
        )
}

/// 会话列表（含顶部搜索框）。
pub fn chat_list(ui: &UiState) -> Element {
    let search = Element::text_input(ui.search_text, strings::SEARCH_PLACEHOLDER).width_match();
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
                    .map(|c| c.title.clone())
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
                        .fg_role(Role::TextMuted)
                        .padding(16)
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
                    Element::label(strings::CHATS_EMPTY_LIST)
                        .fg_role(Role::TextMuted)
                        .padding(16)
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

    Element::col()
        .fill()
        .bg_role(Role::Surface)
        .child(Element::col().width_match().padding(10).child(search))
        .child(body)
}

/// 右侧聊天区。Task 12 起接入真实消息视图，本任务只放空状态。
fn chat_pane(_ui: &UiState) -> Element {
    Element::stack().fill().child(
        Element::label(strings::CHAT_PANE_EMPTY)
            .font_size(15.0)
            .fg_role(Role::TextMuted)
            .align(Align::Center),
    )
}

/// 会话页：左列表 + 右聊天区，可拖动分栏。
pub fn chats_page(ui: &UiState) -> Element {
    Element::split(
        Axis::Horizontal,
        chat_list(ui),
        chat_pane(ui),
        signal(0.3f32),
    )
    .fill()
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;
    const D: i64 = 24 * H;

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
