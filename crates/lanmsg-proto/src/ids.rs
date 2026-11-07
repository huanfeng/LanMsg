// P1 核心骨架：协议标识类型与常量（C2）。
// 四个 32 字节类型（UserId、DeviceId、ConvId、BlobHash）通过声明宏 id32! 生成公共实现，避免重复代码。
// serde 对 32 字节类型序列化为 CBOR bytes（serialize_bytes），反序列化校验长度 32。
// MsgId 字符串形式为 "{author_hex}:{seq}"，Display / FromStr 互逆。
// 注释中文，标识符英文。

use std::fmt;
use std::str::FromStr;

use blake3::Hasher;
use data_encoding::{BASE32_NOPAD, HEXLOWER};
use rand::Rng;
use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// 标识解析错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    /// 十六进制字符串包含非法字符。
    #[error("invalid hex encoding")]
    BadHex,
    /// 字节长度不为 32。
    #[error("length is not 32 bytes")]
    BadLength,
    /// MsgId 字符串格式不符合 "{author_hex}:{seq}"。
    #[error("invalid MsgId string")]
    BadMsgId,
}

/// 32 字节标识类型的公共实现：
/// Copy/Clone/Eq/Ord/Hash/Serialize/Deserialize/Display，以及 to_hex/from_hex。
macro_rules! id32 {
    ($Name:ident) => {
        impl Copy for $Name {}
        impl Clone for $Name {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl PartialEq for $Name {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }
        impl Eq for $Name {}
        impl PartialOrd for $Name {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for $Name {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.0.cmp(&other.0)
            }
        }
        impl std::hash::Hash for $Name {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }
        impl fmt::Debug for $Name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($Name), self.to_hex())
            }
        }
        impl Serialize for $Name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_bytes(&self.0)
            }
        }
        impl<'de> Deserialize<'de> for $Name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct IdVisitor;
                impl<'de> de::Visitor<'de> for IdVisitor {
                    type Value = $Name;
                    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                        write!(f, "a 32-byte string")
                    }
                    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                        if v.len() != 32 {
                            return Err(E::custom(IdError::BadLength));
                        }
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(v);
                        Ok($Name(arr))
                    }
                }
                deserializer.deserialize_bytes(IdVisitor)
            }
        }
        impl fmt::Display for $Name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.to_hex())
            }
        }
        impl $Name {
            /// 小写十六进制编码（64 字符）。
            pub fn to_hex(&self) -> String {
                HEXLOWER.encode(&self.0)
            }
            /// 从小写十六进制解析，长度必须为 32 字节。
            pub fn from_hex(s: &str) -> Result<Self, IdError> {
                let bytes = HEXLOWER.decode(s.as_bytes()).map_err(|_| IdError::BadHex)?;
                if bytes.len() != 32 {
                    return Err(IdError::BadLength);
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Ok($Name(arr))
            }
        }
    };
}

/// 用户身份，等于用户 Ed25519 公钥字节。
pub struct UserId(pub [u8; 32]);

/// 设备身份，等于设备 Ed25519 公钥字节。
pub struct DeviceId(pub [u8; 32]);

/// 会话身份。
pub struct ConvId(pub [u8; 32]);

/// Blob 哈希，BLAKE3 输出。
pub struct BlobHash(pub [u8; 32]);

id32!(UserId);
id32!(DeviceId);
id32!(ConvId);
id32!(BlobHash);

/// base32_nopad_lower(blake3(bytes)[..16])，26 字符，用于界面显示指纹。
fn short_fingerprint(bytes: &[u8; 32]) -> String {
    let hash = blake3::hash(bytes);
    let encoded = BASE32_NOPAD.encode(&hash.as_bytes()[..16]);
    encoded.to_lowercase()
}

impl UserId {
    /// base32_nopad_lower(blake3(self)[..16])，26 字符，用于界面显示指纹。
    pub fn short(&self) -> String {
        short_fingerprint(&self.0)
    }
}

