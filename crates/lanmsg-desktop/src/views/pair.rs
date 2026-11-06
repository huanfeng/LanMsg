// 配对对话框：发起方显示二维码或 6 位比对数字，加入方选择设备或粘贴配对链接。
// 注释中文，标识符英文。

use lanmsg_api::types::{PairStateDto, PairTarget, PeerView};
use qrcode::{Color as QrColor, QrCode};
use windui::prelude::*;

use crate::model::{pair_link_ok, PairUi};
use crate::strings;
use crate::style;
use crate::ui_state::UiState;

/// 二维码边长（逻辑像素）。
const QR_SIZE: u32 = 220;
/// 静区宽度（模块数）。
const QR_QUIET: u32 = 4;
/// 比对数字单格尺寸。
const SAS_CELL_W: i32 = 32;
const SAS_CELL_H: i32 = 44;
const DIALOG_W: i32 = 420;

/// 把 `data` 渲染成白底黑块的 RGBA 位图：四周各留 4 模块静区，每模块取整数像素，
/// 边长不小于 `px`；返回 `(宽, 高, 像素)`。暗色主题下同样白底黑码，保证可扫。
/// 内容过长无法编码时返回同尺寸的纯白图（调用方给的是短链接，实际不会发生）。
pub fn qr_rgba(data: &str, px: u32) -> (u32, u32, Vec<u8>) {
    let (modules, colors) = match QrCode::new(data.as_bytes()) {
        Ok(code) => (code.width() as u32, code.to_colors()),
        Err(_) => (0, Vec::new()),
    };
    let total = modules + 2 * QR_QUIET;
    let scale = px.div_ceil(total.max(1)).max(1);
    let side = if modules == 0 { px } else { total * scale };
    let mut rgba = vec![255u8; (side * side * 4) as usize];
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / scale, y / scale);
            let inside = mx >= QR_QUIET
                && my >= QR_QUIET
                && mx < QR_QUIET + modules
                && my < QR_QUIET + modules;
            if inside
                && colors[((my - QR_QUIET) * modules + (mx - QR_QUIET)) as usize] == QrColor::Dark
            {
                let i = ((y * side + x) * 4) as usize;
                rgba[i..i + 3].fill(0);
            }
        }
    }
    (side, side, rgba)
}

/// 6 位数字分成两组三位：`"123456"` → `"123 456"`；非 6 位原样返回。
pub fn format_sas(code: &str) -> String {
    if code.chars().count() == 6 && code.chars().all(|c| c.is_ascii_digit()) {
        let (a, b) = code.split_at(3);
        format!("{a} {b}")
    } else {
        code.to_string()
    }
}

/// 对话框宿主，放在页面最上层（向导与主界面共用）；随配对界面开关重建。
pub fn pair_host(ui: &UiState) -> Element {
    let open = ui.derive(|m| m.pair.is_some());
    let ui = ui.clone();
    Element::switcher(open, move |on| {
        if on {
            pair_dialog(&ui)
        } else {
            Element::leaf().size(0, 0)
        }
    })
}

fn muted(text: impl Into<TextContent>) -> Element {
    Element::label(text)
        .font_size(style::FONT_BODY)
        .fg_role(Role::TextMuted)
}

/// 配对对话框（模态）；内容与底栏随 `Model.pair` 的状态切换。
pub fn pair_dialog(ui: &UiState) -> Element {
    let show = signal(true);
    ui.register_pair_show(show);
    let state = ui.digest(|m| m.pair.clone());

    let ui_body = ui.clone();
    let body = Element::switcher(state, move |_| match ui_body.read(|m| m.pair.clone()) {
        Some(PairUi::Picking) => picker(&ui_body),
        Some(PairUi::Showing { qr_uri, state, .. }) => showing(qr_uri.as_deref(), &state),
        Some(PairUi::Starting) | None => muted(strings::PAIR_STARTING).width_match(),
    });
    let ui_foot = ui.clone();
    let footer = Element::switcher(state, move |_| {
        footer(&ui_foot, &ui_foot.read(|m| m.pair.clone()))
    });

    let ui_close = ui.clone();
    Element::dialog_panel(
        show,
        strings::PAIR_TITLE,
        DIALOG_W,
        move |_| ui_close.act(|m| m.pair_cancel()),
        body.width_match(),
        footer.width_match(),
    )
}

