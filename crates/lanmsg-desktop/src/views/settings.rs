// 设置页：分组卡片 + 表单行。
// 个人、我的设备、下载与存储、网络、通用、关于；“个人”“下载与存储”“网络”三张卡片各有自己的“保存”，
// 只提交本组字段（SetSettings 以当前已保存的值为底，只替换本组字段），某组非法不影响其他组。
// 本组没有修改或有非法内容时保存按钮禁用，旁边说明原因。开机自启开关与“我的设备”即时生效。
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
/// 自动下载上限可填的 MB 范围；0 表示不自动下载。
const MB_RANGE: std::ops::RangeInclusive<u64> = 0..=10_240;

// ---------------------------------------------------------------------------
// 纯函数
// ---------------------------------------------------------------------------

/// 解析“MB”输入：0–10240 的整数（0 = 不自动下载），返回字节数；其余（空、负数、小数、越界）为 None。
pub fn parse_mb(s: &str) -> Option<u64> {
    let mb: u64 = s.trim().parse().ok()?;
    MB_RANGE.contains(&mb).then(|| mb * MIB)
}

/// 解析某个 MB 字段：文字与已保存值的显示一致（用户没动过）时原样沿用已保存的字节数，不做范围校验——
/// 这样 Android 端写入的“不限”（u64::MAX）等超出 10240 的值不会让整组永远无法保存；
/// 用户改过才按 `parse_mb` 校验。
pub fn resolve_mb(text: &str, saved: u64) -> Option<u64> {
    if text.trim() == mb_text(saved) {
        Some(saved)
    } else {
        parse_mb(text)
    }
}

/// 一组设置的保存按钮状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupState {
    /// 没有未保存的修改。
    Clean,
    /// 有修改但有内容不符合要求。
    Invalid,
    /// 有可保存的修改。
    Dirty,
    /// 有一次保存还没收到回复，暂不能再保存。
    Saving,
}

/// 在途保存期间所有组一律按 `Saving` 显示（保存按钮置灰，说明“正在保存”）。
pub fn with_saving(state: GroupState, saving: bool) -> GroupState {
    if saving {
        GroupState::Saving
    } else {
        state
    }
}

/// 个人组：在已保存设置上只替换设备名。
pub fn personal_dto(base: &SettingsDto, device: &str) -> SettingsDto {
    let mut dto = base.clone();
    dto.device_name = device.trim().to_string();
    dto
}

/// 下载与存储组：只替换下载目录、两个自动下载上限、仅 Wi-Fi 开关；任一字段非法返回 None。
pub fn storage_dto(
    base: &SettingsDto,
    dir: &str,
    image_mb: &str,
    file_mb: &str,
    wifi_only: bool,
) -> Option<SettingsDto> {
    let dir = dir.trim();
    if dir.is_empty() {
        return None;
    }
    let mut dto = base.clone();
    dto.download_dir = dir.to_string();
    dto.auto_image_max = resolve_mb(image_mb, base.auto_image_max)?;
    dto.auto_file_max = resolve_mb(file_mb, base.auto_file_max)?;
    dto.auto_file_wifi_only = wifi_only;
    Some(dto)
}

/// 网络组：只替换 LocalSend 开关与单播通告目标；目标有非法行返回 None。
pub fn network_dto(base: &SettingsDto, localsend: bool, targets: &str) -> Option<SettingsDto> {
    let mut dto = base.clone();
    dto.localsend_enabled = localsend;
    dto.unicast_targets = parse_targets(targets).ok()?;
    Some(dto)
}

/// 由“能否生成本组 DTO”和“与已保存值是否不同”得出按钮状态。
/// 生成不出 DTO（有非法内容）时只可能是用户改过，视为 Invalid。
pub fn group_state(dto: Option<&SettingsDto>, base: &SettingsDto, extra_dirty: bool) -> GroupState {
    match dto {
        None => GroupState::Invalid,
        Some(d) if d != base || extra_dirty => GroupState::Dirty,
        Some(_) => GroupState::Clean,
    }
}

