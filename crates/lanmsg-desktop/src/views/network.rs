// 设置页“网络”分组的扩展：手动添加设备、网段扫描、本机网卡，以及路径角标文字。
// 单播通告目标与 LocalSend 开关仍在 settings.rs 的“网络”卡片里。
// 注释中文，标识符英文。

use std::net::Ipv4Addr;

use lanmsg_api::types::{InterfaceView, PathClassDto};
use windui::prelude::*;

use crate::model::{normalize_peer_addr, scan_cidr_ok};
use crate::strings;
use crate::style;
use crate::ui_state::UiState;
use crate::views::settings::{card, field, note};

/// 非局域网路径的角标文字；局域网不标。
pub fn path_badge_text(p: &PathClassDto) -> Option<&'static str> {
    match p {
        PathClassDto::Lan => None,
        PathClassDto::Vpn => Some(strings::PATH_VPN),
        PathClassDto::Internet => Some(strings::PATH_INTERNET),
        PathClassDto::Relay => Some(strings::PATH_RELAY),
    }
}

/// 隧道类型的显示名；core 给的是稳定字符串，未知的统一叫“隧道”。
pub fn tunnel_label(kind: &str) -> &'static str {
    match kind {
        "tailscale" => strings::TUNNEL_TAILSCALE,
        "zerotier" => strings::TUNNEL_ZEROTIER,
        "wireguard" => strings::TUNNEL_WIREGUARD,
        _ => strings::TUNNEL_OTHER,
    }
}

/// 网卡行扫描按钮要提交的网段：前缀在 /22 到 /30 之间用网卡自身网段，前缀更小（网段过大）
/// 取本机地址所在 /24；/31、/32 没有可探测的邻居，返回空串（不显示按钮）。
/// core 只接受 IPv4 且前缀不小于 /22，非 IPv4 网卡同样返回空串。
pub fn scan_cidr_for(iface: &InterfaceView) -> String {
    let Ok(addr) = iface.addr.parse::<Ipv4Addr>() else {
        return String::new();
    };
    if iface.prefix > 30 {
        return String::new();
    }
    let prefix = if iface.prefix >= lanmsg_proto::consts::SCAN_MIN_PREFIX {
        iface.prefix
    } else {
        24
    };
    match ipnet::Ipv4Net::new(addr, prefix) {
        Ok(net) => net.trunc().to_string(),
        Err(_) => String::new(),
    }
}

/// LocalSend 端口被占用的提示（开关下方）；监听状态由 core 的 `localsend_listening` 给出。
pub fn localsend_busy_note(ui: &UiState) -> Element {
    let busy = ui.derive(|m| {
        m.settings
            .as_ref()
            .is_some_and(|s| s.localsend_enabled && s.localsend_listening == Some(false))
    });
    Element::label(strings::LOCALSEND_PORT_BUSY)
        .font_size(style::FONT_META)
        .fg_role(Role::Warning)
        .width_match()
        .padding_xy(style::SPACE_LG, style::SPACE_SM)
        .visible_signal(busy)
}

fn meta_label(text: impl Into<TextContent>, role: Role) -> Element {
    Element::label(text)
        .font_size(style::FONT_META)
        .fg_role(role)
}

