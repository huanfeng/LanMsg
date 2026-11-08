// 联系人页：左侧分类导航（联系人 / 新的请求 / 附近的人 / 我的设备），右侧为该类内容。
// 注释中文，标识符英文。

use lanmsg_api::types::{ContactView, PeerView, RequestAction, RequestKind, RequestView};
use windui::prelude::*;

use crate::event_text::request_preview_text;
use crate::model::{normalize_peer_addr, parse_ask_mb, CONTACTS_TABS};
use crate::strings;
use crate::style;
use crate::ui_state::UiState;
use crate::views::chats::avatar;
use crate::views::confirm::{confirm_host, Confirm, ConfirmState};
use crate::views::devices::{device_row, devices_panel, remove_confirm_host};
use crate::views::network::{path_badge_text, path_rank};

/// 分类导航栏宽度。
const NAV_W: i32 = 200;
/// 导航项高度。
const NAV_ROW_H: i32 = 40;
/// 内容区最大宽度：宽窗口下卡片不无限拉长。
const CONTENT_MAX_W: i32 = 760;

/// “附近的人”一项：同一用户的多台设备合并。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Nearby {
    pub user_id: String,
    /// LocalSend 设备没有用户身份（user_id 为空），用伪设备 ID 打开会话。
    pub device_id: String,
    pub name: String,
    /// 设备名列表。
    pub devices: Vec<String>,
    /// 首台设备的指纹（PeerView 没有用户指纹字段）。
    pub short: String,
    /// 最优路径的角标；局域网不标。
    pub badge: Option<&'static str>,
}

/// 从在线设备里挑出还不是联系人、也不是自己设备的人，按用户合并。
pub fn nearby_people(peers: &[PeerView]) -> Vec<Nearby> {
    let mut out: Vec<(Nearby, u8)> = Vec::new();
    for p in peers.iter().filter(|p| !p.is_contact && !p.is_mine) {
        let rank = path_rank(&p.path);
        let local_send = p.user_id.is_empty();
        let same = |n: &Nearby| {
            if local_send {
                n.user_id.is_empty() && n.device_id == p.device_id
            } else {
                n.user_id == p.user_id
            }
        };
        match out.iter_mut().find(|(n, _)| same(n)) {
            Some((n, best)) => {
                n.devices.push(p.device_name.clone());
                if rank < *best {
                    *best = rank;
                    n.badge = path_badge_text(&p.path);
                }
            }
            None => out.push((
                Nearby {
                    user_id: p.user_id.clone(),
                    device_id: p.device_id.clone(),
                    name: p.display_name.clone(),
                    devices: vec![p.device_name.clone()],
                    short: p.device_short.clone(),
                    badge: path_badge_text(&p.path),
                },
                rank,
            )),
        }
    }
    out.into_iter().map(|(n, _)| n).collect()
}

/// 卡片外壳：surface 底 + 1px 描边 + 卡片圆角。
fn card() -> Element {
    Element::col()
        .width_match()
        .bg_role(Role::Surface)
        .border_role(Role::Border, 1)
        .corner(style::RADIUS_CARD)
}

fn page_title(text: &'static str) -> Element {
    Element::label(text)
        .font_size(style::FONT_TITLE)
        .font_weight(style::WEIGHT_STRONG)
        .fg_role(Role::Text)
}

fn meta(text: impl Into<TextContent>) -> Element {
    Element::label(text)
        .font_size(style::FONT_META)
        .fg_role(Role::TextSubtle)
        .max_lines(1)
        .truncate(Truncate::End)
}

fn name_label(text: String) -> Element {
    Element::label(text)
        .font_size(style::FONT_BODY)
        .font_weight(style::WEIGHT_STRONG)
        .fg_role(Role::Text)
        .max_lines(1)
        .truncate(Truncate::End)
}

// ---------------------------------------------------------------------------
// 左侧分类导航
// ---------------------------------------------------------------------------

