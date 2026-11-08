// 中转站页：左侧站点列表，右侧当前站点的条目；支持添加文件 / 文件夹 / 文字、拖入、置顶、删除。
// 文件条目的下载状态区复用文件页的 `file_actions`。
// 注释中文，标识符英文。

use lanmsg_api::types::{StationItemView, StationView, StoreMode};
use windui::prelude::*;

use crate::model::ExpiryChoice;
use crate::strings;
use crate::style;
use crate::tz;
use crate::ui_state::UiState;
use crate::views::chats::{civil_month_day, format_time, now_millis};
use crate::views::confirm::{confirm_host, Confirm, ConfirmState};
use crate::views::files::{file_actions, meta_label, thumb_or_icon};
use crate::views::message_row::human_size;

const PAGE_PAD: i32 = style::SPACE_XL;
/// 左侧站点列表宽度。
const LIST_W: i32 = 220;
/// 条目行高：所有状态（按钮 / 进度条 / 灰字）都放得下，行与行之间不抖动。
const ROW_H: i32 = 64;
/// 主操作区（复制 / 下载状态 / 失效说明）宽度，内容整体靠右。
const MAIN_W: i32 = 196;
/// 置顶 / 删除两个链接的固定列宽。
const SIDE_W: i32 = 64;
/// 文字条目名称的显示字数上限。
const TEXT_PREVIEW_CHARS: usize = 80;
const SECS_PER_DAY: i64 = 86_400;

/// 下拉选项与存储方式 / 保留时间的对应（顺序即界面顺序）。
const MODES: [StoreMode; 2] = [StoreMode::Reference, StoreMode::Copy];
const EXPIRIES: [ExpiryChoice; 4] = [
    ExpiryChoice::Never,
    ExpiryChoice::Day1,
    ExpiryChoice::Day7,
    ExpiryChoice::Day30,
];

/// 文字条目的显示名：换行折成空格，超长截断。
pub fn text_title(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let mut out: String = flat.trim().chars().take(TEXT_PREVIEW_CHARS).collect();
    if flat.trim().chars().count() > TEXT_PREVIEW_CHARS {
        out.push('…');
    }
    out
}

/// 过期说明（按本地时区换算的月日），没有过期时间为空。
pub fn expires_label(expires_at: Option<i64>, tz_offset_secs: i32) -> Option<String> {
    let t = expires_at?;
    let day = (t.div_euclid(1000) + tz_offset_secs as i64).div_euclid(SECS_PER_DAY);
    let (m, d) = civil_month_day(day);
    Some(strings::station_expires(m, d))
}

/// 当前站点的主机是否在线。
fn host_online(ui: &UiState) -> Signal<bool> {
    ui.derive(|m| {
        m.current_station
            .as_ref()
            .and_then(|c| m.stations.iter().find(|s| &s.station == c))
            .is_some_and(|s| s.host_online)
    })
}

/// 把下拉里的选择写进 Model，之后放入的内容按它处理。
fn sync_options(ui: &UiState, mode: Signal<usize>, expiry: Signal<usize>) {
    let mode = MODES
        .get(mode.get())
        .cloned()
        .unwrap_or(StoreMode::Reference);
    let expiry = EXPIRIES
        .get(expiry.get())
        .copied()
        .unwrap_or(ExpiryChoice::Never);
    ui.act(|m| {
        m.station_mode = mode;
        m.station_expiry = expiry;
        Vec::new()
    });
}

fn put_paths(
    ui: &UiState,
    mode: Signal<usize>,
    expiry: Signal<usize>,
    paths: Vec<std::path::PathBuf>,
) {
    sync_options(ui, mode, expiry);
    ui.act(|m| m.station_put(paths, now_millis()));
}

// ---------------------------------------------------------------------------
// 左侧站点列表
// ---------------------------------------------------------------------------