impl DeviceId {
    /// base32_nopad_lower(blake3(self)[..16])，26 字符，用于界面显示指纹。
    pub fn short(&self) -> String {
        short_fingerprint(&self.0)
    }
}

impl ConvId {
    /// DM 会话：blake3("lanmsg-dm" || min(a, b) || max(a, b))，与对端顺序无关。
    pub fn dm(a: &UserId, b: &UserId) -> ConvId {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let mut hasher = Hasher::new();
        hasher.update(b"lanmsg-dm");
        hasher.update(&lo.0);
        hasher.update(&hi.0);
        let hash = hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(hash.as_bytes());
        ConvId(arr)
    }

    /// 自身设备会话：blake3("lanmsg-self" || u)。
    pub fn self_devices(u: &UserId) -> ConvId {
        let mut hasher = Hasher::new();
        hasher.update(b"lanmsg-self");
        hasher.update(&u.0);
        let hash = hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(hash.as_bytes());
        ConvId(arr)
    }

    /// LocalSend 会话：blake3("lanmsg-ls" || fingerprint.as_bytes())（P5 使用）。
    pub fn localsend(fingerprint: &str) -> ConvId {
        let mut hasher = Hasher::new();
        hasher.update(b"lanmsg-ls");
        hasher.update(fingerprint.as_bytes());
        let hash = hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(hash.as_bytes());
        ConvId(arr)
    }

    /// 随机会话 ID（访客、中转站用）。
    pub fn random() -> ConvId {
        let mut arr = [0u8; 32];
        rand::thread_rng().fill(&mut arr);
        ConvId(arr)
    }
}

/// 消息 ID，字符串形式为 "{author_hex}:{seq}"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgId {
    pub author: DeviceId,
    pub seq: u64,
}

impl fmt::Display for MsgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.author, self.seq)
    }
}

impl FromStr for MsgId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (author_hex, seq_str) = s.rsplit_once(':').ok_or(IdError::BadMsgId)?;
        let author = DeviceId::from_hex(author_hex)?;
        let seq = seq_str.parse::<u64>().map_err(|_| IdError::BadMsgId)?;
        Ok(MsgId { author, seq })
    }
}

impl Serialize for MsgId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for MsgId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MsgIdVisitor;
        impl<'de> de::Visitor<'de> for MsgIdVisitor {
            type Value = MsgId;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a MsgId string in the form <author_hex>:<seq>")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse::<MsgId>().map_err(E::custom)
            }
        }
        deserializer.deserialize_str(MsgIdVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_hex_roundtrip() {
        let id = DeviceId([7u8; 32]);
        assert_eq!(id.to_hex().len(), 64);
        assert_eq!(DeviceId::from_hex(&id.to_hex()).unwrap(), id);
    }

    #[test]
    fn from_hex_rejects_wrong_length() {
        assert!(matches!(
            DeviceId::from_hex("abcd"),
            Err(IdError::BadLength)
        ));
    }

    #[test]
    fn short_is_26_lowercase_base32() {
        let s = UserId([1u8; 32]).short();
        assert_eq!(s.len(), 26);
        assert!(s
            .chars()
            .all(|c| c.is_ascii_lowercase() || ('2'..='7').contains(&c)));
    }

    #[test]
    fn dm_conv_id_is_order_independent() {
        let a = UserId([1u8; 32]);
        let b = UserId([2u8; 32]);
        assert_eq!(ConvId::dm(&a, &b), ConvId::dm(&b, &a));
        assert_ne!(ConvId::dm(&a, &b), ConvId::self_devices(&a));
    }

    #[test]
    fn msg_id_string_roundtrip() {
        let m = MsgId {
            author: DeviceId([9u8; 32]),
            seq: 42,
        };
        let s = m.to_string();
        assert!(s.ends_with(":42"));
        assert_eq!(s.parse::<MsgId>().unwrap(), m);
        assert!("zz:1".parse::<MsgId>().is_err());
    }
}