fn nav_row(ui: &UiState, index: usize, label: &'static str) -> Element {
    let tab = ui.derive(|m| m.contacts_tab);
    let count = ui.derive(move |m| match index {
        0 => m.contacts.len(),
        1 => m.requests.len(),
        2 => nearby_people(&m.peers).len(),
        _ => m.devices.len(),
    });
    let text = count.map(|n| {
        if *n == 0 {
            String::new()
        } else {
            n.to_string()
        }
    });
    let has_count = count.map(|n| *n > 0);
    let ui_click = ui.clone();
    // 请求数用强调徽章，其余用静音数字
    let trailing = if index == 1 {
        style::badge(text).visible_signal(has_count)
    } else {
        Element::label(text)
            .font_size(style::FONT_META)
            .fg_role(Role::TextSubtle)
    };
    Element::row()
        .width_match()
        .height(NAV_ROW_H)
        .padding_xy(style::SPACE_MD, 0)
        .cross(Align::Center)
        .corner(style::RADIUS_NAV_ITEM)
        .bg_when(move || {
            if tab.get() == index {
                style::tokens().signal_soft
            } else {
                Color::TRANSPARENT
            }
        })
        .clickable()
        .on_click(move |_| ui_click.act(|m| m.set_contacts_tab(index)))
        .child(
            Element::label(label)
                .font_size(style::FONT_BODY)
                .fg_role_signal(tab.map(move |t| {
                    if *t == index {
                        Role::AccentText
                    } else {
                        Role::Text
                    }
                }))
                .weight(1.0),
        )
        .child(trailing)
}

fn side_nav(ui: &UiState) -> Element {
    Element::col()
        .width(NAV_W)
        .height_match()
        .bg_role(Role::Surface)
        .padding(style::SPACE_MD)
        .spacing(style::SPACE_XS)
        .child(nav_row(ui, 0, strings::TAB_CONTACTS))
        .child(nav_row(ui, 1, strings::TAB_REQUESTS))
        .child(nav_row(ui, 2, strings::TAB_NEARBY))
        .child(nav_row(ui, 3, strings::TAB_DEVICES))
}

// ---------------------------------------------------------------------------
// 联系人
// ---------------------------------------------------------------------------

/// 联系人卡片：点击头部展开设备、自动接收策略与“发消息”。
fn contact_card(ui: &UiState, c: &ContactView, expanded: Signal<String>) -> Element {
    let id = c.user_id.clone();
    let open = {
        let id = id.clone();
        expanded.map(move |e| *e == id)
    };
    let auto = signal(c.auto_accept);
    let ask = signal(
        c.ask_above_bytes
            .map(|b| (b >> 20).to_string())
            .unwrap_or_default(),
    );
    let ask_bad = ask.map(|t| parse_ask_mb(t).is_none());
    let ask_ok = ask.map(|t| parse_ask_mb(t).is_some());

    let summary = format!("{} {}", c.devices.len(), strings::CONTACT_DEVICE_COUNT_UNIT);
    let toggle_id = id.clone();
    let ui_dm = ui.clone();
    let dm_user = id.clone();
    let head = Element::row()
        .width_match()
        .padding_xy(style::SPACE_LG, style::SPACE_MD)
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .child(
            Element::row()
                .weight(1.0)
                .spacing(style::SPACE_MD)
                .cross(Align::Center)
                .clickable()
                .on_click(move |_| {
                    expanded.update(|e| {
                        if *e == toggle_id {
                            e.clear();
                        } else {
                            e.clone_from(&toggle_id);
                        }
                    })
                })
                .child(avatar(&c.user_id, &c.display_name, c.online))
                .child(
                    Element::col()
                        .weight(1.0)
                        .spacing(style::SPACE_XS)
                        // 在线状态只由头像角标表示，名字旁不再重复画点
                        .child(name_label(c.display_name.clone()))
                        .child(meta(summary)),
                ),
        )
        .child(
            Element::button(strings::CONTACT_SEND_MESSAGE)
                .outline_soft()
                .intent(Intent::Neutral)
                .on_click(move |_| {
                    let user = dm_user.clone();
                    ui_dm.act(|m| m.open_dm(user));
                }),
        );

    let ui_save = ui.clone();
    let save_user = id.clone();
    let policy = Element::col()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(
            Element::row()
                .spacing(style::SPACE_SM)
                .cross(Align::Center)
                .child(Element::switch(auto))
                .child(
                    Element::label(strings::CONTACT_AUTO_ACCEPT)
                        .font_size(style::FONT_BODY)
                        .fg_role(Role::Text),
                ),
        )
        .child(
            Element::row()
                .spacing(style::SPACE_SM)
                .cross(Align::Center)
                .child(
                    Element::label(strings::CONTACT_ASK_ABOVE)
                        .font_size(style::FONT_BODY)
                        .fg_role(Role::TextMuted),
                )
                .child(
                    Element::text_input(ask, strings::CONTACT_ASK_ABOVE_PLACEHOLDER)
                        .numeric()
                        .width(96)
                        .height(style::CONTROL_H),
                )
                .child(
                    Element::label(strings::CONTACT_ASK_ABOVE_UNIT)
                        .font_size(style::FONT_BODY)
                        .fg_role(Role::TextMuted),
                )
                .child(Element::flex_spacer())
                .child(
                    Element::button(strings::CONTACT_SAVE_POLICY)
                        .enabled_when(move || ask_ok.get())
                        .on_click(move |_| {
                            if let Some(limit) = parse_ask_mb(&ask.get()) {
                                let user = save_user.clone();
                                ui_save.act(|m| m.set_contact_policy(user, auto.get(), limit));
                            }
                        }),
                ),
        )
        .child(
            Element::label(strings::CONTACT_ASK_INVALID)
                .font_size(style::FONT_META)
                .fg_role(Role::Danger)
                .visible_signal(ask_bad),
        );

    let mut details = Element::col()
        .width_match()
        .child(Element::divider())
        .child(
            Element::col()
                .width_match()
                .padding_xy(style::SPACE_LG, style::SPACE_SM)
                .child(meta(format!(
                    "{} {}",
                    strings::CONTACT_FINGERPRINT,
                    c.user_short
                ))),
        );
    for d in &c.devices {
        details = details.child(device_row(d, None));
    }
    let details = details
        .child(Element::divider())
        .child(
            Element::col()
                .width_match()
                .padding(style::SPACE_LG)
                .child(policy),
        )
        .visible_signal(open);

    card().child(head).child(details)
}