/// 手动添加：输入框 + 连接；下方是连接状态和已添加的地址列表。
fn manual_row(ui: &UiState) -> Element {
    let addr = signal(String::new());
    let valid = addr.map(|a| normalize_peer_addr(a).is_ok());
    let status = ui.derive(|m| {
        if m.manual_connecting {
            strings::NET_MANUAL_CONNECTING.to_string()
        } else {
            m.manual_result.clone().unwrap_or_default()
        }
    });
    let has_status = status.map(|s| !s.is_empty());
    let ui_add = ui.clone();
    let input = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(
            Element::text_input(addr, strings::NET_MANUAL_PLACEHOLDER)
                .height(style::CONTROL_H)
                .weight(1.0),
        )
        .child(
            Element::button(strings::MANUAL_PEER_BUTTON)
                .enabled_when(move || valid.get())
                .on_click(move |_| {
                    let a = addr.get();
                    ui_add.act(|m| m.add_manual_peer(a));
                }),
        );
    let ui_list = ui.clone();
    let list = Element::switcher(ui.digest(|m| m.manual_peers.clone()), move |_| {
        let peers = ui_list.read(|m| m.manual_peers.clone());
        if peers.is_empty() {
            return meta_label(strings::NET_MANUAL_EMPTY, Role::TextSubtle).width_match();
        }
        Element::col()
            .width_match()
            .spacing(style::SPACE_XS)
            .child(meta_label(strings::NET_MANUAL_LIST, Role::TextMuted))
            .children(peers.into_iter().map(|p| {
                let ui_rm = ui_list.clone();
                let target = p.addr.clone();
                let mut row = Element::row()
                    .width_match()
                    .spacing(style::SPACE_SM)
                    .cross(Align::Center)
                    .child(
                        Element::label(p.addr)
                            .font_size(style::FONT_BODY)
                            .fg_role(Role::Text)
                            .max_lines(1)
                            .truncate(Truncate::End)
                            .weight(1.0),
                    );
                if p.device_id.is_some() {
                    row = row.child(style::pill(strings::NET_MANUAL_LINKED));
                }
                row.child(
                    Element::button(strings::NET_MANUAL_REMOVE)
                        .outline()
                        .small()
                        .on_click(move |_| ui_rm.act(|m| m.remove_manual_peer(target.clone()))),
                )
            }))
    })
    .width_match();
    field(
        strings::NET_MANUAL_TITLE,
        input,
        vec![
            Element::label_signal(status)
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted)
                .width_match()
                .visible_signal(has_status),
            list,
        ],
    )
}

/// 网段扫描：手输网段、扫描进度与结果。
fn scan_row(ui: &UiState) -> Element {
    let cidr = signal(String::new());
    let scanning = ui.derive(|m| m.scanning.is_some());
    let idle = ui.derive(|m| m.scanning.is_none());
    let ok = cidr.map(|c| scan_cidr_ok(c));
    let ui_scan = ui.clone();
    let input = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(
            Element::text_input(cidr, strings::NET_SCAN_PLACEHOLDER)
                .height(style::CONTROL_H)
                .weight(1.0),
        )
        .child(
            Element::button(strings::NET_SCAN_BUTTON)
                .enabled_when(move || ok.get() && idle.get())
                .on_click(move |_| {
                    let c = cidr.get();
                    ui_scan.act(|m| m.scan(c));
                }),
        );
    let busy_text = ui.derive(|m| {
        m.scanning
            .as_deref()
            .map(strings::net_scanning)
            .unwrap_or_default()
    });
    let busy = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .visible_signal(scanning)
        .child(Element::progress_indeterminate().width(96))
        .child(
            Element::label_signal(busy_text)
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted),
        );
    let result = ui.derive(|m| m.scan_result.clone().unwrap_or_default());
    let has_result = result.map(|r| !r.is_empty());
    field(
        strings::NET_SCAN_TITLE,
        input,
        vec![
            note(strings::NET_SCAN_HINT),
            busy,
            Element::label_signal(result)
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted)
                .width_match()
                .visible_signal(has_result),
        ],
    )
}

/// 一块网卡：名称、`addr/prefix`、隧道角标、“扫描此网段”。
fn iface_row(ui: &UiState, i: &InterfaceView) -> Element {
    let mut title = Element::row()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(
            Element::label(i.name.clone())
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End),
        );
    if let Some(kind) = &i.tunnel_kind {
        title = title.child(style::pill(tunnel_label(kind)));
    }
    let cidr = scan_cidr_for(i);
    let idle = ui.derive(|m| m.scanning.is_none());
    let ui_scan = ui.clone();
    let mut row = Element::row()
        .width_match()
        .spacing(style::SPACE_MD)
        .cross(Align::Center)
        .child(Element::col().weight(1.0).child(title).child(meta_label(
            format!("{}/{}", i.addr, i.prefix),
            Role::TextSubtle,
        )));
    if !cidr.is_empty() {
        row = row.child(
            Element::button(strings::net_scan_cidr_button(&cidr))
                .outline()
                .small()
                .enabled_when(move || idle.get())
                .on_click(move |_| {
                    let c = cidr.clone();
                    ui_scan.act(|m| m.scan(c));
                }),
        );
    }
    row
}

