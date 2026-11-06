// “转发到”对话框：选一个私聊或“我的设备”会话，把文件消息里的文件转发过去。
// 文件视图页（Task 15）复用 `forward_dialog` / `forward_host`。注释中文，标识符英文。

use lanmsg_api::types::{ConvKind, ConversationSummary};
use windui::prelude::*;

use crate::strings;
use crate::style;
use crate::ui_state::UiState;
use crate::views::chats::avatar;

/// 尚未选择会话。
const NONE: usize = usize::MAX;

/// 对话框宿主：随 `ui.forward` 重建；放在一个 stack 的最上层即可。
pub fn forward_host(ui: &UiState) -> Element {
    let ui = ui.clone();
    Element::switcher(ui.forward, move |(_, hashes)| forward_dialog(&ui, hashes))
}

/// 可转发的会话：私聊与“我的设备”。
fn targets(ui: &UiState) -> Vec<ConversationSummary> {
    ui.read(|m| {
        m.ordered_convs()
            .into_iter()
            .filter(|c| matches!(c.kind, ConvKind::SelfDevices | ConvKind::Dm))
            .cloned()
            .collect()
    })
}

/// 转发对话框；`hashes` 为空时返回一个不占位的空元素（对话框已关闭）。
pub fn forward_dialog(ui: &UiState, hashes: Vec<String>) -> Element {
    if hashes.is_empty() {
        return Element::leaf().size(0, 0);
    }
    let convs = targets(ui);
    let selected = signal(NONE);
    let show = signal(true);

    let rows = convs.iter().enumerate().map(|(i, c)| {
        Element::row()
            .width_match()
            .height(48)
            .padding_xy(style::SPACE_MD, 0)
            .spacing(style::SPACE_MD)
            .cross(Align::Center)
            .corner(style::RADIUS_CONTROL)
            .bg_when(move || {
                if selected.get() == i {
                    style::tokens().signal_soft
                } else {
                    Color::TRANSPARENT
                }
            })
            .clickable()
            .on_click(move |_| selected.set(i))
            .child(avatar(&c.conv, &c.title, c.peer_online))
            .child(
                Element::label(c.title.clone())
                    .font_size(style::FONT_BODY)
                    .fg_role(Role::Text)
                    .max_lines(1)
                    .truncate(Truncate::End)
                    .weight(1.0),
            )
    });
    let list = if convs.is_empty() {
        Element::label(strings::FORWARD_EMPTY)
            .font_size(style::FONT_BODY)
            .fg_role(Role::TextMuted)
    } else {
        Element::scroll()
            .width_match()
            .max_height(288)
            .child(Element::col().width_match().children(rows))
    };
    let none_selected = selected.map(|s| *s == NONE);
    let body = Element::col()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(list)
        .child(
            Element::label(strings::FORWARD_PICK_HINT)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle)
                .visible_signal(none_selected),
        );

    let ui_cancel = ui.clone();
    let ui_send = ui.clone();
    let ui_close = ui.clone();
    let footer = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(Element::flex_spacer())
        .child(
            Element::button(strings::FORWARD_CANCEL)
                .outline()
                .on_click(move |_| ui_cancel.close_forward()),
        )
        .child(
            Element::button(strings::FORWARD_SEND)
                .enabled_when(move || selected.get() != NONE)
                .on_click(move |_| {
                    if let Some(c) = convs.get(selected.get()) {
                        let (conv, hs) = (c.conv.clone(), hashes.clone());
                        ui_send.act(|m| m.forward_files(conv, hs));
                        ui_send.close_forward();
                    }
                }),
        );
    Element::dialog_panel(
        show,
        strings::FORWARD_TITLE,
        360,
        move |_| ui_close.close_forward(),
        body,
        footer,
    )
}
