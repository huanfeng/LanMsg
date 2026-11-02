// P2 Task 13：自动下载策略。
// should_auto_download 按规则链依次判定收到的文件是否自动下载；
// 上下文 PolicyCtx 由调用方（ingest 的自动下载接线）构造。
// 注释中文，标识符英文。

use lanmsg_api::types::SettingsDto;
use lanmsg_proto::consts::VPN_THRESHOLD_DIVISOR;
use lanmsg_proto::types::{BlobMeta, PathClass};

use crate::platform::NetworkState;
use crate::store::{ContactRow, ContactState};

/// 自动下载策略上下文：from 设备的连接路径、当前网络、作者是否自己、
/// 作者联系人行（陌生人为 None）、会话 gate 是否对陌生人开放。
pub struct PolicyCtx {
    pub path: PathClass,
    pub net: NetworkState,
    pub from_self: bool,
    pub contact: Option<ContactRow>,
    pub gate_open_stranger: bool,
}

/// 是否自动下载（规则依次）：
/// 1. 既非 from_self 也非 Accepted 联系人 → false（陌生人含"接收一次"的
///    会话都不自动下载）。
/// 2. 联系人 auto_accept == false → false。
/// 3. path ∈ {Internet, Relay} → false。
/// 4. 是否图片 = mime 以 image/ 开头。阈值 t = 图片 ? auto_image_max :
///    auto_file_max；联系人 ask_above_bytes = Some(x) → t = min(t, x)；
///    path == Vpn → t /= VPN_THRESHOLD_DIVISOR。
/// 5. 非图片且 auto_file_wifi_only 且非 wifi/有线 → false；非图片且
///    计费网络 → false。
/// 6. size <= t → true。
pub fn should_auto_download(meta: &BlobMeta, s: &SettingsDto, ctx: &PolicyCtx) -> bool {
    // 规则 1：陌生人（含"接收一次"开放的会话）一律不自动下载。
    let is_accepted = ctx
        .contact
        .as_ref()
        .is_some_and(|c| c.state == ContactState::Accepted);
    if !ctx.from_self && !is_accepted {
        return false;
    }
    // 规则 2：联系人的自动接收关闭 → false（from_self 时 contact 为 None，跳过）。
    if ctx.contact.as_ref().is_some_and(|c| !c.auto_accept) {
        return false;
    }
    // 规则 3：互联网/中转路径不自动下载。
    if matches!(ctx.path, PathClass::Internet | PathClass::Relay) {
        return false;
    }
    // 规则 4：按类型取阈值并收紧（联系人上限 → VPN 折半）。
    let is_image = meta.mime.starts_with("image/");
    let mut t = if is_image {
        s.auto_image_max
    } else {
        s.auto_file_max
    };
    if let Some(c) = &ctx.contact {
        if let Some(x) = c.ask_above_bytes {
            t = t.min(x);
        }
    }
    if ctx.path == PathClass::Vpn {
        t /= VPN_THRESHOLD_DIVISOR;
    }
    // 规则 5：文件受 wifi 限制与计费限制；图片不受。
    if !is_image {
        if s.auto_file_wifi_only && !ctx.net.is_wifi_or_wired {
            return false;
        }
        if ctx.net.is_metered {
            return false;
        }
    }
    // 规则 6：不超过阈值 → 自动下载。
    meta.size <= t
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanmsg_proto::ids::BlobHash;
    use lanmsg_proto::types::Hlc;

    /// 表驱动基准：默认设置（图片 10MB、文件 50MB、wifi 才自动下载文件）。
    fn base_ctx() -> PolicyCtx {
        PolicyCtx {
            path: PathClass::Lan,
            net: NetworkState {
                is_metered: false,
                is_wifi_or_wired: true,
            },
            from_self: false,
            contact: Some(ContactRow {
                user: lanmsg_proto::ids::UserId([7; 32]),
                state: ContactState::Accepted,
                auto_accept: true,
                ask_above_bytes: None,
                updated: Hlc {
                    millis: 0,
                    counter: 0,
                },
            }),
            gate_open_stranger: false,
        }
    }

    fn settings() -> SettingsDto {
        SettingsDto {
            device_name: "测试".into(),
            download_dir: "/tmp/dd".into(),
            auto_image_max: 10 * 1024 * 1024,
            auto_file_max: 50 * 1024 * 1024,
            auto_file_wifi_only: true,
            localsend_enabled: true,
            unicast_targets: Vec::new(),
            background_mode: lanmsg_api::types::BackgroundMode::Realtime,
            localsend_listening: None,
        }
    }

    fn meta(size: u64, mime: &str) -> BlobMeta {
        BlobMeta {
            hash: BlobHash([0; 32]),
            size,
            name: "文件".into(),
            mime: mime.into(),
            media: None,
            thumb: None,
        }
    }

    /// 计划表驱动用例：注释即用例名。
    #[test]
    fn policy_table() {
        let s = settings();
        let lan = base_ctx();
        // 图片 5MB 联系人 Lan → true。
        assert!(should_auto_download(
            &meta(5 * 1024 * 1024, "image/jpeg"),
            &s,
            &lan
        ));
        // 图片 11MB → false。
        assert!(!should_auto_download(
            &meta(11 * 1024 * 1024, "image/jpeg"),
            &s,
            &lan
        ));
        // 视频 40MB Lan wifi → true。
        assert!(should_auto_download(
            &meta(40 * 1024 * 1024, "video/mp4"),
            &s,
            &lan
        ));
        // 视频 40MB 非 wifi → false。
        let not_wifi = PolicyCtx {
            net: NetworkState {
                is_metered: false,
                is_wifi_or_wired: false,
            },
            ..base_ctx()
        };
        assert!(!should_auto_download(
            &meta(40 * 1024 * 1024, "video/mp4"),
            &s,
            &not_wifi
        ));
        // 视频 40MB Vpn（阈值 25MB）→ false。
        let vpn = PolicyCtx {
            path: PathClass::Vpn,
            ..base_ctx()
        };
        assert!(!should_auto_download(
            &meta(40 * 1024 * 1024, "video/mp4"),
            &s,
            &vpn
        ));
        // 视频 20MB Vpn → true。
        assert!(should_auto_download(
            &meta(20 * 1024 * 1024, "video/mp4"),
            &s,
            &vpn
        ));
        // 任意 Internet → false（即使极小的图片）。
        let internet = PolicyCtx {
            path: PathClass::Internet,
            ..base_ctx()
        };
        assert!(!should_auto_download(&meta(1, "image/jpeg"), &s, &internet));
        // 陌生人 → false。
        let stranger = PolicyCtx {
            contact: None,
            ..base_ctx()
        };
        assert!(!should_auto_download(&meta(1, "image/jpeg"), &s, &stranger));
        // 联系人 auto_accept=false → false。
        let no_auto = PolicyCtx {
            contact: Some(ContactRow {
                auto_accept: false,
                ..base_ctx().contact.unwrap()
            }),
            ..base_ctx()
        };
        assert!(!should_auto_download(&meta(1, "image/jpeg"), &s, &no_auto));
        // ask_above=Some(1MB) 图片 2MB → false。
        let ask_1m = PolicyCtx {
            contact: Some(ContactRow {
                ask_above_bytes: Some(1024 * 1024),
                ..base_ctx().contact.unwrap()
            }),
            ..base_ctx()
        };
        assert!(!should_auto_download(
            &meta(2 * 1024 * 1024, "image/jpeg"),
            &s,
            &ask_1m
        ));
        // from_self 图片 5MB → true（自己设备发的文件自动下载）。
        let mine = PolicyCtx {
            from_self: true,
            contact: None,
            ..base_ctx()
        };
        assert!(should_auto_download(
            &meta(5 * 1024 * 1024, "image/jpeg"),
            &s,
            &mine
        ));
    }
}