fn station_row(ui: &UiState, s: &StationView) -> Element {
    let id = s.station.clone();
    let selected = {
        let id = id.clone();
        ui.derive(move |m| m.current_station.as_deref() == Some(id.as_str()))
    };
    let ui_click = ui.clone();
    let dot_role = if s.host_online {
        Role::Success
    } else {
        Role::TextSubtle
    };
    let meta = format!(
        "{} · {}",
        s.host_device,
        strings::station_item_count(s.item_count)
    );
    Element::row()
        .width_match()
        .padding_xy(style::SPACE_MD, style::SPACE_SM)
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .corner(style::RADIUS_CONTROL)
        .bg_when(move || {
            if selected.get() {
                style::tokens().signal_soft
            } else {
                Color::TRANSPARENT
            }
        })
        .clickable()
        .on_click(move |_| ui_click.act(|m| m.open_station(id.clone())))
        .child(Element::leaf().size(8, 8).corner(4.0).bg_role(dot_role))
        .child(
            Element::col()
                .weight(1.0)
                .spacing(style::SPACE_XS)
                .child(
                    Element::label(s.name.clone())
                        .font_size(style::FONT_BODY)
                        .fg_role(Role::Text)
                        .max_lines(1)
                        .truncate(Truncate::End),
                )
                .child(meta_label(meta, Role::TextSubtle)),
        )
}

fn station_list(ui: &UiState, new_show: Signal<bool>) -> Element {
    let key = ui.digest(|m| (m.stations.clone(), m.current_station.clone()));
    let ui_rows = ui.clone();
    let list = Element::switcher(key, move |_| {
        let stations = ui_rows.read(|m| m.stations.clone());
        Element::col()
            .width_match()
            .spacing(style::SPACE_XS)
            .children(stations.iter().map(|s| station_row(&ui_rows, s)))
    });
    Element::col()
        .width(LIST_W)
        .height_match()
        .padding(style::SPACE_MD)
        .spacing(style::SPACE_MD)
        .child(Element::scroll().weight(1.0).child(list))
        .child(
            Element::button(strings::STATION_NEW)
                .outline()
                .width_match()
                .on_click(move |_| new_show.set(true)),
        )
}

// ---------------------------------------------------------------------------
// 右侧条目
// ---------------------------------------------------------------------------

fn item_row(ui: &UiState, it: &StationItemView, confirm: ConfirmState) -> Element {
    let now = now_millis();
    let tz_off = tz::local_offset_secs();
    let title = match (&it.text, &it.file) {
        (Some(t), _) => text_title(t),
        (None, Some(f)) => f.name.clone(),
        (None, None) => String::new(),
    };
    // 标题起点固定；置顶标记放在标题右侧
    let mut name_row = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(
            Element::label(title)
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End)
                .weight(1.0),
        );
    if it.pinned {
        name_row = name_row.child(style::pill(strings::STATION_PINNED_TAG));
    }
    // 信息行：大小、谁在什么时候放的、过期时间
    let mut parts = Vec::new();
    if let Some(f) = &it.file {
        parts.push(human_size(f.size));
    }
    parts.push(format!(
        "{} {}",
        it.added_by,
        format_time(it.added_at, now, tz_off)
    ));
    if let Some(e) = expires_label(it.expires_at, tz_off) {
        parts.push(e);
    }
    let mut meta = Element::flow()
        .width_match()
        .spacing(style::SPACE_MD)
        .run_spacing(style::SPACE_XS);
    for (i, p) in parts.into_iter().enumerate() {
        meta = meta.child(meta_label(
            p,
            if i == 0 {
                Role::TextMuted
            } else {
                Role::TextSubtle
            },
        ));
    }
    let info = Element::col()
        .weight(1.0)
        .spacing(style::SPACE_XS)
        .child(name_row)
        .child(meta);

    let lead = match &it.file {
        Some(f) => thumb_or_icon(f, 40, style::RADIUS_CONTROL),
        None => Element::stack()
            .size(40, 40)
            .corner(style::RADIUS_CONTROL)
            .bg_when(|| style::tokens().signal_soft)
            .child(
                style::icon(style::ICON_TEXT_SNIPPET, 24, Role::AccentText).align(Align::Center),
            ),
    };

    // 主操作：文字 -> 复制；文件 -> 下载状态区；失效 -> 灰字说明
    let main = if it.unavailable == Some(true) {
        meta_label(strings::STATION_UNAVAILABLE, Role::TextMuted)
    } else if let Some(t) = it.text.clone() {
        Element::button(strings::STATION_COPY_TEXT)
            .outline()
            .small()
            .on_click(move |ctx| {
                ctx.clipboard_set(&t);
                ctx.toast(strings::STATION_COPIED);
            })
    } else if let Some(f) = &it.file {
        file_actions(ui, f, String::new())
    } else {
        Element::leaf().size(0, 0)
    };

    let (ui_pin, id_pin, pinned) = (ui.clone(), it.item.clone(), it.pinned);
    let id_del = it.item.clone();
    // 置顶 / 删除：固定宽的窄列，两个链接上下排
    let side = Element::col()
        .width(SIDE_W)
        .spacing(style::SPACE_XS)
        .cross(Align::End)
        .child(
            Element::link(if pinned {
                strings::STATION_UNPIN
            } else {
                strings::STATION_PIN
            })
            .on_click(move |_| ui_pin.act(|m| m.station_set_pinned(id_pin.clone(), !pinned))),
        )
        .child(Element::link(strings::STATION_DELETE).on_click(move |_| {
            confirm.open(Confirm {
                id: id_del.clone(),
                title: strings::STATION_DELETE_TITLE,
                message: strings::STATION_DELETE_MESSAGE.to_string(),
                action: strings::STATION_DELETE,
            })
        }));
    // 主操作区：固定宽、内容靠右，各种状态的对齐方式一致
    let main = Element::col().width(MAIN_W).cross(Align::End).child(main);

    Element::col()
        .width_match()
        .child(
            Element::row()
                .width_match()
                .height(ROW_H)
                .padding_edges(Insets {
                    top: style::SPACE_SM,
                    bottom: style::SPACE_SM,
                    left: style::SPACE_LG,
                    right: style::SPACE_XL,
                })
                .spacing(style::SPACE_MD)
                .cross(Align::Center)
                .child(lead)
                .child(info)
                .child(main)
                .child(side),
        )
        .child(Element::divider())
}

