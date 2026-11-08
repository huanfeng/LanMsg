//! LocalSend v2 的 JSON 类型，字段名严格按官方协议（camelCase）。

use std::collections::BTreeMap;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// 传输协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LsProtocol {
    Http,
    Https,
}

/// 设备信息（通告、register、prepare-upload 共用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LsInfo {
    pub alias: String,
    pub version: String,
    #[serde(default)]
    pub device_model: Option<String>,
    /// mobile / desktop / web / headless / server；未知值由界面按 desktop 处理。
    #[serde(default)]
    pub device_type: Option<String>,
    pub fingerprint: String,
    pub port: u16,
    pub protocol: LsProtocol,
    #[serde(default)]
    pub download: bool,
}

/// 组播通告：设备信息加 `announce`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsAnnounce {
    #[serde(flatten)]
    pub info: LsInfo,
    pub announce: bool,
}

/// 文件时间戳（ISO 8601 字符串）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsFileTimes {
    #[serde(default)]
    pub modified: Option<String>,
    #[serde(default)]
    pub accessed: Option<String>,
}

/// prepare-upload 中单个文件的描述。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LsFileMeta {
    pub id: String,
    pub file_name: String,
    pub size: u64,
    pub file_type: String,
    #[serde(default)]
    pub sha256: Option<String>,
    /// 文字消息的正文放在这里。
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub metadata: Option<LsFileTimes>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareUploadRequest {
    pub info: LsInfo,
    pub files: BTreeMap<String, LsFileMeta>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadResponse {
    pub session_id: String,
    /// fileId -> token
    pub files: BTreeMap<String, String>,
}

/// 发现到的对端：设备信息加来源 IP。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LsPeer {
    pub info: LsInfo,
    pub ip: IpAddr,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_official_announce_fixture() {
        let a: LsAnnounce =
            serde_json::from_str(include_str!("../tests/fixtures/announce.json")).unwrap();
        assert_eq!(a.info.version, "2.0");
        assert!(a.announce);
        assert_eq!(a.info.protocol, LsProtocol::Https);
        assert_eq!(a.info.alias, "Nice Orange");
        assert!(a.info.download);
    }

    #[test]
    fn download_field_defaults_false_and_nulls_parse() {
        let j = r#"{"alias":"x","version":"2.0","deviceModel":null,"deviceType":null,"fingerprint":"f","port":53317,"protocol":"http","announce":true}"#;
        let a: LsAnnounce = serde_json::from_str(j).unwrap();
        assert!(!a.info.download);
        assert_eq!(a.info.device_type, None);
        assert_eq!(a.info.device_model, None);
    }

    #[test]
    fn register_fixture_parses_as_info() {
        let i: LsInfo =
            serde_json::from_str(include_str!("../tests/fixtures/register.json")).unwrap();
        assert_eq!(i.port, 53317);
    }

    #[test]
    fn prepare_upload_serializes_camel_case() {
        let req: PrepareUploadRequest = serde_json::from_str(include_str!(
            "../tests/fixtures/prepare_upload_request.json"
        ))
        .unwrap();
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"fileName\""));
        assert!(s.contains("\"fileType\""));
        assert!(s.contains("\"deviceModel\""));
        assert!(!s.contains("file_name"));
        let f = &req.files["file-id-1"];
        assert_eq!(f.size, 12345);
        assert_eq!(f.metadata.as_ref().unwrap().accessed, None);
    }

    #[test]
    fn prepare_upload_response_fixture_parses() {
        let r: PrepareUploadResponse = serde_json::from_str(include_str!(
            "../tests/fixtures/prepare_upload_response.json"
        ))
        .unwrap();
        assert_eq!(r.session_id, "session-1");
        assert_eq!(r.files["file-id-1"], "token-1");
        assert!(serde_json::to_string(&r).unwrap().contains("\"sessionId\""));
    }

    #[test]
    fn text_message_fixture_parses_with_missing_optionals() {
        let r: PrepareUploadRequest =
            serde_json::from_str(include_str!("../tests/fixtures/text_message_request.json"))
                .unwrap();
        let f = &r.files["text-id-1"];
        assert_eq!(f.file_type, "text/plain");
        assert_eq!(f.preview.as_deref(), Some("hello"));
        assert_eq!(f.sha256, None);
        assert_eq!(f.metadata, None);
    }
}
