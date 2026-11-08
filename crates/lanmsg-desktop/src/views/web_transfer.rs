// 网页传输窗口：二维码 / 地址 / 配对码、浏览器连接请求的允许与拒绝、访客会话入口。
// 由 shell 以模态面板叠加显示（`Model.web_open`）；隐藏只关窗口，不停止服务。
// 注释中文，标识符英文。

use lanmsg_api::types::{ConvKind, ConversationSummary};
use windui::prelude::*;

use crate::event_text::conv_title;
use crate::model::{WebRequestUi, WebUi};
use crate::strings;
use crate::style;
use crate::ui_state::UiState;
use crate::views::pair::qr_rgba;

const DIALOG_W: i32 = 600;
const QR_SIZE: u32 = 180;
/// 访客列表最多占的高度，超出滚动。
const GUESTS_MAX_H: i32 = 110;
/// 配对码字号。
const CODE_FONT: f32 = 30.0;
/// 无法识别浏览器时保留的 UA 前缀长度（字符数）。
const UA_FALLBACK_CHARS: usize = 40;

/// 配对码分组显示：8 字符 → `"XXXX XXXX"`，其他长度原样。
pub fn group_code(code: &str) -> String {
    if code.chars().count() == 8 {
        let head: String = code.chars().take(4).collect();
        let tail: String = code.chars().skip(4).collect();
        format!("{head} {tail}")
    } else {
        code.to_string()
    }
}

/// 浏览器摘要：识别 Edge / Chrome / Firefox / Safari，识别不了取前 40 个字符；
/// 能识别系统时追加 ` · 系统名`。
pub fn ua_summary(ua: &str) -> String {
    let browser = if ua.contains("Edg/") {
        "Edge".to_string()
    } else if ua.contains("Chrome/") {
        "Chrome".to_string()
    } else if ua.contains("Firefox/") {
        "Firefox".to_string()
    } else if ua.contains("Safari/") {
        "Safari".to_string()
    } else {
        ua.chars().take(UA_FALLBACK_CHARS).collect()
    };
    // Android UA 同时含 Linux，iPhone UA 同时含 Mac OS X：先判更具体的
    let os = [
        ("Android", "Android"),
        ("iPhone", "iPhone"),
        ("Windows", "Windows"),
        ("Mac OS X", "macOS"),
        ("Linux", "Linux"),
    ]
    .iter()
    .find(|(key, _)| ua.contains(key))
    .map(|(_, name)| *name);
    match os {
        Some(os) => format!("{browser} · {os}"),
        None => browser,
    }
}

fn muted(text: impl Into<TextContent>) -> Element {
    Element::label(text)
        .font_size(style::FONT_META)
        .fg_role(Role::TextMuted)
        .width_match()
}

/// 二维码卡片：白底黑码，外圈细边在暗色主题下勾出轮廓。
fn qr_card(url: &str) -> Element {
    let (w, h, rgba) = qr_rgba(url, QR_SIZE);
    Element::col()
        .padding(style::SPACE_SM)
        .corner(style::RADIUS_CARD)
        .border_role(Role::Border, 1)
        .bg_role(Role::Surface)
        .child(
            Element::image_rgba(w, h, &rgba)
                .size(w as i32, h as i32)
                .corner(style::RADIUS_CONTROL),
        )
}

/// 屏幕上显示的地址：去掉 `#k=` 之后的密钥片段（密钥只进二维码和复制内容，不得显示）。
pub fn display_url(full: &str) -> String {
    full.split_once('#')
        .map_or(full, |(head, _)| head)
        .to_string()
}

/// 第 `i` 个地址的（显示文本，完整地址）；越界取空串。
fn url_pair(urls: &[String], i: usize) -> (String, String) {
    let full = urls.get(i).cloned().unwrap_or_default();
    (display_url(&full), full)
}

/// 复制按钮：复制 `full`（含密钥）。
fn copy_button(full: String) -> Element {
    Element::button(strings::WEB_COPY)
        .outline()
        .small()
        .on_click(move |ctx| {
            ctx.clipboard_set(&full);
            ctx.toast(strings::WEB_COPIED);
        })
}

/// 单个地址：显示文本 + 复制按钮。
fn url_row(display: String, full: String) -> Element {
    Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(
            Element::label(display)
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::Middle)
                .weight(1.0),
        )
        .child(copy_button(full))
}