fn items_view(ui: &UiState, confirm: ConfirmState) -> Element {
    let key = ui.digest(|m| (m.station_items.clone(), m.current_station.clone()));
    let ui = ui.clone();
    Element::switcher(key, move |_| {
        let items = ui.read(|m| m.station_items.clone());
        if items.is_empty() {
            return style::empty_state(
                strings::STATION_ITEMS_EMPTY_TITLE,
                strings::STATION_ITEMS_EMPTY,
                None,
            );
        }
        Element::scroll().fill().child(
            Element::col()
                .width_match()
                .children(items.iter().map(|it| item_row(&ui, it, confirm))),
        )
    })
    .fill()
}

/// 标签 + 下拉。
fn labeled_dropdown(
    label: &'static str,
    options: Vec<&'static str>,
    sel: Signal<usize>,
    w: i32,
) -> Element {
    Element::row()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(meta_label(label, Role::TextMuted))
        .child(
            Element::dropdown(options, sel)
                .width(w)
                .height(style::CONTROL_H),
        )
}

fn pick_buttons(
    ui: &UiState,
    mode: Signal<usize>,
    expiry: Signal<usize>,
    text_show: Signal<bool>,
) -> Element {
    let online = host_online(ui);
    let (ui_f, ui_d) = (ui.clone(), ui.clone());
    let files = Element::button(strings::STATION_ADD_FILE)
        .outline()
        .enabled_when(move || online.get())
        .on_click(move |ctx| {
            let ui = ui_f.clone();
            let dlg = PickDialog::new().title(strings::STATION_PICK_FILES_TITLE);
            ctx.request_pick_files(dlg, move |ps| {
                if let Some(ps) = ps.filter(|p| !p.is_empty()) {
                    put_paths(&ui, mode, expiry, ps.to_vec());
                }
            });
        });
    let folder = Element::button(strings::STATION_ADD_FOLDER)
        .outline()
        .enabled_when(move || online.get())
        .on_click(move |ctx| {
            let ui = ui_d.clone();
            let dlg = PickDialog::new().title(strings::STATION_PICK_FOLDER_TITLE);
            ctx.request_pick_folder(dlg, move |p| {
                if let Some(p) = p {
                    put_paths(&ui, mode, expiry, vec![p]);
                }
            });
        });
    let text = Element::button(strings::STATION_ADD_TEXT)
        .outline()
        .enabled_when(move || online.get())
        .on_click(move |_| text_show.set(true));
    Element::row()
        .spacing(style::SPACE_SM)
        .child(files)
        .child(folder)
        .child(text)
}