/// 发起方 / 加入方共用的状态展示。
fn showing(qr_uri: Option<&str>, state: &PairStateDto) -> Element {
    let col = Element::col()
        .width_match()
        .spacing(style::SPACE_MD)
        .cross(Align::Center);
    match state {
        PairStateDto::WaitingPeer => match qr_uri {
            Some(uri) => {
                let (w, h, rgba) = qr_rgba(uri, QR_SIZE);
                // 二维码居中卡片：白底黑码（位图自带静区），外圈细边在暗色主题下勾出轮廓
                let card = Element::col()
                    .padding(style::SPACE_SM)
                    .corner(style::RADIUS_CARD)
                    .border_role(Role::Border, 1)
                    .bg_role(Role::Surface)
                    .child(
                        Element::image_rgba(w, h, &rgba)
                            .size(w as i32, h as i32)
                            .corner(style::RADIUS_CONTROL),
                    );
                col.child(card)
                    .child(
                        Element::label(strings::PAIR_QR_HINT)
                            .font_size(style::FONT_HEADING)
                            .font_weight(style::WEIGHT_STRONG)
                            .fg_role(Role::Text),
                    )
                    .child(
                        Element::label(strings::PAIR_QR_WAITING)
                            .font_size(style::FONT_META)
                            .fg_role(Role::TextSubtle),
                    )
            }
            None => col
                .child(
                    Element::label(strings::PAIR_SAS_WAITING)
                        .font_size(style::FONT_HEADING)
                        .font_weight(style::WEIGHT_STRONG)
                        .fg_role(Role::Text),
                )
                .child(
                    Element::label(strings::PAIR_SAS_WAITING_HINT)
                        .font_size(style::FONT_BODY)
                        .fg_role(Role::TextMuted)
                        .max_lines(3),
                ),
        },
        PairStateDto::ShowSas { code } => col
            .child(muted(strings::PAIR_SAS_HINT))
            .child(sas_digits(&format_sas(code))),
        PairStateDto::Verifying => col.child(muted(strings::PAIR_VERIFYING)),
        PairStateDto::Done => col.child(
            Element::label(strings::PAIR_DONE)
                .font_size(style::FONT_TITLE)
                .font_weight(style::WEIGHT_STRONG)
                .fg_role(Role::Success),
        ),
        PairStateDto::Failed { reason } => col
            .child(
                Element::label(strings::PAIR_FAILED)
                    .font_size(style::FONT_HEADING)
                    .font_weight(style::WEIGHT_STRONG)
                    .fg_role(Role::Danger),
            )
            .child(
                Element::label(reason.clone())
                    .font_size(style::FONT_BODY)
                    .fg_role(Role::TextMuted)
                    .max_lines(3),
            ),
    }
}

/// 比对数字：每位一个等宽格，两组三位之间留空；格子等宽，不依赖字体的数字宽度。
fn sas_digits(text: &str) -> Element {
    let mut row = Element::row().spacing(style::SPACE_XS).cross(Align::Center);
    for c in text.chars() {
        if c == ' ' {
            row = row.child(Element::leaf().size(style::SPACE_LG, 1));
            continue;
        }
        row = row.child(
            Element::stack()
                .size(SAS_CELL_W, SAS_CELL_H)
                .corner(style::RADIUS_CONTROL)
                .bg_when(|| style::tokens().signal_soft)
                .child(
                    Element::label(c.to_string())
                        .font_size(style::FONT_TITLE)
                        .font_weight(style::WEIGHT_STRONG)
                        .fg_role(Role::Text)
                        .align(Align::Center),
                ),
        );
    }
    row
}

