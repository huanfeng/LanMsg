// 设置页：分组卡片 + 表单行。
// 个人、我的设备、下载与存储、网络、通用、关于；改动后点“保存”才写入（SetSettings / SetProfile），
// 数值或地址非法时保存按钮禁用，并在字段下方用红字说明。开机自启开关即时生效。
// 注释中文，标识符英文。

use std::net::{IpAddr, SocketAddr};

use ipnet::IpNet;
use lanmsg_api::types::SettingsDto;
use lanmsg_proto::ids::DeviceId;
use windui::prelude::*;

use crate::autostart;
use crate::model::wizard_name_ok;
use crate::strings;
use crate::style;
use crate::ui_state::UiState;
use crate::views::confirm::ConfirmState;
use crate::views::devices::{devices_panel, remove_confirm_host};

/// 页面内容最大宽度。
const CONTENT_MAX_W: i32 = 760;
/// 表单行左侧标签列宽度。
const LABEL_W: i32 = 168;
/// 单行输入框宽度（名字类字段不需要铺满）。
const FIELD_W: i32 = 280;
/// 1 MB 的字节数。
const MIB: u64 = 1024 * 1024;
/// 自动下载上限可填的 MB 范围。
const MB_RANGE: std::ops::RangeInclusive<u64> = 1..=10_240;

// ---------------------------------------------------------------------------
// 纯函数
// ---------------------------------------------------------------------------

/// 解析“MB”输入：1–10240 的整数，返回字节数；其余（空、负数、小数、越界）为 None。
pub fn parse_mb(s: &str) -> Option<u64> {
    let mb: u64 = s.trim().parse().ok()?;
    MB_RANGE.contains(&mb).then(|| mb * MIB)
}

/// 字节数换成输入框里的 MB 文字（向下取整；非整 MB 的旧值保存时会按整 MB 写回）。
fn mb_text(bytes: u64) -> String {
    (bytes / MIB).to_string()
}

/// 解析单播通告目标：每行一个，去首尾空白、忽略空行；每行必须是 IP、`IP:端口` 或网段（CIDR）。
/// 出错返回带行号的说明（行号按原文计，空行也占行号）。
pub fn parse_targets(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (i, raw) in s.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let ok = line.parse::<IpAddr>().is_ok()
            || line.parse::<SocketAddr>().is_ok()
            || line.parse::<IpNet>().is_ok();
        if !ok {
            return Err(strings::settings_target_invalid(i + 1, line));
        }
        out.push(line.to_string());
    }
    Ok(out)
}

/// 本机设备指纹（由设备 ID 推出）；ID 不是合法 hex 时为空串。
fn device_fingerprint(device_id: &str) -> String {
    DeviceId::from_hex(device_id)
        .map(|d| d.short())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 视图部件
// ---------------------------------------------------------------------------

fn group_title(text: &'static str) -> Element {
    Element::label(text)
        .font_size(style::FONT_HEADING)
        .font_weight(style::WEIGHT_STRONG)
        .fg_role(Role::Text)
}

/// 一个分组：标题在上，内容紧随其后（标题与内容的间距小于分组之间的间距）。
fn group(title: &'static str, content: Element) -> Element {
    Element::col()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(group_title(title))
        .child(content)
}

/// 分组卡片：行与行之间 1px 分隔线。
fn card(rows: Vec<Element>) -> Element {
    let mut col = Element::col()
        .width_match()
        .bg_role(Role::Surface)
        .border_role(Role::Border, 1)
        .corner(style::RADIUS_CARD);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            col = col.child(Element::divider());
        }
        col = col.child(r);
    }
    col
}

/// 说明文字（meta 字号）。
fn note(text: &'static str) -> Element {
    Element::label(text)
        .font_size(style::FONT_META)
        .fg_role(Role::TextMuted)
        .width_match()
}

/// 校验失败的红字：`bad` 为真时显示。
fn error_note(text: impl Into<TextContent>, bad: Signal<bool>) -> Element {
    Element::label(text)
        .font_size(style::FONT_META)
        .fg_role(Role::Danger)
        .width_match()
        .visible_signal(bad)
}

/// 表单行：左标签、右控件，控件下方放说明与错误提示。
fn field(label: &'static str, control: Element, below: Vec<Element>) -> Element {
    let right = Element::col()
        .weight(1.0)
        .spacing(style::SPACE_XS)
        .child(control)
        .children(below);
    Element::row()
        .width_match()
        .padding(style::SPACE_LG)
        .spacing(style::SPACE_MD)
        .child(
            Element::label(label)
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .width(LABEL_W)
                .margin_edges(Insets {
                    top: 8,
                    bottom: 0,
                    left: 0,
                    right: 0,
                }),
        )
        .child(right)
}

