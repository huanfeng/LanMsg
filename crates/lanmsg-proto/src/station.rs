// P5 文件中转站帧：站点信息、条目与主机/拉取端之间的子消息。
// 注释中文，标识符英文。

use data_encoding::HEXLOWER;
use rand::Rng;
use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use crate::ids::{ConvId, DeviceId, IdError, UserId};
use crate::types::{BlobMeta, FolderEntry};

/// 中转站条目标识（16 字节，serde 为 bytes）。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(pub [u8; 16]);

impl ItemId {
    /// 随机生成。
    pub fn random() -> ItemId {
        let mut arr = [0u8; 16];
        rand::thread_rng().fill(&mut arr);
        ItemId(arr)
    }

    /// 小写十六进制编码（32 字符）。
    pub fn to_hex(&self) -> String {
        HEXLOWER.encode(&self.0)
    }

    /// 从小写十六进制解析，长度必须为 16 字节。
    pub fn from_hex(s: &str) -> Result<Self, IdError> {
        let bytes = HEXLOWER.decode(s.as_bytes()).map_err(|_| IdError::BadHex)?;
        let arr: [u8; 16] = bytes.try_into().map_err(|_| IdError::BadLength)?;
        Ok(ItemId(arr))
    }
}

impl fmt::Debug for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ItemId({})", self.to_hex())
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Serialize for ItemId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for ItemId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ItemIdVisitor;
        impl<'de> de::Visitor<'de> for ItemIdVisitor {
            type Value = ItemId;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a 16-byte string")
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                let arr: [u8; 16] = v.try_into().map_err(|_| E::custom(IdError::BadLength))?;
                Ok(ItemId(arr))
            }
        }
        deserializer.deserialize_bytes(ItemIdVisitor)
    }
}

/// 站点访问控制；v1 只使用 OwnerOnly。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StationAcl {
    OwnerOnly,
    Contacts { users: Vec<UserId> },
}

/// 站点信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationInfo {
    pub station: ConvId,
    pub name: String,
    pub host: DeviceId,
    pub owner: UserId,
    pub acl: StationAcl,
    pub created_at: i64,
    pub deleted: bool,
}

/// 条目内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StationContent {
    Text {
        text: String,
    },
    File {
        blob: BlobMeta,
    },
    Folder {
        name: String,
        entries: Vec<FolderEntry>,
    },
}

/// 站点条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationItem {
    pub item: ItemId,
    pub added_by: DeviceId,
    pub added_at: i64,
    pub expires_at: Option<i64>,
    pub content: StationContent,
    pub pinned: bool,
    pub removed: bool,
    pub available: bool,
    pub updated_at: i64,
}

/// Station 帧的子消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StationMsg {
    /// 主机 -> 自己的设备：我托管的站点。
    Announce { stations: Vec<StationInfo> },
    /// 拉取端 -> 主机：要 updated_at > since 的条目。
    Subscribe { station: ConvId, since: i64 },
    /// 主机 -> 拉取端。
    Snapshot {
        info: StationInfo,
        items: Vec<StationItem>,
        now: i64,
    },
    /// 主机主动推送（含 removed = true）。
    Delta {
        station: ConvId,
        items: Vec<StationItem>,
    },
    /// 拉取端 -> 主机。
    Put {
        station: ConvId,
        req: u64,
        item: StationItem,
        mode_copy: bool,
    },
    Remove {
        station: ConvId,
        req: u64,
        item: ItemId,
    },
    SetPinned {
        station: ConvId,
        req: u64,
        item: ItemId,
        pinned: bool,
    },
    /// 主机 -> 拉取端。
    Result {
        req: u64,
        ok: bool,
        error: Option<String>,
    },
}