/// 字节数换成输入框里的 MB 文字（向下取整；非整 MB 的旧值保存时会按整 MB 写回）。
/// 超出可填范围（如 Android 的“不限”）显示占位“不限”，`resolve_mb` 见到它视为没改、保留原值。
fn mb_text(bytes: u64) -> String {
    let mb = bytes / MIB;
    if MB_RANGE.contains(&mb) {
        mb.to_string()
    } else {
        strings::SETTINGS_MB_UNLIMITED.to_string()
    }
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
        // 规则与 lanmsg-core 的 parse_unicast_targets 一致：IP、IP:端口（端口非 0）、
        // 前缀不小于 /24 的 IPv4 网段（IPv6 网段不接受）。
        let ok = line.parse::<IpAddr>().is_ok()
            || line.parse::<SocketAddr>().is_ok_and(|a| a.port() != 0)
            || matches!(line.parse::<IpNet>(), Ok(IpNet::V4(n)) if n.prefix_len() >= 24);
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

/// 分组卡片底部右侧的“保存”：只有本组有可保存的修改时可点；不可点时旁边说明原因。
fn group_footer(
    state: impl Fn() -> GroupState + Copy + 'static,
    on_save: impl Fn(&mut windui::core::EventCtx) + 'static,
) -> Element {
    let hint = |text: &'static str, role: Role, shown: GroupState| {
        Element::label(text)
            .font_size(style::FONT_META)
            .fg_role(role)
            .visible_when(move || state() == shown)
    };
    Element::row()
        .width_match()
        .padding_xy(style::SPACE_LG, style::SPACE_MD)
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .child(Element::leaf().weight(1.0))
        .child(hint(
            strings::SETTINGS_SAVE_CLEAN,
            Role::TextSubtle,
            GroupState::Clean,
        ))
        .child(hint(
            strings::SETTINGS_SAVE_BLOCKED,
            Role::Danger,
            GroupState::Invalid,
        ))
        .child(hint(
            strings::SETTINGS_SAVING,
            Role::TextSubtle,
            GroupState::Saving,
        ))
        .child(
            Element::button(strings::SETTINGS_SAVE)
                .enabled_when(move || state() == GroupState::Dirty)
                .on_click(on_save),
        )
}

/// 设置表单。控件的草稿值放在本函数创建的信号里，只在设置首次到达时创建一次；
/// 保存后重新读到的设置只更新“已保存值”，其他组尚未保存的草稿不会被冲掉。
fn form(ui: &UiState, init: SettingsDto, remove_confirm: ConfirmState) -> Element {
    // 已保存的值：随 Model 刷新；表单只在设置存在时才建，取不到时各组按“没有修改”处理
    let saved = ui.derive(|m| m.settings.clone());
    let saved_name = ui.derive(|m| {
        m.me.as_ref()
            .and_then(|me| me.display_name.clone())
            .unwrap_or_default()
    });
    let display = signal(saved_name.get());
    let device = signal(init.device_name.clone());
    let dir = signal(init.download_dir.clone());
    let image_mb = signal(mb_text(init.auto_image_max));
    let file_mb = signal(mb_text(init.auto_file_max));
    let wifi_only = signal(init.auto_file_wifi_only);
    let localsend = signal(init.localsend_enabled);
    let targets = signal(init.unicast_targets.join("\n"));
    let auto_start = signal(autostart::is_enabled());

    // 校验：每项一个派生信号
    let display_ok = display.map(|v| wizard_name_ok(v));
    let device_ok = device.map(|v| wizard_name_ok(v));
    let dir_ok = dir.map(|v| !v.trim().is_empty());
    // MB 字段的合法性要看已保存值：没动过的超大原值算合法
    let image_ok = {
        let saved_image = saved.map(|s| s.as_ref().map_or(0, |s| s.auto_image_max));
        image_mb.map(move |v| resolve_mb(v, saved_image.get()).is_some())
    };
    let file_ok = {
        let saved_file = saved.map(|s| s.as_ref().map_or(0, |s| s.auto_file_max));
        file_mb.map(move |v| resolve_mb(v, saved_file.get()).is_some())
    };
    let targets_err = targets.map(|v| parse_targets(v).err().unwrap_or_default());
    let targets_ok = targets_err.map(|e| e.is_empty());
    let not = |ok: Signal<bool>| ok.map(|v| !*v);
    let saving = ui.derive(|m| m.settings_saving > 0);

    // 各组按钮状态
    let personal_state = move || {
        with_saving(
            match saved.get() {
                None => GroupState::Clean,
                Some(b) => {
                    if !display_ok.get() || !device_ok.get() {
                        return GroupState::Invalid;
                    }
                    let name_changed = display.get().trim() != saved_name.get();
                    group_state(Some(&personal_dto(&b, &device.get())), &b, name_changed)
                }
            },
            saving.get(),
        )
    };
    let storage_state = move || {
        with_saving(
            match saved.get() {
                None => GroupState::Clean,
                Some(b) => group_state(
                    storage_dto(
                        &b,
                        &dir.get(),
                        &image_mb.get(),
                        &file_mb.get(),
                        wifi_only.get(),
                    )
                    .as_ref(),
                    &b,
                    false,
                ),
            },
            saving.get(),
        )
    };
    let network_state = move || {
        with_saving(
            match saved.get() {
                None => GroupState::Clean,
                Some(b) => group_state(
                    network_dto(&b, localsend.get(), &targets.get()).as_ref(),
                    &b,
                    false,
                ),
            },
            saving.get(),
        )
    };

    let ui_personal = ui.clone();
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
        group_footer(personal_state, move |_| {
            let Some(b) = saved.get() else { return };
            let dto = personal_dto(&b, &device.get());
            let name = display.get();
            ui_personal.act(|m| m.save_settings(dto, &name));
        }),
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
    let ui_storage = ui.clone();
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
        switch_row(
            strings::SETTINGS_WIFI_ONLY,
            strings::SETTINGS_WIFI_ONLY_HINT,
            Element::switch(wifi_only),
        ),
        group_footer(storage_state, move |_| {
            let Some(b) = saved.get() else { return };
            if let Some(dto) = storage_dto(
                &b,
                &dir.get(),
                &image_mb.get(),
                &file_mb.get(),
                wifi_only.get(),
            ) {
                // 显示名不属于本组：传当前已保存的名字，Model 不会发 SetProfile
                let name = saved_name.get();
                ui_storage.act(|m| m.save_settings(dto, &name));
            }
        }),
    ]);

    let ui_network = ui.clone();
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
        group_footer(network_state, move |_| {
            let Some(b) = saved.get() else { return };
            if let Some(dto) = network_dto(&b, localsend.get(), &targets.get()) {
                let name = saved_name.get();
                ui_network.act(|m| m.save_settings(dto, &name));
            }
        }),
    ]);

    let data_dir = ui.data_dir().to_path_buf();
    let me = ui.read(|m| m.me.clone());
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
        .child(group(strings::SETTINGS_GROUP_GENERAL, general))
        .child(group(strings::SETTINGS_GROUP_ABOUT, about))
}

