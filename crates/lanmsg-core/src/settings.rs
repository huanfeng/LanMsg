// P2 Task 4：设置读写。
// settings 键存 CBOR hex 的 SettingsDto；device_name 独立存 setting 且加载时以其为准
// （设备名改动同时驱动证书重签，独立键便于不解析 settings 包体即可读取）。
// 注释中文，标识符英文。

use std::path::{Path, PathBuf};

use directories::UserDirs;
use lanmsg_api::types::{BackgroundMode, SettingsDto};
use lanmsg_proto::consts::{AUTO_DOWNLOAD_FILE_MAX, AUTO_DOWNLOAD_IMAGE_MAX};
use lanmsg_proto::{from_cbor, to_cbor};

use crate::error::CoreError;
use crate::identity::{hex_decode, hex_encode};
use crate::store::Store;

/// settings 表键名。
const SETTING_SETTINGS: &str = "settings";
/// 数据目录 setting 键（Service::start 每次启动写入）。
pub(crate) const SETTING_DATA_DIR: &str = "data_dir";
/// 设备名 setting 键（init/reissue_cert/SetSettings 写入）。
pub(crate) const SETTING_DEVICE_NAME: &str = "device_name";
/// 待接入标记 setting 键（Init JoinExisting 写入）。
pub(crate) const SETTING_JOIN_PENDING: &str = "join_pending";
/// 已应用的最大 ProfileUpdated hlc。
pub(crate) const SETTING_PROFILE_HLC: &str = "profile_hlc";

/// 默认设置：下载目录取平台下载目录下的 LanMsg 子目录（取不到则 data_dir/downloads），
/// 自动下载阈值按 AUTO_DOWNLOAD_* 常量，wifi 才自动下载文件、LocalSend 开启、实时后台。
pub fn default_settings(data_dir: &Path, device_name: &str) -> SettingsDto {
    let download_dir = UserDirs::new()
        .as_ref()
        .and_then(|u| u.download_dir())
        .map(|p| p.join("LanMsg"))
        .unwrap_or_else(|| data_dir.join("downloads"));
    SettingsDto {
        device_name: device_name.to_string(),
        download_dir: download_dir.to_string_lossy().into_owned(),
        auto_image_max: AUTO_DOWNLOAD_IMAGE_MAX,
        auto_file_max: AUTO_DOWNLOAD_FILE_MAX,
        auto_file_wifi_only: true,
        localsend_enabled: true,
        unicast_targets: Vec::new(),
        background_mode: BackgroundMode::Realtime,
        localsend_listening: None,
    }
}

/// 从存储加载设置；无记录时返回默认值。
///
/// `data_dir` 读 setting "data_dir"（缺失用空 PathBuf），设备名总是以 setting
/// "device_name" 为准（缺失为空串）。
pub fn load_settings(store: &Store) -> Result<SettingsDto, CoreError> {
    let data_dir: PathBuf = store
        .get_setting(SETTING_DATA_DIR)?
        .map(PathBuf::from)
        .unwrap_or_default();
    let device_name = store.get_setting(SETTING_DEVICE_NAME)?.unwrap_or_default();
    let mut s = match store.get_setting(SETTING_SETTINGS)? {
        Some(hex) => {
            let bytes = hex_decode(&hex)?;
            from_cbor(&bytes)
                .map_err(|e| CoreError::Proto(format!("failed to decode settings: {e}")))?
        }
        None => default_settings(&data_dir, &device_name),
    };
    s.device_name = device_name;
    Ok(s)
}

/// 保存设置（CBOR hex 写 setting "settings"）。
pub fn save_settings(store: &Store, s: &SettingsDto) -> Result<(), CoreError> {
    let bytes = to_cbor(s);
    store.set_setting(SETTING_SETTINGS, &hex_encode(&bytes))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_use_spec_thresholds() {
        let s = default_settings(Path::new("/tmp/x"), "pc");
        assert_eq!(s.auto_image_max, 10 * 1024 * 1024);
        assert_eq!(s.auto_file_max, 50 * 1024 * 1024);
        assert!(s.auto_file_wifi_only);
    }

    #[test]
    fn settings_roundtrip_through_store() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(SETTING_DATA_DIR, "/tmp/dd").unwrap();
        store.set_setting(SETTING_DEVICE_NAME, "手机").unwrap();

        // 无记录 → 默认值，device_name 以 setting 为准。
        let s = load_settings(&store).unwrap();
        assert_eq!(s.device_name, "手机");
        assert_eq!(s.auto_image_max, 10 * 1024 * 1024);

        // 保存后往返一致。
        let mut s2 = s.clone();
        s2.auto_file_max = 1;
        save_settings(&store, &s2).unwrap();
        let s3 = load_settings(&store).unwrap();
        assert_eq!(s3.auto_file_max, 1);
        assert_eq!(s3.device_name, "手机", "device_name 仍以 setting 为准");
    }
}
