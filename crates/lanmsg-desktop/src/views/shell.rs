// 外壳：左侧图标栏 + 顶部连接状态条 + 右侧页面内容区。
// 各页内容在本任务先放标题占位，由后续任务替换。
// 注释中文，标识符英文。

use windui::prelude::*;

use crate::model::Page;
use crate::strings;
use crate::ui_state::UiState;
use crate::views::{chats, wizard};

/// 页面内容；尚未实现的页先放标题占位。
fn page_view(ui: &UiState, p: Page) -> Element {
    let title = match p {
        Page::Wizard => return wizard::wizard(ui),
        Page::Chats => return chats::chats_page(ui),
        Page::Contacts => strings::PAGE_CONTACTS,
        Page::Files => strings::PAGE_FILES,
        Page::Settings => strings::PAGE_SETTINGS,
    };
    Element::col()
        .fill()
        .padding(24)
        .child(Element::label(title).font_size(22.0).fg_role(Role::Text))
}

/// 侧栏一个页签：选中时文字用强调色。
fn nav_item(
    ui: &UiState,
    page: Page,
    text: &'static str,
    badge: Option<Signal<String>>,
) -> Element {
    let role = ui.derive(move |m| {
        if m.page == page {
            Role::AccentText
        } else {
            Role::TextMuted
        }
    });
    let ui_click = ui.clone();
    let mut item = Element::col()
        .width_match()
        .padding_xy(4, 10)
        .spacing(4)
        .cross(Align::Center)
        .clickable()
        .on_click(move |_| ui_click.act(|m| m.set_page(page)))
        .child(Element::label(text).font_size(13.0).fg_role_signal(role));
    if let Some(b) = badge {
        let show = b.map(|s| !s.is_empty());
        item = item.child(Element::badge(b).visible_signal(show));
    }
    item
}

/// 顶层界面。
pub fn shell(ui: &UiState) -> Element {
    let page = ui.derive(|m| m.page);
    let in_wizard = ui.derive(|m| m.page == Page::Wizard);
    let not_wizard = in_wizard.map(|w| !*w);
    let disconnected = ui.disconnected();
    // 未处理请求数：0 时为空串，徽章随之隐藏
    let pending = ui.derive(|m| match m.requests.len() {
        0 => String::new(),
        n => n.to_string(),
    });

    let nav = Element::col()
        .width(76)
        .height_match()
        .bg_role(Role::Surface)
        .padding_xy(4, 12)
        .spacing(6)
        .visible_signal(not_wizard)
        .child(nav_item(ui, Page::Chats, strings::NAV_CHATS, None))
        .child(nav_item(
            ui,
            Page::Contacts,
            strings::NAV_CONTACTS,
            Some(pending),
        ))
        .child(nav_item(ui, Page::Files, strings::NAV_FILES, None))
        .child(nav_item(ui, Page::Settings, strings::NAV_SETTINGS, None));

    let banner = Element::row()
        .width_match()
        .padding_xy(16, 8)
        .bg_role(Role::Warning)
        .visible_signal(disconnected)
        .child(
            Element::label(strings::DISCONNECTED_BANNER)
                .font_size(13.0)
                .fg_role(Role::OnAccent),
        );

    let ui_page = ui.clone();
    let content = Element::col()
        .weight(1.0)
        .height_match()
        .child(banner)
        .child(Element::switcher(page, move |p| page_view(&ui_page, p).fill()).weight(1.0));

    Element::row()
        .fill()
        .bg_role(Role::Bg)
        .child(nav)
        .child(content)
}
