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

/// a 64-byte signature，serde 序列化为 bytes（与 32 字节标识类型一致）。
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
                write!(f, "a 64-byte signature")
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                if v.len() != 64 {
                    return Err(E::custom("signature length is not 64 bytes"));
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
    ContactUpdated(ContactRecord),
    ProfileUpdated { display_name: String },
    LocalDeleted { msgs: Vec<MsgId> },
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
    /// 会话元信息：接收方尚无该会话时据此创建。
    pub meta: Option<ConvMeta>,
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

// ===== P2：同步、信任与文件传输类型 =====

/// 线上会话类型（SelfDevices 系统消息与 ContactRecord 引用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConvKindWire {
    SelfDevices,
    Dm,
}

/// 会话元信息（随 SyncOffer 携带，供新设备建会话）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvMeta {
    pub kind: ConvKindWire,
    pub peer_user: Option<UserId>,
}

/// 线上联系人状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactStateWire {
    Outgoing,
    Incoming,
    Accepted,
    Rejected,
    Blocked,
}

/// 联系人记录（配对包与同步携带）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactRecord {
    pub user: UserId,
    pub display_name: String,
    pub state: ContactStateWire,
    pub devices: Vec<DeviceCert>,
    pub auto_accept: bool,
    pub ask_above_bytes: Option<u64>,
    pub updated: Hlc,
}

/// 设备吊销记录，由用户密钥签名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revocation {
    pub user: UserId,
    pub device: DeviceId,
    pub revoked_at: i64,
    pub key_epoch: u32,
    pub sig: Signature64,
}

impl Revocation {
    /// 签发一条吊销记录（sig 由用户密钥覆盖置零后的 CBOR）。
    pub fn issue(user: &crate::sign::UserKey, device: DeviceId, revoked_at: i64) -> Revocation {
        let mut r = Revocation {
            user: user.user_id(),
            device,
            revoked_at,
            key_epoch: 0,
            sig: Signature64([0u8; 64]),
        };
        crate::sign::sign_with_zeroed_sig(&mut r, |b| user.sign(b));
        r
    }

    /// 用记录中的用户公钥验证签名。
    pub fn verify(&self) -> Result<(), crate::sign::SignError> {
        crate::sign::verify_with_zeroed_sig(self, &self.user.0)
    }
}

/// 陌生人联系人请求（经设备转发）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactRequest {
    pub request_id: [u8; 16],
    pub from_display_name: String,
    pub devices: Vec<DeviceCert>,
}

/// 联系人请求的应答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactResponse {
    pub request_id: [u8; 16],
    pub accepted: bool,
    pub devices: Vec<DeviceCert>,
}

/// Blob 查询：列出待取的哈希。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobQuery {
    pub hashes: Vec<BlobHash>,
}

/// Blob 应答：只列出发送方持有的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobHave {
    pub hashes: Vec<BlobHash>,
}

/// 配对方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairMethodWire {
    Qr,
    Sas,
}

/// 配对包：加入方从首设备取得完整身份与数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairBundle {
    pub user_secret: [u8; 32],
    pub display_name: String,
    pub device_cert: DeviceCert,
    pub my_devices: Vec<DeviceCert>,
    pub revocations: Vec<Revocation>,
    pub contacts: Vec<ContactRecord>,
}

/// 配对协议消息（TLS 之上，复用 Frame 通道，类型号 30）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairMsg {
    Begin {
        session: Option<String>,
        method: PairMethodWire,
        device_name: String,
        device_type: DeviceType,
    },
    Proof {
        mac: [u8; 32],
    },
    SasConfirm {
        ok: bool,
    },
    Bundle(Box<PairBundle>),
    Done,
    Abort {
        reason: String,
    },
}

/// 判断消息体是否为隐藏消息（不进会话 UI、仅同步）。
pub fn body_is_hidden(body: &MsgBody) -> bool {
    match body {
        MsgBody::Recall { .. } | MsgBody::Read { .. } => true,
        MsgBody::System { event } => matches!(
            event,
            SystemEvent::ContactUpdated(_)
                | SystemEvent::ProfileUpdated { .. }
                | SystemEvent::LocalDeleted { .. }
        ),
        _ => false,
    }
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

    #[test]
    fn sync_offer_with_meta_roundtrip() {
        let offer = SyncOffer {
            conv: ConvId::dm(&UserId([1u8; 32]), &UserId([2u8; 32])),
            vv: VersionVector(BTreeMap::new()),
            meta: Some(ConvMeta {
                kind: ConvKindWire::Dm,
                peer_user: Some(UserId([2u8; 32])),
            }),
        };
        let decoded: SyncOffer = from_cbor(&to_cbor(&offer)).unwrap();
        assert_eq!(decoded, offer);
    }

    #[test]
    fn hidden_bodies() {
        assert!(body_is_hidden(&MsgBody::Read {
            upto: Hlc {
                millis: 1,
                counter: 0
            }
        }));
        assert!(body_is_hidden(&MsgBody::Recall {
            target: MsgId {
                author: DeviceId([0; 32]),
                seq: 1
            }
        }));
        assert!(body_is_hidden(&MsgBody::System {
            event: SystemEvent::ProfileUpdated {
                display_name: "新名".into()
            }
        }));
        assert!(body_is_hidden(&MsgBody::System {
            event: SystemEvent::LocalDeleted { msgs: vec![] }
        }));
        assert!(body_is_hidden(&MsgBody::System {
            event: SystemEvent::ContactUpdated(ContactRecord {
                user: UserId([0; 32]),
                display_name: "同事".into(),
                state: ContactStateWire::Accepted,
                devices: vec![],
                auto_accept: true,
                ask_above_bytes: None,
                updated: Hlc {
                    millis: 0,
                    counter: 0
                },
            })
        }));
        assert!(!body_is_hidden(&MsgBody::Text { text: "x".into() }));
        assert!(!body_is_hidden(&MsgBody::System {
            event: SystemEvent::DeviceRemoved {
                device: DeviceId([0; 32])
            }
        }));
    }
}