/// 设置页：设置到达前显示读取提示；到达后建一次表单，之后的刷新只更新“已保存值”。
/// 移除设备的确认框放在滚动区之外的最上层，遮罩才是全窗的。
pub fn settings_page(ui: &UiState) -> Element {
    let arrived = ui.derive(|m| m.settings.is_some());
    let remove_confirm = ConfirmState::new();
    let ui_form = ui.clone();
    let body = Element::switcher(arrived, move |_| {
        match ui_form.read(|m| m.settings.clone()) {
            Some(s) => form(&ui_form, s, remove_confirm),
            None => Element::label(strings::SETTINGS_LOADING)
                .font_size(style::FONT_BODY)
                .fg_role(Role::TextMuted)
                .margin(style::SPACE_XL),
        }
    });
    Element::stack()
        .fill()
        .bg_role(Role::Bg)
        .child(
            // 右侧留出滚动条槽，滚动条不贴窗口边
            Element::col()
                .fill()
                .padding_edges(Insets {
                    top: 0,
                    bottom: 0,
                    left: 0,
                    right: style::SPACE_XS,
                })
                .child(Element::scroll().fill().child(body)),
        )
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
        assert_eq!(parse_mb("0"), Some(0));
        assert_eq!(parse_mb(" 0 "), Some(0));
        assert_eq!(parse_mb("10241"), None);
        assert_eq!(parse_mb(""), None);
        assert_eq!(parse_mb("-5"), None);
        assert_eq!(parse_mb("1.5"), None);
        assert_eq!(parse_mb("abc"), None);
        assert_eq!(parse_mb("99999999999999999999"), None);
    }

    fn dto() -> SettingsDto {
        SettingsDto {
            device_name: "旧设备".into(),
            download_dir: "/d".into(),
            auto_image_max: 10 * MIB,
            auto_file_max: 50 * MIB,
            auto_file_wifi_only: true,
            localsend_enabled: true,
            unicast_targets: vec!["10.0.0.1".into()],
            background_mode: lanmsg_api::types::BackgroundMode::Realtime,
            localsend_listening: None,
        }
    }

    #[test]
    fn resolve_mb_keeps_untouched_huge_value_and_validates_edits() {
        // Android 的“不限”= u64::MAX：没动过就沿用，不阻塞保存
        let unlimited = u64::MAX;
        assert_eq!(resolve_mb(&mb_text(unlimited), unlimited), Some(unlimited));
        // 改过则按范围校验
        assert_eq!(resolve_mb("20000", unlimited), None);
        assert_eq!(resolve_mb("0", unlimited), Some(0));
        assert_eq!(resolve_mb("100", unlimited), Some(100 * MIB));
        // 非整 MB 的旧值，文字没变时字节数原样保留
        assert_eq!(resolve_mb("1", MIB + 5), Some(MIB + 5));
    }

    #[test]
    fn mb_text_shows_unlimited_placeholder_beyond_range() {
        assert_eq!(mb_text(50 * MIB), "50");
        assert_eq!(mb_text(10_240 * MIB), "10240");
        assert_eq!(mb_text(10_241 * MIB), strings::SETTINGS_MB_UNLIMITED);
        assert_eq!(mb_text(u64::MAX), strings::SETTINGS_MB_UNLIMITED);
        // 文字等于占位视为没改，原值（含超大值）保留
        let ph = strings::SETTINGS_MB_UNLIMITED;
        assert_eq!(resolve_mb(ph, u64::MAX), Some(u64::MAX));
        assert_eq!(
            resolve_mb(&format!(" {ph} "), 10_241 * MIB),
            Some(10_241 * MIB)
        );
        // 已保存值在范围内时，占位文字不是合法输入
        assert_eq!(resolve_mb(ph, 5 * MIB), None);
        // 改成数字才按范围校验
        assert_eq!(resolve_mb("20000", u64::MAX), None);
    }

    #[test]
    fn saving_overrides_group_state() {
        for s in [GroupState::Clean, GroupState::Invalid, GroupState::Dirty] {
            assert_eq!(with_saving(s, true), GroupState::Saving);
            assert_eq!(with_saving(s, false), s);
        }
    }

    #[test]
    fn storage_group_saves_with_huge_value_and_zero() {
        let mut base = dto();
        base.auto_file_max = u64::MAX;
        let file = mb_text(u64::MAX);
        let d = storage_dto(&base, "/d", "0", &file, true).expect("合法");
        assert_eq!(d.auto_image_max, 0);
        assert_eq!(d.auto_file_max, u64::MAX);
        assert_eq!(group_state(Some(&d), &base, false), GroupState::Dirty);
        // 全部没改 -> 没有修改
        let same = storage_dto(&base, "/d", &mb_text(base.auto_image_max), &file, true);
        assert_eq!(group_state(same.as_ref(), &base, false), GroupState::Clean);
        // 非法 -> Invalid
        let bad = storage_dto(&base, "/d", "abc", &file, true);
        assert_eq!(group_state(bad.as_ref(), &base, false), GroupState::Invalid);
        assert!(storage_dto(&base, "  ", "1", "1", true).is_none());
    }

    #[test]
    fn group_dtos_only_touch_their_own_fields() {
        let base = dto();
        let p = personal_dto(&base, " 新设备 ");
        assert_eq!(p.device_name, "新设备");
        assert_eq!(
            SettingsDto {
                device_name: base.device_name.clone(),
                ..p
            },
            base
        );

        let s = storage_dto(&base, " /new ", "5", "6", false).expect("合法");
        assert_eq!(s.download_dir, "/new");
        assert_eq!((s.auto_image_max, s.auto_file_max), (5 * MIB, 6 * MIB));
        assert!(!s.auto_file_wifi_only);
        assert_eq!(s.device_name, base.device_name);
        assert_eq!(s.localsend_enabled, base.localsend_enabled);
        assert_eq!(s.unicast_targets, base.unicast_targets);

        let n = network_dto(&base, false, "10.1.1.1\n10.2.0.0/24").expect("合法");
        assert!(!n.localsend_enabled);
        assert_eq!(n.unicast_targets, vec!["10.1.1.1", "10.2.0.0/24"]);
        assert_eq!(n.download_dir, base.download_dir);
        assert_eq!(n.auto_file_max, base.auto_file_max);
        assert_eq!(n.device_name, base.device_name);
        assert!(network_dto(&base, true, "bad").is_none());
    }

    #[test]
    fn group_state_distinguishes_clean_dirty_invalid() {
        let base = dto();
        assert_eq!(group_state(Some(&base), &base, false), GroupState::Clean);
        // 显示名改了但 DTO 没变（个人组）
        assert_eq!(group_state(Some(&base), &base, true), GroupState::Dirty);
        assert_eq!(group_state(None, &base, false), GroupState::Invalid);
        let n = network_dto(&base, true, "10.0.0.1").expect("合法");
        assert_eq!(group_state(Some(&n), &base, false), GroupState::Clean);
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
        // 与 core 的 parse_unicast_targets 同一套规则：网段不得小于 /24、不接受 IPv6 网段与端口 0。
        assert!(parse_targets("10.0.0.0/16").is_err());
        assert!(parse_targets("fd00::/120").is_err());
        assert!(parse_targets("10.0.0.1:0").is_err());
    }

    #[test]
    fn device_fingerprint_handles_bad_ids() {
        assert_eq!(device_fingerprint("not-hex"), "");
        assert_eq!(device_fingerprint(&"ab".repeat(32)).len(), 26);
    }
}