fn contacts_tab(ui: &UiState) -> Element {
    let digest = ui.digest(|m| m.contacts.clone());
    let expanded = signal(String::new());
    let ui = ui.clone();
    Element::switcher(digest, move |_| {
        let cs = ui.read(|m| m.contacts.clone());
        if cs.is_empty() {
            return style::empty_state(strings::CONTACTS_EMPTY, strings::CONTACTS_EMPTY_HINT, None);
        }
        Element::col()
            .width_match()
            .spacing(style::SPACE_MD)
            .children(cs.iter().map(|c| contact_card(&ui, c, expanded)))
    })
}

// ---------------------------------------------------------------------------
// 新的请求
// ---------------------------------------------------------------------------

fn request_kind_label(kind: &RequestKind) -> &'static str {
    match kind {
        RequestKind::Contact => strings::REQUEST_KIND_CONTACT,
        RequestKind::Message => strings::REQUEST_KIND_MESSAGE,
        RequestKind::File => strings::REQUEST_KIND_FILE,
    }
}

fn request_card(ui: &UiState, r: &RequestView, confirm: ConfirmState) -> Element {
    let act = |label: &'static str, action: RequestAction| {
        let (ui, id) = (ui.clone(), r.request.clone());
        Element::button(label)
            .outline_soft()
            .intent(Intent::Neutral)
            .on_click(move |_| {
                let request = id.clone();
                ui.act(|m| m.respond_request(request, action.clone()));
            })
    };
    let (block_id, block_name) = (r.request.clone(), r.from_name.clone());
    let block = Element::button(strings::REQUEST_BLOCK)
        .outline_soft()
        .intent(Intent::Danger)
        .on_click(move |_| {
            confirm.open(Confirm {
                id: block_id.clone(),
                title: strings::BLOCK_TITLE,
                message: strings::with_colon(&block_name, strings::BLOCK_MESSAGE_SUFFIX),
                action: strings::REQUEST_BLOCK,
            })
        });
    let (ui_add, add_id) = (ui.clone(), r.request.clone());
    let add = Element::button(strings::REQUEST_ADD_CONTACT).on_click(move |_| {
        let request = add_id.clone();
        ui_add.act(|m| m.respond_request(request, RequestAction::AddContact));
    });

    let mut info = Element::col()
        .weight(1.0)
        .spacing(style::SPACE_XS)
        .child(
            Element::row()
                .spacing(style::SPACE_SM)
                .cross(Align::Center)
                .child(name_label(r.from_name.clone())),
        )
        .child(
            Element::label(request_kind_label(&r.kind))
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted),
        )
        .child(meta(format!(
            "{} {}",
            strings::CONTACT_FINGERPRINT,
            r.from_short
        )));
    if let Some(addr) = &r.source_addr {
        info = info.child(meta(format!("{} {addr}", strings::REQUEST_SOURCE_ADDR)));
    }
    if r.addr_changed {
        info = info.child(
            Element::label(strings::REQUEST_ADDR_CHANGED)
                .font_size(style::FONT_META)
                .fg_role(Role::Danger)
                .max_lines(2),
        );
    }
    if let Some(text) = request_preview_text(&r.preview) {
        info = info.child(
            Element::label(text)
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(2)
                .truncate(Truncate::End),
        );
    }
    card()
        .padding(style::SPACE_LG)
        .spacing(style::SPACE_MD)
        .child(
            Element::row()
                .width_match()
                .spacing(style::SPACE_MD)
                .child(avatar(&r.from_user, &r.from_name, false))
                .child(info),
        )
        .child(
            Element::row()
                .width_match()
                .spacing(style::SPACE_SM)
                .child(act(strings::REQUEST_ACCEPT_ONCE, RequestAction::AcceptOnce))
                .children((!r.addr_changed).then_some(add))
                .child(act(strings::REQUEST_REJECT, RequestAction::Reject))
                .child(block),
        )
}

