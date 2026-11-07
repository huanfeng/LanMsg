// “我的设备”分组：本人设备列表、移除（二次确认）与添加（扫码 / 数字比对）。
// 联系人页的“我的设备”分类与设置页共用 `devices_panel`，确认框宿主 `remove_confirm_host` 由页面放在滚动区之外。
// 注释中文，标识符英文。

use lanmsg_api::types::{DeviceView, PairMethod, PathClassDto};
use windui::prelude::*;

use crate::strings;
use crate::style;
use crate::ui_state::UiState;
use crate::views::confirm::{confirm_host, Confirm, ConfirmState};

/// 设备类型的中文名；未知类型原样显示。
pub fn device_type_label(ty: &str) -> String {
    match ty {
        "desktop" => strings::DEVICE_TYPE_DESKTOP,
        "laptop" => strings::DEVICE_TYPE_LAPTOP,
        "phone" => strings::DEVICE_TYPE_PHONE,
        "tablet" => strings::DEVICE_TYPE_TABLET,
        other => other,
    }
    .to_string()
}

/// 非局域网路径的角标文字；局域网不标。
pub fn path_label(path: &PathClassDto) -> Option<&'static str> {
    match path {
        PathClassDto::Lan => None,
        PathClassDto::Vpn => Some(strings::PATH_VPN),
        PathClassDto::Internet => Some(strings::PATH_INTERNET),
        PathClassDto::Relay => Some(strings::PATH_RELAY),
    }
}

/// 一台设备的一行：名字 + 本机标记 / 路径角标，下方类型与指纹；非本机带“移除”。
pub fn device_row(d: &DeviceView, confirm: Option<ConfirmState>) -> Element {
    let mut title = Element::row()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(
            Element::label(d.name.clone())
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End),
        );
    if d.is_self {
        title = title.child(style::pill(strings::DEVICES_THIS));
    }
    if let Some(p) = d.path.as_ref().and_then(path_label) {
        title = title.child(style::pill(p));
    }
    let status = if d.online {
        strings::DEVICES_ONLINE
    } else {
        strings::DEVICES_OFFLINE
    };
    let dot = Element::leaf().size(8, 8).corner(4.0).bg_role(if d.online {
        Role::Success
    } else {
        Role::TextSubtle
    });
    let meta = Element::label(device_type_label(&d.device_type))
        .font_size(style::FONT_META)
        .fg_role(Role::TextSubtle);
    let fingerprint = Element::label(format!(
        "{} {}",
        strings::CONTACT_FINGERPRINT,
        d.device_short
    ))
    .font_size(style::FONT_META)
    .fg_role(Role::TextSubtle);

    let mut row = Element::row()
        .width_match()
        .padding_xy(style::SPACE_LG, style::SPACE_MD)
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .child(dot)
        .child(
            Element::col()
                .weight(1.0)
                .spacing(style::SPACE_XS)
                .child(title)
                .child(meta),
        )
        .child(fingerprint)
        .child(
            Element::label(status)
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted),
        );
    if let (false, Some(confirm)) = (d.is_self, confirm) {
        let (id, name) = (d.device_id.clone(), d.name.clone());
        row = row.child(
            Element::button(strings::DEVICES_REMOVE)
                .outline_soft()
                .intent(Intent::Danger)
                .on_click(move |_| {
                    confirm.open(Confirm {
                        id: id.clone(),
                        title: strings::DEVICES_REMOVE_TITLE,
                        message: format!("{name}：{}", strings::DEVICES_REMOVE_MESSAGE),
                        action: strings::DEVICES_REMOVE,
                    })
                }),
        );
    }
    row
}

/// 移除设备的确认框宿主：放在页面最外层的 stack 里，遮罩才是全窗的。
pub fn remove_confirm_host(ui: &UiState, confirm: ConfirmState) -> Element {
    let ui = ui.clone();
    confirm_host(confirm, move |device| {
        ui.act(|m| m.remove_device(device));
    })
}

/// 设备列表 + 添加入口，不含确认框宿主；“移除”按钮把待确认的设备交给 `confirm`。
pub fn devices_panel(ui: &UiState, confirm: ConfirmState) -> Element {
    let digest = ui.digest(|m| m.devices.clone());

    let ui_list = ui.clone();
    let list = Element::switcher(digest, move |_| {
        let ds = ui_list.read(|m| m.devices.clone());
        let rows = ds.iter().enumerate().flat_map(|(i, d)| {
            let sep = (i > 0).then(Element::divider);
            sep.into_iter().chain([device_row(d, Some(confirm))])
        });
        Element::col()
            .width_match()
            .bg_role(Role::Surface)
            .border_role(Role::Border, 1)
            .corner(style::RADIUS_CARD)
            .children(rows)
    });

    let (ui_qr, ui_sas) = (ui.clone(), ui.clone());
    let in_pair = ui.derive(|m| m.pair.is_some());
    // 每个按钮下一行说明它做什么
    let caption = |text: &'static str| {
        Element::label(text)
            .font_size(style::FONT_META)
            .fg_role(Role::TextSubtle)
    };
    let actions = Element::col()
        .spacing(style::SPACE_SM)
        .child(
            Element::row()
                .spacing(style::SPACE_LG)
                .cross(Align::Start)
                .child(
                    Element::col()
                        .spacing(style::SPACE_XS)
                        .child(
                            Element::button(strings::DEVICES_ADD)
                                .enabled_when(move || !in_pair.get())
                                .on_click(move |_| ui_qr.act(|m| m.pair_start(PairMethod::Qr))),
                        )
                        .child(caption(strings::DEVICES_ADD_HINT)),
                )
                .child(
                    Element::col()
                        .spacing(style::SPACE_XS)
                        .child(
                            Element::button(strings::DEVICES_ADD_SAS)
                                .outline_soft()
                                .intent(Intent::Neutral)
                                .enabled_when(move || !in_pair.get())
                                .on_click(move |_| ui_sas.act(|m| m.pair_start(PairMethod::Sas))),
                        )
                        .child(caption(strings::DEVICES_ADD_SAS_HINT)),
                ),
        )
        .child(caption(strings::DEVICES_ADD_BUSY).visible_signal(in_pair));

    Element::col()
        .width_match()
        .spacing(style::SPACE_MD)
        .child(list)
        .child(actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_type_label_maps_known_and_keeps_unknown() {
        assert_eq!(device_type_label("phone"), "手机");
        assert_eq!(device_type_label("localsend"), "localsend");
    }

    #[test]
    fn path_label_hides_lan_only() {
        assert_eq!(path_label(&PathClassDto::Lan), None);
        assert_eq!(path_label(&PathClassDto::Vpn), Some(strings::PATH_VPN));
    }
}
