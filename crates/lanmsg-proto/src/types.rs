// P1 核心骨架：协议数据类型（C3）。
// 所有类型在 P1 定义，P2 起使用的类型（BlobMeta、MediaInfo、FolderEntry、SystemEvent）
// 也在此处结构定义好，避免 P2 改动线格式。
// 注释中文，标识符英文。

use std::collections::BTreeMap;
use std::fmt;

use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ids::{BlobHash, ConvId, DeviceId, MsgId, UserId};

/// 网络路径类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathClass {
    Lan,
    Vpn,
    Internet,
    Relay,
}

/// 设备类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceType {
    Desktop,
    Laptop,
    Phone,
    Tablet,
    Headless,
    Web,
}

/// 在线状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Online,
    Invisible,
    Busy,
}

/// 混合逻辑时钟，Ord 按 (millis, counter)。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hlc {
    pub millis: i64,
    pub counter: u32,
}

/// 版本向量，记录每个作者在会话中连续收齐的最大 conv_seq。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionVector(pub BTreeMap<DeviceId, u64>);

/// 64 字节签名，serde 序列化为 bytes（与 32 字节标识类型一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature64(pub [u8; 64]);

impl Serialize for Signature64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Signature64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SigVisitor;
        impl<'de> de::Visitor<'de> for SigVisitor {
            type Value = Signature64;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "64 字节签名")
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                if v.len() != 64 {
                    return Err(E::custom("签名长度不为 64 字节"));
                }
                let mut arr = [0u8; 64];
                arr.copy_from_slice(v);
                Ok(Signature64(arr))
            }
        }
        deserializer.deserialize_bytes(SigVisitor)
    }
}

/// 设备证书，由用户密钥签发，sig 覆盖置零后的整个结构 CBOR 编码。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCert {
    pub device: DeviceId,
    pub user: UserId,
    pub device_name: String,
    pub device_type: DeviceType,
    pub created_at: i64,
    pub key_epoch: u32,
    pub sig: Signature64,
}

/// 消息信封，由设备密钥签名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub conv: ConvId,
    pub id: MsgId,
    pub conv_seq: u64,
    pub hlc: Hlc,
    pub body: MsgBody,
    pub enc: Option<Vec<u8>>,
    pub sig: Signature64,
}

/// 消息体。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MsgBody {
    Text {
        text: String,
    },
    Files {
        items: Vec<BlobMeta>,
    },
    Folder {
        name: String,
        entries: Vec<FolderEntry>,
    },
    Recall {
        target: MsgId,
    },
    Read {
        upto: Hlc,
    },
    System {
        event: SystemEvent,
    },
}

/// Blob 元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobMeta {
    pub hash: BlobHash,
    pub size: u64,
    pub name: String,
    pub mime: String,
    pub media: Option<MediaInfo>,
    pub thumb: Option<Vec<u8>>,
}

/// 媒体信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub duration_ms: Option<u64>,
}

/// 文件夹条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderEntry {
    pub rel_path: String,
    pub blob: BlobMeta,
}

/// 系统事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemEvent {
    DeviceAdded { device: DeviceId, name: String },
    DeviceRemoved { device: DeviceId },
    ContactAdded { user: UserId },
}

/// 设备通告，由设备密钥签名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announce {
    pub proto: u16,
    pub cert: DeviceCert,
    pub display_name: String,
    pub avatar_hash: Option<BlobHash>,
    pub ports: Vec<u16>,
    pub status: Status,
    pub sig: Signature64,
}

/// 同步提供。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOffer {
    pub conv: ConvId,
    pub vv: VersionVector,
}

/// 同步推送。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPush {
    pub envelopes: Vec<Envelope>,
}

/// 确认。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ack {
    pub ids: Vec<MsgId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign::{from_cbor, to_cbor};

    #[test]
    fn signature64_cbor_roundtrip() {
        let sig = Signature64([42u8; 64]);
        let encoded = to_cbor(&sig);
        let decoded: Signature64 = from_cbor(&encoded).unwrap();
        assert_eq!(decoded, sig);
    }
}