fn requests_tab(ui: &UiState, confirm: ConfirmState) -> Element {
    let digest = ui.digest(|m| m.requests.clone());
    let ui = ui.clone();
    Element::switcher(digest, move |_| {
        let rs = ui.read(|m| m.requests.clone());
        if rs.is_empty() {
            return style::empty_state(strings::REQUESTS_EMPTY, strings::REQUESTS_EMPTY_HINT, None);
        }
        Element::col()
            .width_match()
            .spacing(style::SPACE_MD)
            .children(rs.iter().map(|r| request_card(&ui, r, confirm)))
    })
}

// ---------------------------------------------------------------------------
// 附近的人
// ---------------------------------------------------------------------------

/// 手动添加对端地址（跨网段 / VPN）。
fn manual_peer_card(ui: &UiState) -> Element {
    let addr = signal(String::new());
    let empty = addr.map(|a| a.trim().is_empty());
    let valid = addr.map(|a| normalize_peer_addr(a).is_ok());
    // 非空但不合法时显示具体原因（红字）；空输入时说明按钮为何不可用
    let bad = addr.map(|a| !a.trim().is_empty() && normalize_peer_addr(a).is_err());
    let bad_text = addr.map(|a| normalize_peer_addr(a).err().unwrap_or_default().to_string());
    let ui = ui.clone();
    card()
        .padding(style::SPACE_LG)
        .spacing(style::SPACE_SM)
        .child(
            Element::label(strings::MANUAL_PEER_TITLE)
                .font_size(style::FONT_HEADING)
                .font_weight(style::WEIGHT_STRONG)
                .fg_role(Role::Text),
        )
        .child(meta(strings::manual_peer_hint()))
        .child(
            Element::row()
                .width_match()
                .spacing(style::SPACE_SM)
                .child(
                    Element::text_input(addr, strings::MANUAL_PEER_PLACEHOLDER)
                        .height(style::CONTROL_H)
                        .weight(1.0),
                )
                .child(
                    Element::button(strings::MANUAL_PEER_BUTTON)
                        .enabled_when(move || valid.get())
                        .on_click(move |_| {
                            let a = addr.get();
                            ui.act(|m| m.add_manual_peer(a));
                        }),
                ),
        )
        .child(
            Element::label(bad_text)
                .font_size(style::FONT_META)
                .fg_role(Role::Danger)
                .visible_signal(bad),
        )
        .child(
            Element::label(strings::MANUAL_PEER_EMPTY_HINT)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle)
                .visible_signal(empty),
        )
}

fn nearby_card(ui: &UiState, n: &Nearby) -> Element {
    let (ui, user, device) = (ui.clone(), n.user_id.clone(), n.device_id.clone());
    let local_send = n.user_id.is_empty();
    let mut title = Element::row()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(name_label(n.name.clone()));
    if let Some(b) = n.badge {
        title = title.child(style::pill(b));
    }
    card().child(
        Element::row()
            .width_match()
            .padding_xy(style::SPACE_LG, style::SPACE_MD)
            .spacing(style::SPACE_MD)
            .cross(Align::Center)
            .child(avatar(&n.user_id, &n.name, true))
            .child(
                Element::col()
                    .weight(1.0)
                    .spacing(style::SPACE_XS)
                    .child(title)
                    .child(meta(n.devices.join(strings::LIST_SEPARATOR)))
                    .child(meta(format!("{} {}", strings::NEARBY_FINGERPRINT, n.short))),
            )
            .child(
                Element::button(if local_send {
                    strings::CONTACT_SEND_MESSAGE
                } else {
                    strings::CONTACT_ADD
                })
                .on_click(move |_| {
                    if local_send {
                        let d = device.clone();
                        ui.act(|m| m.open_dm(d));
                    } else {
                        let u = user.clone();
                        ui.act(|m| m.request_contact(u));
                    }
                }),
            ),
    )
}

