// 首次向导：填写显示名与设备名，再选择“第一台设备”或“加入已有设备”。
// 注释中文，标识符英文。

use lanmsg_api::types::InitMode;
use windui::prelude::*;

use crate::model::wizard_name_ok;
use crate::strings;
use crate::style;
use crate::ui_state::UiState;

/// 默认设备名：主机名环境变量，其次 /etc/hostname，最后兜底文案。
fn default_device_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| strings::DEFAULT_DEVICE_NAME.to_string())
}

/// 一个表单项：标签、输入框，以及紧贴输入框下方的若干说明/校验提示。
fn field(label: &'static str, input: Element, hints: Vec<Element>) -> Element {
    let col = Element::col()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(
            Element::label(label)
                .font_size(style::FONT_BODY)
                .fg_role(Role::TextMuted),
        )
        .child(input.width_match());
    hints.into_iter().fold(col, |c, h| c.child(h))
}

/// 输入框下的说明文字（meta 字号），条件为真时才显示；`role` 区分普通说明与校验失败。
fn hint(text: &'static str, role: Role, show: Signal<bool>) -> Element {
    Element::label(text)
        .font_size(style::FONT_META)
        .fg_role(role)
        .visible_signal(show)
}

/// 一个选项块：按钮 + 下方一行说明。
fn choice(button: Element, hint: &'static str) -> Element {
    Element::col()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(button.width_match())
        .child(
            Element::label(hint)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle),
        )
}

/// 首次向导页。
pub fn wizard(ui: &UiState) -> Element {
    let name = signal(String::new());
    let device = signal(default_device_name());
    // 设备名允许留空（用默认值），非空则限 1–32 个字符
    let name_bad = name.map(|n| !n.is_empty() && !wizard_name_ok(n));
    // 名字为空时说明为什么两个按钮不可用
    let name_empty = name.map(|n| n.is_empty());
    let device_bad = device.map(|d| !d.trim().is_empty() && !wizard_name_ok(d));
    let in_flight = ui.derive(|m| m.init_in_flight);
    // 名字合法、设备名合法、且没有在途的 Init 才允许提交
    let can_submit = move || wizard_name_ok(&name.get()) && !device_bad.get() && !in_flight.get();
    let joining = ui.derive(|m| m.wizard_joining);
    let not_joining = joining.map(|j| !*j);

    let ui_first = ui.clone();
    let first = Element::button(strings::WIZARD_FIRST_DEVICE)
        .intent(Intent::Primary)
        .enabled_when(can_submit)
        .on_click(move |_| {
            let (n, d) = (name.get(), device.get());
            ui_first.act(|m| m.submit_init(&n, &d, InitMode::FirstDevice));
        });
    let ui_join = ui.clone();
    // 次按钮：surface 底 + 1px line + ink 字（outline_soft 的中性灰边 + 意图色文字）
    let join = Element::button(strings::WIZARD_JOIN_EXISTING)
        .intent(Intent::Neutral)
        .outline_soft()
        .enabled_when(can_submit)
        .on_click(move |_| {
            let (n, d) = (name.get(), device.get());
            ui_join.act(|m| m.submit_init(&n, &d, InitMode::JoinExisting));
        });

    let form = Element::col()
        .width_match()
        .spacing(style::SPACE_LG)
        .visible_signal(not_joining)
        .child(field(
            strings::WIZARD_NAME_LABEL,
            Element::text_input(name, strings::WIZARD_NAME_PLACEHOLDER).autofocus(),
            vec![
                hint(strings::WIZARD_NAME_INVALID, Role::Danger, name_bad),
                hint(
                    strings::WIZARD_NAME_REQUIRED_HINT,
                    Role::TextSubtle,
                    name_empty,
                ),
            ],
        ))
        .child(field(
            strings::WIZARD_DEVICE_LABEL,
            Element::text_input(device, strings::WIZARD_DEVICE_PLACEHOLDER),
            vec![hint(
                strings::WIZARD_DEVICE_INVALID,
                Role::Danger,
                device_bad,
            )],
        ))
        .child(Element::divider())
        .child(choice(first, strings::WIZARD_FIRST_DEVICE_HINT))
        .child(choice(join, strings::WIZARD_JOIN_HINT));

    // 加入流程的配对界面由 Task 14 提供；此前只提示用户到已有设备上操作
    let ui_back = ui.clone();
    let waiting = Element::col()
        .width_match()
        .spacing(style::SPACE_MD)
        .visible_signal(joining)
        .child(
            Element::label(strings::WIZARD_JOINING)
                .font_size(style::FONT_BODY)
                .fg_role(Role::TextMuted),
        )
        .child(
            Element::button(strings::WIZARD_BACK)
                .intent(Intent::Neutral)
                .on_click(move |_| {
                    ui_back.act(|m| m.wizard_back());
                }),
        );

    let card = Element::col()
        .width(style::WIZARD_CARD_W)
        .padding(style::SPACE_XL)
        .spacing(style::SPACE_LG)
        .bg_role(Role::Surface)
        .border_role(Role::Border, 1)
        .corner(style::RADIUS_CARD)
        .child(
            Element::label(strings::WIZARD_TITLE)
                .font_size(style::FONT_TITLE)
                .font_weight(style::WEIGHT_STRONG)
                .fg_role(Role::Text),
        )
        .child(
            Element::label(strings::WIZARD_SUBTITLE)
                .font_size(style::FONT_BODY)
                .fg_role(Role::TextMuted),
        )
        .child(form)
        .child(waiting);

    // 卡片上方：48px 应用标识 + 产品名，避免卡片在大面积底色里悬空
    let brand = Element::col()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(style::logo(48))
        .child(
            Element::label(strings::APP_TITLE)
                .font_size(style::FONT_HEADING)
                .font_weight(style::WEIGHT_STRONG)
                .fg_role(Role::Text),
        );

    let column = Element::col()
        .spacing(style::SPACE_LG)
        .cross(Align::Center)
        .child(brand)
        .child(card);
    Element::stack().fill().child(column.align(Align::Center))
}