/// 开关行：标题 + 说明在左，开关在右。
fn switch_row(label: &'static str, hint: &'static str, sw: Element) -> Element {
    let mut text = Element::col().weight(1.0).spacing(style::SPACE_XS).child(
        Element::label(label)
            .font_size(style::FONT_BODY)
            .fg_role(Role::Text),
    );
    if !hint.is_empty() {
        text = text.child(note(hint));
    }
    Element::row()
        .width_match()
        .padding(style::SPACE_LG)
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .child(text)
        .child(sw)
}

/// 只读信息行（关于）。
fn info_row(label: &'static str, value: String) -> Element {
    Element::row()
        .width_match()
        .padding(style::SPACE_LG)
        .spacing(style::SPACE_MD)
        .child(
            Element::label(label)
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .width(LABEL_W),
        )
        .child(
            Element::label(value)
                .font_size(style::FONT_BODY)
                .fg_role(Role::TextMuted)
                .weight(1.0),
        )
}

// ---------------------------------------------------------------------------
// 页面
// ---------------------------------------------------------------------------

/// 设置表单；控件的草稿值放在本函数创建的信号里，设置从后台重新读到时整页重建。
fn form(ui: &UiState, s: SettingsDto, remove_confirm: ConfirmState) -> Element {
    let me = ui.read(|m| m.me.clone());
    let display = signal(
        me.as_ref()
            .and_then(|m| m.display_name.clone())
            .unwrap_or_default(),
    );
    let device = signal(s.device_name.clone());
    let dir = signal(s.download_dir.clone());
    let image_mb = signal(mb_text(s.auto_image_max));
    let file_mb = signal(mb_text(s.auto_file_max));
    let wifi_only = signal(s.auto_file_wifi_only);
    let localsend = signal(s.localsend_enabled);
    let targets = signal(s.unicast_targets.join("\n"));
    let auto_start = signal(autostart::is_enabled());

    // 校验：每项一个派生信号，保存按钮要全部通过
    let display_ok = display.map(|v| wizard_name_ok(v));
    let device_ok = device.map(|v| wizard_name_ok(v));
    let dir_ok = dir.map(|v| !v.trim().is_empty());
    let image_ok = image_mb.map(|v| parse_mb(v).is_some());
    let file_ok = file_mb.map(|v| parse_mb(v).is_some());
    let targets_err = targets.map(|v| parse_targets(v).err().unwrap_or_default());
    let targets_ok = targets_err.map(|e| e.is_empty());
    let not = |ok: Signal<bool>| ok.map(|v| !*v);
    let all_ok = move || {
        display_ok.get()
            && device_ok.get()
            && dir_ok.get()
            && image_ok.get()
            && file_ok.get()
            && targets_ok.get()
    };

    let personal = card(vec![
        field(
            strings::SETTINGS_DISPLAY_NAME,
            Element::text_input(display, "")
                .width(FIELD_W)
                .height(style::CONTROL_H),
            vec![
                note(strings::SETTINGS_DISPLAY_NAME_HINT),
                error_note(strings::SETTINGS_DISPLAY_NAME_INVALID, not(display_ok)),
            ],
        ),
        field(
            strings::SETTINGS_DEVICE_NAME,
            Element::text_input(device, "")
                .width(FIELD_W)
                .height(style::CONTROL_H),
            vec![
                note(strings::SETTINGS_DEVICE_NAME_HINT),
                error_note(strings::SETTINGS_DEVICE_NAME_INVALID, not(device_ok)),
            ],
        ),
    ]);

    let pick_failed = signal(false);
    let pick = Element::button(strings::SETTINGS_PICK_DIR)
        .outline()
        .on_click(move |ctx| {
            pick_failed.set(false);
            let dlg = PickDialog::new().title(strings::SETTINGS_PICK_DIR_TITLE);
            ctx.request_pick_folder(dlg, move |p| match p {
                Some(p) => dir.set(p.to_string_lossy().into_owned()),
                // 用户取消不提示；弹不出选择框时才说明，仍可直接输入路径
                None => pick_failed.set(PickDialog::last_pick_failed()),
            });
        });
    let mb_input = |sig: Signal<String>| {
        Element::row()
            .spacing(style::SPACE_SM)
            .cross(Align::Center)
            .child(
                Element::text_input(sig, "")
                    .numeric()
                    .width(96)
                    .height(style::CONTROL_H),
            )
            .child(
                Element::label(strings::SETTINGS_MB)
                    .font_size(style::FONT_BODY)
                    .fg_role(Role::TextMuted),
            )
    };
    let storage = card(vec![
        field(
            strings::SETTINGS_DOWNLOAD_DIR,
            Element::row()
                .width_match()
                .spacing(style::SPACE_SM)
                .cross(Align::Center)
                .child(
                    Element::text_input(dir, "")
                        .height(style::CONTROL_H)
                        .weight(1.0),
                )
                .child(pick),
            vec![
                error_note(strings::SETTINGS_DOWNLOAD_DIR_INVALID, not(dir_ok)),
                error_note(strings::SETTINGS_PICK_DIR_FAILED, pick_failed),
            ],
        ),
        field(
            strings::SETTINGS_AUTO_IMAGE,
            mb_input(image_mb),
            vec![error_note(strings::SETTINGS_MB_INVALID, not(image_ok))],
        ),
        field(
            strings::SETTINGS_AUTO_FILE,
            mb_input(file_mb),
            vec![
                note(strings::SETTINGS_AUTO_HINT),
                error_note(strings::SETTINGS_MB_INVALID, not(file_ok)),
            ],
        ),
        switch_row(strings::SETTINGS_WIFI_ONLY, "", Element::switch(wifi_only)),
    ]);

    let network = card(vec![
        switch_row(
            strings::SETTINGS_LOCALSEND,
            strings::SETTINGS_LOCALSEND_HINT,
            Element::switch(localsend),
        ),
        field(
            strings::SETTINGS_TARGETS,
            Element::text_input(targets, strings::SETTINGS_TARGETS_PLACEHOLDER)
                .multiline()
                .width_match()
                .height(96),
            vec![
                note(strings::SETTINGS_TARGETS_HINT),
                error_note(targets_err, not(targets_ok)),
            ],
        ),
    ]);

    let data_dir = ui.data_dir().to_path_buf();
    let general = card(vec![switch_row(
        strings::SETTINGS_AUTOSTART,
        strings::SETTINGS_AUTOSTART_HINT,
        Element::switch(auto_start).on_switch_change(move |ctx, on| {
            // 即时生效；失败时把开关拨回去并说明原因
            if let Err(e) = autostart::set_enabled(on, &data_dir) {
                auto_start.set(!on);
                ctx.toast(format!("{}{}", strings::SETTINGS_AUTOSTART_FAILED, e));
            }
        }),
    )]);

    let (user_fp, device_fp) = match &me {
        Some(m) => (
            m.user_short.clone().unwrap_or_default(),
            device_fingerprint(&m.device_id),
        ),
        None => (String::new(), String::new()),
    };
    let mut about_rows = vec![
        info_row(
            strings::SETTINGS_ABOUT_VERSION,
            env!("CARGO_PKG_VERSION").into(),
        ),
        info_row(
            strings::SETTINGS_ABOUT_LICENSE,
            strings::SETTINGS_ABOUT_LICENSE_VALUE.into(),
        ),
    ];
    if !user_fp.is_empty() {
        about_rows.push(info_row(strings::SETTINGS_ABOUT_USER_FINGERPRINT, user_fp));
    }
    if !device_fp.is_empty() {
        about_rows.push(info_row(
            strings::SETTINGS_ABOUT_DEVICE_FINGERPRINT,
            device_fp,
        ));
    }
    let about = card(about_rows);

    let ui_save = ui.clone();
    let base = s;
    let save = Element::button(strings::SETTINGS_SAVE)
        .enabled_when(all_ok)
        .on_click(move |_| {
            let mut dto = base.clone();
            dto.device_name = device.get().trim().to_string();
            dto.download_dir = dir.get().trim().to_string();
            dto.auto_image_max = parse_mb(&image_mb.get()).unwrap_or(base.auto_image_max);
            dto.auto_file_max = parse_mb(&file_mb.get()).unwrap_or(base.auto_file_max);
            dto.auto_file_wifi_only = wifi_only.get();
            dto.localsend_enabled = localsend.get();
            dto.unicast_targets =
                parse_targets(&targets.get()).unwrap_or_else(|_| base.unicast_targets.clone());
            let name = display.get();
            ui_save.act(|m| m.save_settings(dto, &name));
        });
    let blocked = Element::label(strings::SETTINGS_SAVE_BLOCKED)
        .font_size(style::FONT_META)
        .fg_role(Role::Danger)
        .visible_when(move || !all_ok());

    let actions = Element::row()
        .width_match()
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .child(save)
        .child(blocked);
    Element::col()
        .width_match()
        .max_width(CONTENT_MAX_W)
        .padding(style::SPACE_XL)
        .spacing(style::SPACE_XL)
        .child(
            Element::label(strings::PAGE_SETTINGS)
                .font_size(style::FONT_TITLE)
                .font_weight(style::WEIGHT_STRONG)
                .fg_role(Role::Text),
        )
        .child(group(strings::SETTINGS_GROUP_PERSONAL, personal))
        .child(group(
            strings::SETTINGS_GROUP_DEVICES,
            devices_panel(ui, remove_confirm),
        ))
        .child(group(strings::SETTINGS_GROUP_STORAGE, storage))
        .child(group(strings::SETTINGS_GROUP_NETWORK, network))
        .child(actions)
        .child(group(strings::SETTINGS_GROUP_GENERAL, general))
        .child(group(strings::SETTINGS_GROUP_ABOUT, about))
}