fn nearby_tab(ui: &UiState) -> Element {
    let people = ui.derive(|m| nearby_people(&m.peers));
    let ui_list = ui.clone();
    let list = Element::switcher(people, move |ps| {
        if ps.is_empty() {
            return style::empty_state(strings::NEARBY_EMPTY, strings::NEARBY_EMPTY_HINT, None);
        }
        Element::col()
            .width_match()
            .spacing(style::SPACE_MD)
            .children(ps.iter().map(|n| nearby_card(&ui_list, n)))
    });
    Element::col()
        .width_match()
        .spacing(style::SPACE_LG)
        .child(manual_peer_card(ui))
        .child(list)
}

// ---------------------------------------------------------------------------
// 页面
// ---------------------------------------------------------------------------

fn content(ui: &UiState, confirm: ConfirmState, remove_confirm: ConfirmState) -> Element {
    let tab = ui.derive(|m| m.contacts_tab);
    let ui = ui.clone();
    Element::switcher(tab, move |t| {
        let (title, body) = match t.min(CONTACTS_TABS - 1) {
            0 => (strings::TAB_CONTACTS, contacts_tab(&ui)),
            1 => (strings::TAB_REQUESTS, requests_tab(&ui, confirm)),
            2 => (strings::TAB_NEARBY, nearby_tab(&ui)),
            _ => (strings::TAB_DEVICES, devices_panel(&ui, remove_confirm)),
        };
        Element::col()
            .width_match()
            .max_width(CONTENT_MAX_W)
            .padding(style::SPACE_XL)
            .spacing(style::SPACE_LG)
            .child(page_title(title))
            .child(body)
    })
}

/// 联系人页：左分类导航 + 右内容；屏蔽请求、移除设备前二次确认。
/// 两个确认框都放在滚动区之外的最上层，遮罩才是全窗的。
pub fn contacts_page(ui: &UiState) -> Element {
    let confirm = ConfirmState::new();
    let remove_confirm = ConfirmState::new();
    let ui_ok = ui.clone();
    let body = Element::row()
        .fill()
        .child(side_nav(ui))
        .child(style::vline())
        .child(
            // 右侧留出滚动条槽，滚动条不贴窗口边
            Element::col()
                .weight(1.0)
                .height_match()
                .padding_edges(Insets {
                    top: 0,
                    bottom: 0,
                    left: 0,
                    right: style::SPACE_XS,
                })
                .child(Element::scroll().width_match().weight(1.0).child(content(
                    ui,
                    confirm,
                    remove_confirm,
                ))),
        );
    Element::stack()
        .fill()
        .bg_role(Role::Bg)
        .child(body)
        .child(confirm_host(confirm, move |request| {
            ui_ok.act(|m| m.respond_request(request, RequestAction::Block));
        }))
        .child(remove_confirm_host(ui, remove_confirm))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanmsg_api::types::PathClassDto;

    fn peer(device: &str, user: &str, path: PathClassDto, contact: bool, mine: bool) -> PeerView {
        PeerView {
            device_id: device.into(),
            device_short: device.into(),
            user_id: user.into(),
            display_name: user.into(),
            device_name: format!("{device}-name"),
            device_type: "laptop".into(),
            addr: "10.0.0.1:7000".into(),
            path,
            is_contact: contact,
            is_mine: mine,
        }
    }

    #[test]
    fn nearby_people_excludes_contacts_and_own_devices() {
        let ps = vec![
            peer("d1", "u1", PathClassDto::Lan, true, false),
            peer("d2", "u2", PathClassDto::Lan, false, true),
            peer("d3", "u3", PathClassDto::Lan, false, false),
        ];
        let n = nearby_people(&ps);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].user_id, "u3");
        assert_eq!(n[0].badge, None);
    }

    #[test]
    fn localsend_devices_without_user_are_listed_separately() {
        let ps = vec![
            peer("d1", "", PathClassDto::Lan, false, false),
            peer("d2", "", PathClassDto::Lan, false, false),
        ];
        let n = nearby_people(&ps);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].device_id, "d1");
        assert_eq!(n[1].device_id, "d2");
    }

    #[test]
    fn nearby_people_merges_devices_and_keeps_best_path_badge() {
        let ps = vec![
            peer("d1", "u1", PathClassDto::Vpn, false, false),
            peer("d2", "u1", PathClassDto::Lan, false, false),
            peer("d3", "u2", PathClassDto::Vpn, false, false),
        ];
        let n = nearby_people(&ps);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].devices, vec!["d1-name", "d2-name"]);
        assert_eq!(n[0].badge, None, "同一用户有局域网路径时不标角标");
        assert_eq!(n[1].badge, Some(strings::PATH_VPN));
    }
}