fn detail(
    ui: &UiState,
    mode: Signal<usize>,
    expiry: Signal<usize>,
    text_show: Signal<bool>,
    confirm: ConfirmState,
) -> Element {
    let title = ui.derive(|m| {
        m.current_station
            .as_ref()
            .and_then(|c| m.stations.iter().find(|s| &s.station == c))
            .map(|s| s.name.clone())
            .unwrap_or_default()
    });
    let online = host_online(ui);
    let status = online.map(|o| {
        if *o {
            strings::STATION_HOST_ONLINE
        } else {
            strings::STATION_HOST_OFFLINE
        }
        .to_string()
    });
    let putting = ui.derive(|m| m.station_putting > 0);
    let ui_drop = ui.clone();
    let header = Element::col()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(
            Element::row()
                .width_match()
                .spacing(style::SPACE_MD)
                .cross(Align::Center)
                .child(
                    Element::label_signal(title)
                        .font_size(style::FONT_TITLE)
                        .font_weight(style::WEIGHT_STRONG)
                        .fg_role(Role::Text)
                        .max_lines(1)
                        .truncate(Truncate::End),
                )
                .child(
                    Element::label_signal(status)
                        .font_size(style::FONT_META)
                        .fg_role_signal(
                            online.map(|o| if *o { Role::Success } else { Role::Warning }),
                        ),
                ),
        )
        .child(pick_buttons(ui, mode, expiry, text_show))
        .child(
            Element::flow()
                .width_match()
                .spacing(style::SPACE_LG)
                .run_spacing(style::SPACE_SM)
                .child(labeled_dropdown(
                    strings::STATION_EXPIRY,
                    vec![
                        strings::STATION_EXPIRY_NEVER,
                        strings::STATION_EXPIRY_1D,
                        strings::STATION_EXPIRY_7D,
                        strings::STATION_EXPIRY_30D,
                    ],
                    expiry,
                    120,
                ))
                .child(
                    Element::dropdown(
                        vec![strings::STATION_MODE_REFERENCE, strings::STATION_MODE_COPY],
                        mode,
                    )
                    .width(180)
                    .height(style::CONTROL_H),
                ),
        )
        .child(
            Element::label(strings::STATION_PUTTING)
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted)
                .visible_signal(putting),
        );
    Element::col()
        .weight(1.0)
        .height_match()
        .on_drop_files(move |_ctx, paths| {
            if ui_drop.read(|m| m.current_station.is_some()) {
                put_paths(&ui_drop, mode, expiry, paths.to_vec());
            }
        })
        .child(
            Element::col()
                .width_match()
                .padding_edges(Insets {
                    top: style::SPACE_XL,
                    bottom: style::SPACE_MD,
                    left: PAGE_PAD,
                    right: PAGE_PAD,
                })
                .child(header),
        )
        .child(Element::divider())
        .child(items_view(ui, confirm).weight(1.0))
}

// ---------------------------------------------------------------------------
// 对话框
// ---------------------------------------------------------------------------

fn new_station_dialog(ui: &UiState, show: Signal<bool>, name: Signal<String>) -> Element {
    let valid = name.map(|n| {
        let c = n.trim().chars().count();
        (1..=64).contains(&c)
    });
    let ui_ok = ui.clone();
    let body = Element::text_input(name, strings::STATION_NAME_PLACEHOLDER)
        .height(style::CONTROL_H)
        .width_match()
        .autofocus();
    let footer = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(Element::flex_spacer())
        .child(
            Element::button(strings::STATION_CANCEL)
                .outline()
                .on_click(move |_| show.set(false)),
        )
        .child(
            Element::button(strings::STATION_NEW_CONFIRM)
                .enabled_when(move || valid.get())
                .on_click(move |_| {
                    let n = name.get().trim().to_string();
                    name.set(String::new());
                    show.set(false);
                    ui_ok.act(|m| m.create_station(n));
                }),
        );
    Element::dialog_panel(
        show,
        strings::STATION_NEW_TITLE,
        360,
        move |_| show.set(false),
        body,
        footer,
    )
}