fn footer(ui: &UiState, p: &Option<PairUi>) -> Element {
    let row = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(Element::flex_spacer());
    let ui_cancel = ui.clone();
    let cancel = |label: &'static str| {
        Element::button(label)
            .outline()
            .on_click(move |_| ui_cancel.act(|m| m.pair_cancel()))
    };
    match p {
        Some(PairUi::Showing { state, .. }) => match state {
            PairStateDto::ShowSas { .. } => {
                let (ui_no, ui_yes) = (ui.clone(), ui.clone());
                row.child(
                    Element::button(strings::PAIR_SAS_MISMATCH_BUTTON)
                        .outline()
                        .intent(Intent::Danger)
                        .on_click(move |_| ui_no.act(|m| m.pair_confirm(false))),
                )
                .child(
                    Element::button(strings::PAIR_SAS_MATCH)
                        .on_click(move |_| ui_yes.act(|m| m.pair_confirm(true))),
                )
            }
            PairStateDto::Done => {
                let ui_done = ui.clone();
                row.child(Element::button(strings::PAIR_CLOSE).on_click(move |_| {
                    ui_done.act(|m| m.pair_close());
                }))
            }
            PairStateDto::Failed { .. } => {
                let ui_retry = ui.clone();
                row.child(cancel(strings::PAIR_CLOSE)).child(
                    Element::button(strings::PAIR_RETRY).on_click(move |_| {
                        ui_retry.act(|m| m.pair_retry());
                    }),
                )
            }
            _ => row.child(cancel(strings::PAIR_CANCEL)),
        },
        _ => row.child(cancel(strings::PAIR_CANCEL)),
    }
}

/// 加入方：可见设备列表（点选）+ 粘贴配对链接。
fn picker(ui: &UiState) -> Element {
    let peers = ui.digest(|m| m.peers.clone());
    let link = signal(String::new());
    let link_empty = link.map(|l| l.trim().is_empty());

    let ui_list = ui.clone();
    let list = Element::switcher(peers, move |_| {
        let ps = ui_list.read(|m| m.peers.clone());
        if ps.is_empty() {
            return Element::col()
                .width_match()
                .spacing(style::SPACE_XS)
                .child(muted(strings::PAIR_JOIN_NONE))
                .child(
                    Element::label(strings::PAIR_JOIN_NONE_HINT)
                        .font_size(style::FONT_META)
                        .fg_role(Role::TextSubtle)
                        .max_lines(2),
                );
        }
        let rows = ps.iter().map(|p| peer_row(&ui_list, p));
        Element::scroll()
            .width_match()
            .max_height(192)
            .child(Element::col().width_match().children(rows))
    });

    let link_ok = link.map(|l| pair_link_ok(l));
    // 非空但不合法：红字提示；空输入：说明按钮为何不可用（共用输入框下方同一位置）
    let link_bad = link.map(|l| !l.trim().is_empty() && !pair_link_ok(l));
    let ui_link = ui.clone();
    let link_row = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(
            Element::text_input(link, strings::PAIR_JOIN_LINK_PLACEHOLDER)
                .height(style::CONTROL_H)
                .weight(1.0),
        )
        .child(
            Element::button(strings::PAIR_JOIN_LINK_BUTTON)
                .enabled_when(move || link_ok.get())
                .on_click(move |_| {
                    let uri = link.get().trim().to_string();
                    ui_link.act(|m| m.pair_join(PairTarget::QrUri { uri }));
                }),
        );
    let link_hint = Element::col()
        .width_match()
        .child(
            Element::label(strings::PAIR_JOIN_LINK_INVALID)
                .font_size(style::FONT_META)
                .fg_role(Role::Danger)
                .visible_signal(link_bad),
        )
        .child(
            Element::label(strings::PAIR_JOIN_LINK_EMPTY_HINT)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle)
                .visible_signal(link_empty),
        );

    Element::col()
        .width_match()
        .spacing(style::SPACE_MD)
        .child(
            Element::label(strings::PAIR_JOIN_PICK)
                .font_size(style::FONT_HEADING)
                .font_weight(style::WEIGHT_STRONG)
                .fg_role(Role::Text),
        )
        .child(list)
        .child(Element::divider())
        .child(muted(strings::PAIR_JOIN_LINK_LABEL))
        .child(link_row)
        .child(link_hint)
}