/// 设置页：设置到达前显示读取提示；到达（或保存后重新读到）时按新值重建表单。
/// 移除设备的确认框放在滚动区之外的最上层，遮罩才是全窗的。
pub fn settings_page(ui: &UiState) -> Element {
    let key = ui.digest(|m| m.settings.clone());
    let remove_confirm = ConfirmState::new();
    let ui_form = ui.clone();
    let body = Element::switcher(key, move |_| match ui_form.read(|m| m.settings.clone()) {
        None => Element::label(strings::SETTINGS_LOADING)
            .font_size(style::FONT_BODY)
            .fg_role(Role::TextMuted)
            .margin(style::SPACE_XL),
        Some(s) => form(&ui_form, s, remove_confirm),
    });
    Element::stack()
        .fill()
        .bg_role(Role::Bg)
        .child(Element::scroll().fill().child(body))
        .child(remove_confirm_host(ui, remove_confirm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mb_accepts_range_only() {
        assert_eq!(parse_mb("1"), Some(MIB));
        assert_eq!(parse_mb(" 50 "), Some(50 * MIB));
        assert_eq!(parse_mb("10240"), Some(10_240 * MIB));
        assert_eq!(parse_mb("0"), None);
        assert_eq!(parse_mb("10241"), None);
        assert_eq!(parse_mb(""), None);
        assert_eq!(parse_mb("-5"), None);
        assert_eq!(parse_mb("1.5"), None);
        assert_eq!(parse_mb("abc"), None);
        assert_eq!(parse_mb("99999999999999999999"), None);
    }

    #[test]
    fn mb_text_round_trips_whole_megabytes() {
        assert_eq!(mb_text(50 * MIB), "50");
        assert_eq!(parse_mb(&mb_text(10 * MIB)), Some(10 * MIB));
    }

    #[test]
    fn parse_targets_accepts_ip_socket_and_cidr() {
        let t =
            parse_targets("10.8.0.5\n 192.168.1.20:7000 \n\n10.8.0.0/24\nfe80::1\n[::1]:7000\n");
        assert_eq!(
            t,
            Ok(vec![
                "10.8.0.5".to_string(),
                "192.168.1.20:7000".into(),
                "10.8.0.0/24".into(),
                "fe80::1".into(),
                "[::1]:7000".into(),
            ])
        );
        assert_eq!(parse_targets(""), Ok(Vec::new()));
        assert_eq!(parse_targets("\n  \n"), Ok(Vec::new()));
    }

    #[test]
    fn parse_targets_reports_line_number_and_text() {
        assert_eq!(
            parse_targets("10.0.0.1\n\nnot-an-ip\n10.0.0.2"),
            Err("第 3 行无效：not-an-ip".to_string())
        );
        assert_eq!(
            parse_targets("10.0.0.1:99999"),
            Err("第 1 行无效：10.0.0.1:99999".to_string())
        );
        // 主机名不在允许范围内
        assert!(parse_targets("example.com").is_err());
    }

    #[test]
    fn device_fingerprint_handles_bad_ids() {
        assert_eq!(device_fingerprint("not-hex"), "");
        assert_eq!(device_fingerprint(&"ab".repeat(32)).len(), 26);
    }
}