/// 会话信息区：左侧二维码，右侧地址（多个时为下拉 + 复制）和配对码。
/// `sel` 是当前选中的地址下标，由 `UiState` 持有，窗口或会话信息重建后保持。
fn session_block(w: &WebUi, sel: Signal<usize>) -> Element {
    let urls = w.urls();
    if sel.get() >= urls.len() {
        sel.set(0);
    }
    let urls_qr = urls.clone();
    // 切换地址只重建二维码，配对码不变
    let qr = Element::switcher(sel, move |i| qr_card(&url_pair(&urls_qr, i).1));
    let mut right = Element::col()
        .weight(1.0)
        .spacing(style::SPACE_SM)
        .child(muted(strings::WEB_SCAN_HINT));
    if urls.len() > 1 {
        let shown: Vec<String> = urls.iter().map(|u| display_url(u)).collect();
        let urls_copy = urls.clone();
        right = right.child(
            Element::row()
                .width_match()
                .spacing(style::SPACE_SM)
                .cross(Align::Center)
                .child(
                    Element::dropdown(shown, sel)
                        .height(style::CONTROL_H)
                        .weight(1.0),
                )
                .child(
                    Element::button(strings::WEB_COPY)
                        .outline()
                        .small()
                        .on_click(move |ctx| {
                            ctx.clipboard_set(&url_pair(&urls_copy, sel.get()).1);
                            ctx.toast(strings::WEB_COPIED);
                        }),
                ),
        );
    } else {
        let (display, full) = url_pair(&urls, 0);
        right = right.child(url_row(display, full));
    }
    let right = right
        .child(
            Element::label(group_code(&w.info.code))
                .font_size(CODE_FONT)
                .font_weight(style::WEIGHT_STRONG)
                .fg_role(Role::Text),
        )
        .child(muted(strings::WEB_CODE_HINT));
    Element::row()
        .width_match()
        .spacing(style::SPACE_LG)
        .child(qr)
        .child(right)
}

/// 浏览器请求横幅：只显示最早的一条，处理后显示下一条。
fn request_banner(ui: &UiState, r: &WebRequestUi) -> Element {
    let (ui_allow, ui_deny) = (ui.clone(), ui.clone());
    let (req_allow, req_deny) = (r.request.clone(), r.request.clone());
    Element::col()
        .width_match()
        .padding(style::SPACE_MD)
        .spacing(style::SPACE_SM)
        .corner(style::RADIUS_CARD)
        .bg_when(|| style::tokens().signal_soft)
        .child(
            Element::label(strings::web_request_text(&r.ip, &ua_summary(&r.user_agent)))
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(4)
                .width_match(),
        )
        .child(
            Element::row()
                .width_match()
                .spacing(style::SPACE_SM)
                .child(Element::flex_spacer())
                .child(
                    Element::button(strings::WEB_DENY)
                        .outline()
                        .on_click(move |_| ui_deny.act(|m| m.web_respond(&req_deny, false))),
                )
                .child(
                    Element::button(strings::WEB_ALLOW)
                        .on_click(move |_| ui_allow.act(|m| m.web_respond(&req_allow, true))),
                ),
        )
}

/// 访客会话行：标题（界面按类型本地化）+ 最近一条预览 + “打开会话”。
fn guest_row(ui: &UiState, c: &ConversationSummary) -> Element {
    let (ui_open, conv) = (ui.clone(), c.conv.clone());
    Element::row()
        .width_match()
        .padding_xy(0, style::SPACE_XS)
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .child(
            Element::label(conv_title(c))
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End)
                .weight(1.0),
        )
        .child(
            Element::button(strings::WEB_OPEN_CONV)
                .outline()
                .small()
                .on_click(move |_| {
                    ui_open.act(|m| {
                        m.web_open = false;
                        Vec::new()
                    });
                    ui_open.request_open_conv(conv.clone());
                }),
        )
}

fn guests_block(ui: &UiState) -> Element {
    let guests: Vec<ConversationSummary> = ui.read(|m| {
        m.convs
            .iter()
            .filter(|c| c.kind == ConvKind::Guest)
            .cloned()
            .collect()
    });
    let mut col = Element::col().width_match().spacing(style::SPACE_XS).child(
        Element::label(strings::WEB_GUESTS)
            .font_size(style::FONT_HEADING)
            .font_weight(style::WEIGHT_STRONG)
            .fg_role(Role::Text),
    );
    if guests.is_empty() {
        col = col.child(muted(strings::WEB_GUESTS_EMPTY));
    }
    let list = Element::col()
        .width_match()
        .children(guests.iter().map(|c| guest_row(ui, c)));
    col.child(
        Element::scroll()
            .width_match()
            .max_height(GUESTS_MAX_H)
            .child(list),
    )
}