fn interfaces_row(ui: &UiState) -> Element {
    let ui_list = ui.clone();
    let list = Element::switcher(ui.digest(|m| m.interfaces.clone()), move |_| {
        let ifaces = ui_list.read(|m| m.interfaces.clone());
        if ifaces.is_empty() {
            return meta_label(strings::NET_IFACES_EMPTY, Role::TextSubtle).width_match();
        }
        Element::col()
            .width_match()
            .spacing(style::SPACE_SM)
            .children(ifaces.iter().map(|i| iface_row(&ui_list, i)))
    })
    .width_match();
    field(strings::NET_IFACES_TITLE, list, Vec::new())
}

/// 设置页“网络”分组里单播目标之外的部分：手动添加、网段扫描、本机网卡。
pub fn network_section(ui: &UiState) -> Element {
    card(vec![manual_row(ui), scan_row(ui), interfaces_row(ui)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(addr: &str, prefix: u8) -> InterfaceView {
        InterfaceView {
            name: "eth0".into(),
            addr: addr.into(),
            prefix,
            tunnel_kind: None,
        }
    }

    #[test]
    fn path_badge_text_cases() {
        assert_eq!(path_badge_text(&PathClassDto::Lan), None);
        assert_eq!(path_badge_text(&PathClassDto::Vpn), Some("VPN"));
        assert_eq!(path_badge_text(&PathClassDto::Internet), Some("公网"));
        assert_eq!(path_badge_text(&PathClassDto::Relay), Some("中继"));
    }

    #[test]
    fn scan_cidr_for_small_and_large_prefix() {
        assert_eq!(scan_cidr_for(&iface("192.168.1.10", 24)), "192.168.1.0/24");
        assert_eq!(scan_cidr_for(&iface("10.1.2.3", 8)), "10.1.2.0/24");
        assert_eq!(scan_cidr_for(&iface("10.1.4.5", 22)), "10.1.4.0/22");
        // 前缀恰在边界：/21 比下限大一档，退到 /24
        assert_eq!(scan_cidr_for(&iface("10.1.4.5", 21)), "10.1.4.0/24");
        // /25–/30 用自身网段；/31、/32（点对点隧道）没有邻居，不提供扫描
        assert_eq!(
            scan_cidr_for(&iface("192.168.1.130", 25)),
            "192.168.1.128/25"
        );
        assert_eq!(scan_cidr_for(&iface("10.8.0.2", 31)), "");
        assert_eq!(scan_cidr_for(&iface("100.101.102.103", 32)), "");
        // core 只接受 IPv4
        assert_eq!(scan_cidr_for(&iface("fe80::1", 64)), "");
    }

    #[test]
    fn scan_cidr_for_result_is_accepted_by_scan_rule() {
        for (a, p) in [("192.168.1.10", 24), ("10.1.2.3", 8), ("172.16.5.9", 16)] {
            assert!(scan_cidr_ok(&scan_cidr_for(&iface(a, p))), "{a}/{p}");
        }
    }

    #[test]
    fn tunnel_label_maps_known_kinds() {
        assert_eq!(tunnel_label("tailscale"), "Tailscale");
        assert_eq!(tunnel_label("zerotier"), "ZeroTier");
        assert_eq!(tunnel_label("wireguard"), "WireGuard");
        assert_eq!(tunnel_label("tun"), "隧道");
        assert_eq!(tunnel_label("whatever"), "隧道");
    }
}