fn add_text_dialog(
    ui: &UiState,
    show: Signal<bool>,
    text: Signal<String>,
    mode: Signal<usize>,
    expiry: Signal<usize>,
) -> Element {
    let blank = text.map(|t| t.trim().is_empty());
    let ui_ok = ui.clone();
    let body = Element::text_input(text, strings::STATION_TEXT_PLACEHOLDER)
        .multiline()
        .height(120)
        .width_match()
        .autofocus();
    let footer = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(Element::flex_spacer())
        .child(
            Element::button(strings::STATION_CANCEL)
                .outline()
                .on_click(move |_| show.set(false)),
        )
        .child(
            Element::button(strings::STATION_ADD_TEXT_CONFIRM)
                .enabled_when(move || !blank.get())
                .on_click(move |_| {
                    let t = text.get();
                    text.set(String::new());
                    show.set(false);
                    sync_options(&ui_ok, mode, expiry);
                    ui_ok.act(|m| m.station_put_text(t, now_millis()));
                }),
        );
    Element::dialog_panel(
        show,
        strings::STATION_ADD_TEXT_TITLE,
        420,
        move |_| show.set(false),
        body,
        footer,
    )
}

// ---------------------------------------------------------------------------
// 页面
// ---------------------------------------------------------------------------

pub fn stations_page(ui: &UiState) -> Element {
    let new_show = signal(false);
    let new_name = signal(String::new());
    let text_show = signal(false);
    let text_val = signal(String::new());
    // 下拉初值取自 Model（切页回来保持上次的选择）
    let (m0, e0) = ui.read(|m| {
        (
            MODES.iter().position(|x| *x == m.station_mode).unwrap_or(0),
            EXPIRIES
                .iter()
                .position(|x| *x == m.station_expiry)
                .unwrap_or(0),
        )
    });
    let mode = signal(m0);
    let expiry = signal(e0);
    let confirm = ConfirmState::new();
    let ui_ok = ui.clone();

    let empty = ui.derive(|m| m.stations.is_empty());
    let ui_body = ui.clone();
    let body = Element::switcher(empty, move |is_empty| {
        if is_empty {
            let create =
                Element::button(strings::STATION_NEW).on_click(move |_| new_show.set(true));
            style::empty_state(
                strings::STATION_EMPTY_TITLE,
                strings::STATION_EMPTY,
                Some(create),
            )
        } else {
            Element::row()
                .fill()
                .child(station_list(&ui_body, new_show))
                .child(style::vline())
                .child(detail(&ui_body, mode, expiry, text_show, confirm))
        }
    })
    .fill();

    Element::stack()
        .fill()
        .bg_role(Role::Bg)
        .child(body)
        .child(new_station_dialog(ui, new_show, new_name))
        .child(add_text_dialog(ui, text_show, text_val, mode, expiry))
        .child(confirm_host(confirm, move |item| {
            ui_ok.act(|m| m.station_remove(item));
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_title_flattens_and_truncates() {
        assert_eq!(text_title("a\nb"), "a b");
        let long = "字".repeat(100);
        let t = text_title(&long);
        assert_eq!(t.chars().count(), TEXT_PREVIEW_CHARS + 1);
        assert!(t.ends_with('…'));
        assert_eq!(text_title(&"x".repeat(80)).chars().count(), 80);
    }

    #[test]
    fn expires_label_uses_local_month_day() {
        // 2026-03-05 23:30 UTC，东八区已是 3月6日
        let t = 1_772_753_400_000;
        assert_eq!(expires_label(Some(t), 0).as_deref(), Some("· 3月5日 过期"));
        assert_eq!(
            expires_label(Some(t), 8 * 3600).as_deref(),
            Some("· 3月6日 过期")
        );
        assert_eq!(expires_label(None, 0), None);
    }
}