/// 窗口宿主，放在页面最上层；随 `web_open` 开关重建。
pub fn web_transfer_host(ui: &UiState) -> Element {
    let open = ui.derive(|m| m.web_open);
    let ui = ui.clone();
    Element::switcher(open, move |on| {
        if on {
            web_transfer_window(&ui)
        } else {
            Element::leaf().size(0, 0)
        }
    })
}

/// 网页传输窗口（模态面板）。
pub fn web_transfer_window(ui: &UiState) -> Element {
    let show = signal(true);
    ui.register_web_show(show);

    // 三块独立重建：请求横幅、会话信息、访客列表；某一块变化不重置另两块（如地址下拉的选择）
    let ui_req = ui.clone();
    let pending = Element::switcher(
        ui.digest(|m| m.web.as_ref().and_then(|w| w.pending.first().cloned())),
        move |_| match ui_req.read(|m| m.web.as_ref().and_then(|w| w.pending.first().cloned())) {
            Some(r) => request_banner(&ui_req, &r),
            None => Element::leaf().size(0, 0),
        },
    )
    .width_match();
    let ui_info = ui.clone();
    let info = Element::switcher(
        ui.digest(|m| m.web.as_ref().map(|w| w.info.clone())),
        move |_| match ui_info.read(|m| m.web.clone()) {
            Some(w) if !w.is_placeholder() => session_block(&w, ui_info.web_url_sel),
            _ => muted(strings::WEB_STARTING),
        },
    )
    .width_match();
    let ui_guests = ui.clone();
    let guests = Element::switcher(
        ui.digest(|m| {
            m.convs
                .iter()
                .filter(|c| c.kind == ConvKind::Guest)
                .cloned()
                .collect::<Vec<_>>()
        }),
        move |_| guests_block(&ui_guests),
    )
    .width_match();

    let body = Element::col()
        .width_match()
        .spacing(style::SPACE_LG)
        .child(pending)
        .child(info)
        .child(guests);

    let (ui_hide, ui_stop, ui_close) = (ui.clone(), ui.clone(), ui.clone());
    let footer = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(Element::flex_spacer())
        .child(
            Element::button(strings::WEB_HIDE)
                .outline()
                .on_click(move |_| {
                    ui_hide.act(|m| {
                        m.web_open = false;
                        Vec::new()
                    })
                }),
        )
        .child(
            Element::button(strings::WEB_STOP)
                .danger()
                .on_click(move |_| ui_stop.act(|m| m.web_stop())),
        );
    Element::dialog_panel(
        show,
        strings::WEB_TITLE,
        DIALOG_W,
        move |_| {
            ui_close.act(|m| {
                m.web_open = false;
                Vec::new()
            })
        },
        body,
        footer,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_url_hides_key_but_full_url_keeps_it() {
        let full = "http://192.168.1.2:53330/#k=Zm9v".to_string();
        assert_eq!(display_url(&full), "http://192.168.1.2:53330/");
        assert_eq!(display_url("http://10.0.0.1:1/"), "http://10.0.0.1:1/");
        // 二维码与复制取完整地址，显示取去密钥的地址
        let urls = vec![full.clone(), "http://10.8.0.2:53330/#k=Zm9v".to_string()];
        let (shown, copied) = url_pair(&urls, 1);
        assert!(!shown.contains("#k="));
        assert!(copied.contains("#k="));
        assert_eq!(url_pair(&urls, 9), (String::new(), String::new()));
    }

    #[test]
    fn group_code_4_4() {
        assert_eq!(group_code("K7QM3XAP"), "K7QM 3XAP");
        assert_eq!(group_code("abc"), "abc");
        assert_eq!(group_code(""), "");
    }

    #[test]
    fn ua_summary_cases() {
        let android_chrome = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 \
            (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36";
        assert_eq!(ua_summary(android_chrome), "Chrome · Android");
        let iphone_safari = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) \
            AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";
        assert_eq!(ua_summary(iphone_safari), "Safari · iPhone");
        let edge = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
            (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0";
        assert_eq!(ua_summary(edge), "Edge · Windows");
        let firefox = "Mozilla/5.0 (X11; Linux x86_64; rv:127.0) Gecko/20100101 Firefox/127.0";
        assert_eq!(ua_summary(firefox), "Firefox · Linux");
        let mac_safari = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
            (KHTML, like Gecko) Version/17.5 Safari/605.1.15";
        assert_eq!(ua_summary(mac_safari), "Safari · macOS");
    }

    #[test]
    fn ua_summary_unknown_is_truncated_to_40_chars() {
        let ua = "x".repeat(100);
        assert_eq!(ua_summary(&ua), "x".repeat(40));
        assert_eq!(ua_summary(""), "");
    }
}