fn peer_row(ui: &UiState, p: &PeerView) -> Element {
    let ui = ui.clone();
    let device = p.device_id.clone();
    Element::row()
        .width_match()
        .height(48)
        .padding_xy(style::SPACE_MD, 0)
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .corner(style::RADIUS_CONTROL)
        .clickable()
        .on_click(move |_| {
            let device = device.clone();
            ui.act(|m| m.pair_join(PairTarget::Device { device }));
        })
        .child(
            Element::col()
                .weight(1.0)
                .child(
                    Element::label(p.device_name.clone())
                        .font_size(style::FONT_BODY)
                        .fg_role(Role::Text)
                        .max_lines(1)
                        .truncate(Truncate::End),
                )
                .child(
                    Element::label(p.display_name.clone())
                        .font_size(style::FONT_META)
                        .fg_role(Role::TextSubtle)
                        .max_lines(1)
                        .truncate(Truncate::End),
                ),
        )
        .child(
            Element::label(format!(
                "{} {}",
                strings::CONTACT_FINGERPRINT,
                p.device_short
            ))
            .font_size(style::FONT_META)
            .fg_role(Role::TextSubtle),
        )
        .child(
            Element::label(strings::PAIR_JOIN_ACTION)
                .font_size(style::FONT_BODY)
                .fg_role(Role::AccentText),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_rgba_is_square_black_on_white() {
        let (w, h, px) = qr_rgba("lanmsg://pair?d=x&s=y", 220);
        assert_eq!(w, h);
        assert!(w >= 200, "边长 {w} 应不小于 200");
        assert_eq!(px.len(), (w * h * 4) as usize);
        // 左上角落在静区：白
        assert_eq!(&px[0..4], &[255, 255, 255, 255]);
        // 存在黑像素，且全部像素非黑即白、不透明
        assert!(px.chunks(4).any(|p| p[..3] == [0, 0, 0]));
        assert!(px
            .chunks(4)
            .all(|p| p[3] == 255 && (p[..3] == [0, 0, 0] || p[..3] == [255, 255, 255])));
    }

    #[test]
    fn qr_rgba_quiet_zone_is_four_modules_on_every_side() {
        let (w, _, px) = qr_rgba("lanmsg://pair?s=demo", 220);
        let at = |x: u32, y: u32| px[((y * w + x) * 4) as usize];
        // 四条边与四个角上的像素都在静区内：全白
        for i in 0..w {
            assert_eq!(at(i, 0), 255);
            assert_eq!(at(0, i), 255);
            assert_eq!(at(i, w - 1), 255);
            assert_eq!(at(w - 1, i), 255);
        }
        assert!(px.chunks(4).filter(|p| p[0] == 0).count() > 100);
    }

    #[test]
    fn qr_rgba_unencodable_falls_back_to_white() {
        let (w, h, px) = qr_rgba(&"x".repeat(5000), 200);
        assert_eq!((w, h), (200, 200));
        assert!(px.iter().all(|b| *b == 255));
    }

    #[test]
    fn format_sas_cases() {
        assert_eq!(format_sas("123456"), "123 456");
        assert_eq!(format_sas("12345"), "12345");
        assert_eq!(format_sas("1234567"), "1234567");
        assert_eq!(format_sas("12a456"), "12a456");
        assert_eq!(format_sas(""), "");
    }
}
