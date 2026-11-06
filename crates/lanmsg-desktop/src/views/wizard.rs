// 首次向导：填写显示名与设备名，再选择“第一台设备”或“加入已有设备”。
// 注释中文，标识符英文。

use lanmsg_api::types::InitMode;
use windui::prelude::*;

use crate::model::wizard_name_ok;
use crate::strings;
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

fn field(label: &'static str, input: Element) -> Element {
    Element::col()
        .width_match()
        .spacing(6)
        .child(
            Element::label(label)
                .font_size(13.0)
                .fg_role(Role::TextMuted),
        )
        .child(input.width_match())
}

/// 一个选项块：大按钮 + 下方一行说明。
fn choice(button: Element, hint: &'static str) -> Element {
    Element::col()
        .width_match()
        .spacing(6)
        .child(button.width_match())
        .child(
            Element::label(hint)
                .font_size(12.0)
                .fg_role(Role::TextSubtle),
        )
}

/// 首次向导页。
pub fn wizard(ui: &UiState) -> Element {
    let name = signal(String::new());
    let device = signal(default_device_name());
    let can_submit = name.map(|n| wizard_name_ok(n));
    let joining = ui.derive(|m| m.wizard_joining);
    let not_joining = joining.map(|j| !*j);

    let ui_first = ui.clone();
    let first = Element::button(strings::WIZARD_FIRST_DEVICE)
        .intent(Intent::Primary)
        .enabled_signal(can_submit)
        .on_click(move |_| {
            let (n, d) = (name.get(), device.get());
            ui_first.act(|m| m.submit_init(&n, &d, InitMode::FirstDevice));
        });
    let ui_join = ui.clone();
    let join = Element::button(strings::WIZARD_JOIN_EXISTING)
        .intent(Intent::Neutral)
        .enabled_signal(can_submit)
        .on_click(move |_| {
            let (n, d) = (name.get(), device.get());
            ui_join.act(|m| m.submit_init(&n, &d, InitMode::JoinExisting));
        });

    let form = Element::col()
        .width_match()
        .spacing(18)
        .visible_signal(not_joining)
        .child(field(
            strings::WIZARD_NAME_LABEL,
            Element::text_input(name, strings::WIZARD_NAME_PLACEHOLDER).autofocus(),
        ))
        .child(field(
            strings::WIZARD_DEVICE_LABEL,
            Element::text_input(device, strings::WIZARD_DEVICE_PLACEHOLDER),
        ))
        .child(Element::divider())
        .child(choice(first, strings::WIZARD_FIRST_DEVICE_HINT))
        .child(choice(join, strings::WIZARD_JOIN_HINT));

    // 加入流程的配对界面由 Task 14 提供；此前只提示用户到已有设备上操作
    let waiting = Element::label(strings::WIZARD_JOINING)
        .font_size(14.0)
        .fg_role(Role::TextMuted)
        .visible_signal(joining);

    let card = Element::col()
        .width(460)
        .padding(28)
        .spacing(18)
        .bg_role(Role::Surface)
        .corner(12.0)
        .child(
            Element::label(strings::WIZARD_TITLE)
                .font_size(24.0)
                .fg_role(Role::Text),
        )
        .child(
            Element::label(strings::WIZARD_SUBTITLE)
                .font_size(13.0)
                .fg_role(Role::TextMuted),
        )
        .child(form)
        .child(waiting);

    Element::stack().fill().child(card.align(Align::Center))
}
