// 外壳：左侧图标栏 + 顶部连接状态条 + 右侧页面内容区。
// 注释中文，标识符英文。

use windui::prelude::*;

use crate::model::Page;
use crate::strings;
use crate::style;
use crate::ui_state::UiState;
use crate::views::{
    chats, contacts, files, pair, preview, settings, stations, web_transfer, wizard,
};

/// 页面内容。
fn page_view(ui: &UiState, p: Page) -> Element {
    match p {
        Page::Wizard => wizard::wizard(ui),
        Page::Chats => chats::chats_page(ui),
        Page::Contacts => contacts::contacts_page(ui),
        Page::Files => files::files_page(ui),
        Page::Stations => stations::stations_page(ui),
        Page::Settings => settings::settings_page(ui),
    }
}

/// 图标栏一个页签：20px 图标 + 下方 12px 文字；选中时整块 `signal_soft` 底、图标与文字用 `signal`；
/// 角标叠在图标右上角。
fn nav_item(
    ui: &UiState,
    page: Page,
    icon_svg: &'static [u8],
    text: &'static str,
    badge: Option<Signal<String>>,
) -> Element {
    let selected = ui.derive(move |m| m.page == page);
    let idle = selected.map(|s| !*s);
    let role = selected.map(|s| {
        if *s {
            Role::AccentText
        } else {
            Role::TextMuted
        }
    });
    let ui_click = ui.clone();

    // 图标区 48x30：图标水平居中、贴底（上方 10px 留给角标），角标贴右上、与图标仅在角上相接
    let mut holder = Element::stack()
        .size(style::NAV_ICON_BOX_W, style::NAV_ICON_BOX_H)
        .child(
            style::icon(icon_svg, style::ICON_MD, Role::TextMuted)
                .align_xy(Align::Center, Align::End)
                .visible_signal(idle),
        )
        .child(
            style::icon(icon_svg, style::ICON_MD, Role::AccentText)
                .align_xy(Align::Center, Align::End)
                .visible_signal(selected),
        );
    if let Some(b) = badge {
        let show = b.map(|s| !s.is_empty());
        holder = holder.child(
            style::badge(b)
                .align_xy(Align::End, Align::Start)
                .visible_signal(show),
        );
    }

    let block = Element::col()
        .size(style::NAV_ITEM_W, style::NAV_ITEM_H)
        .padding_xy(0, style::SPACE_SM)
        .cross(Align::Center)
        .corner(style::RADIUS_NAV_ITEM)
        .bg_when(move || {
            if selected.get() {
                style::tokens().signal_soft
            } else {
                Color::TRANSPARENT
            }
        })
        .clickable()
        .on_click(move |_| ui_click.act(|m| m.set_page(page)))
        .child(holder)
        .child(
            Element::label(text)
                .font_size(style::FONT_META)
                .fg_role_signal(role),
        )
        .align(Align::Center);
    // 选中指示条：signal_soft 在 rail 上只差一线，再加左缘 3px signal 竖条；指示条在块之外，不挡点击
    let bar = Element::leaf()
        .size(style::NAV_BAR_W, style::NAV_BAR_H)
        .corner(style::NAV_BAR_W as f32 / 2.0)
        .bg_role(Role::Accent)
        .align_xy(Align::Start, Align::Center)
        .visible_signal(selected);
    Element::stack()
        .size(style::NAV_SLOT_W, style::NAV_ITEM_H)
        .child(bar)
        .child(block)
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

    // 图标栏：rail 底，顶部应用标识，右侧 1px 分隔线
    let rail = Element::col()
        .width(style::RAIL_WIDTH)
        .height_match()
        .bg_when(|| style::tokens().rail)
        .padding_edges(Insets {
            top: style::SPACE_LG,
            bottom: style::SPACE_MD,
            left: 0,
            right: 0,
        })
        .spacing(style::SPACE_XS)
        .cross(Align::Center)
        .child(style::logo(style::LOGO_SIZE))
        .child(Element::leaf().height(style::SPACE_MD))
        .child(nav_item(
            ui,
            Page::Chats,
            style::ICON_CHATS,
            strings::NAV_CHATS,
            None,
        ))
        .child(nav_item(
            ui,
            Page::Contacts,
            style::ICON_CONTACTS,
            strings::NAV_CONTACTS,
            Some(pending),
        ))
        .child(nav_item(
            ui,
            Page::Files,
            style::ICON_FILES,
            strings::NAV_FILES,
            None,
        ))
        .child(nav_item(
            ui,
            Page::Stations,
            style::ICON_STATIONS,
            strings::NAV_STATIONS,
            None,
        ))
        .child(nav_item(
            ui,
            Page::Settings,
            style::ICON_SETTINGS,
            strings::NAV_SETTINGS,
            None,
        ));
    let nav = Element::row()
        .height_match()
        .visible_signal(not_wizard)
        .child(rail)
        .child(style::vline());

    let ui_start = ui.clone();
    let banner = Element::row()
        .width_match()
        .padding_xy(style::SPACE_LG, style::SPACE_SM)
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .bg_role(Role::Warning)
        .visible_signal(disconnected)
        .child(
            Element::label_signal(ui.banner_text())
                .font_size(style::FONT_BODY)
                .fg_role(Role::OnAccent),
        )
        .child(
            Element::button(strings::DAEMON_START_BUTTON)
                .on_click(move |_| ui_start.start_daemon())
                .visible_signal(ui.daemon_quit()),
        );

    let ui_page = ui.clone();
    let content = Element::col()
        .weight(1.0)
        .height_match()
        .child(banner)
        .child(Element::switcher(page, move |p| page_view(&ui_page, p).fill()).weight(1.0));

    let main = Element::row()
        .fill()
        .bg_role(Role::Bg)
        .child(nav)
        .child(content);
    // 配对对话框在最上层：向导（加入流程）与主界面共用
    Element::stack()
        .fill()
        .child(main)
        .child(pair::pair_host(ui))
        .child(web_transfer::web_transfer_host(ui))
        .child(preview::preview_host(ui))
}
