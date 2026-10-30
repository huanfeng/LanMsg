// P1 核心骨架：签名与编解码（C3/C4）。
// 签名规则：sig 覆盖“把 sig 字段置为全零后整个结构的 CBOR 编码”。
// 注释中文，标识符英文。

use ed25519_dalek::{pkcs8::EncodePrivateKey, Signer, SigningKey, Verifier, VerifyingKey};
use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;

use crate::ids::{DeviceId, UserId};
use crate::types::{Announce, DeviceCert, Envelope, Revocation, Signature64};

/// 签名与密钥错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignError {
    /// 公钥字节不合法。
    #[error("无效的公钥字节")]
    BadKey,
    /// 签名不合法或验证失败。
    #[error("签名验证失败")]
    BadSignature,
}

/// CBOR 编解码错误（Task 4 复用同一枚举）。
#[derive(Debug, Error)]
pub enum CodecError {
    /// 帧超过最大长度。
    #[error("帧超过最大长度 {0} 字节")]
    TooLarge(usize),
    /// CBOR 编解码失败。
    #[error("CBOR 编解码错误: {0}")]
    Cbor(String),
    /// 数据被截断。
    #[error("数据被截断")]
    Truncated,
}

/// 可签名的协议结构。
pub trait Signed: Serialize + Clone {
    fn sig_mut(&mut self) -> &mut Signature64;
    fn sig(&self) -> &Signature64;
}

/// 设备密钥（Ed25519）。
pub struct DeviceKey(SigningKey);

/// 用户密钥（Ed25519）。
pub struct UserKey(SigningKey);

macro_rules! signing_key {
    ($Type:ident, $Id:ident, $id_method:ident) => {
        impl $Type {
            /// 随机生成密钥。
            pub fn generate() -> Self {
                Self(SigningKey::generate(&mut rand::thread_rng()))
            }

            /// 从 32 字节种子恢复密钥。
            pub fn from_bytes(bytes: &[u8; 32]) -> Self {
                Self(SigningKey::from_bytes(bytes))
            }

            /// 导出 32 字节种子。
            pub fn to_bytes(&self) -> [u8; 32] {
                self.0.to_bytes()
            }

            /// 对应的身份（设备/用户公钥字节）。
            pub fn $id_method(&self) -> $Id {
                $Id(self.0.verifying_key().to_bytes())
            }

            /// 对消息签名，返回 64 字节签名。
            pub fn sign(&self, msg: &[u8]) -> Signature64 {
                let signature = self.0.sign(msg);
                Signature64(signature.to_bytes())
            }

            /// 导出 PKCS8 DER 私钥。
            pub fn to_pkcs8_der(&self) -> Vec<u8> {
                match self.0.to_pkcs8_der() {
                    Ok(der) => der.as_bytes().to_vec(),
                    Err(_) => Vec::new(),
                }
            }
        }
    };
}

signing_key!(DeviceKey, DeviceId, device_id);
signing_key!(UserKey, UserId, user_id);

/// 用公钥验证签名。
pub fn verify(pubkey: &[u8; 32], msg: &[u8], sig: &Signature64) -> Result<(), SignError> {
    let verifying_key = VerifyingKey::from_bytes(pubkey).map_err(|_| SignError::BadKey)?;
    let signature = ed25519_dalek::Signature::from_bytes(&sig.0);
    verifying_key
        .verify(msg, &signature)
        .map_err(|_| SignError::BadSignature)?;
    Ok(())
}

/// 把 `value` 的 sig 字段置零，序列化为 CBOR，再签名并写回 sig 字段。
pub fn sign_with_zeroed_sig<T: Signed>(value: &mut T, sign: impl FnOnce(&[u8]) -> Signature64) {
    *value.sig_mut() = Signature64([0u8; 64]);
    let encoded = to_cbor(value);
    *value.sig_mut() = sign(&encoded);
}

/// 把 `value` 的 sig 字段置零后序列化为 CBOR，用公钥验证其 sig 字段。
pub fn verify_with_zeroed_sig<T: Signed>(value: &T, pubkey: &[u8; 32]) -> Result<(), SignError> {
    let mut cloned = value.clone();
    *cloned.sig_mut() = Signature64([0u8; 64]);
    let encoded = to_cbor(&cloned);
    verify(pubkey, &encoded, value.sig())
}

/// CBOR 序列化（写入 Vec 永不失败，异常分支为不可达）。
pub fn to_cbor<T: Serialize>(v: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    match ciborium::into_writer(v, &mut buf) {
        Ok(()) => buf,
        Err(e) => panic!("CBOR 编码失败: {e}"),
    }
}

/// CBOR 反序列化。
pub fn from_cbor<T: DeserializeOwned>(b: &[u8]) -> Result<T, CodecError> {
    ciborium::from_reader(b).map_err(|e| CodecError::Cbor(e.to_string()))
}

impl Signed for DeviceCert {
    fn sig_mut(&mut self) -> &mut Signature64 {
        &mut self.sig
    }
    fn sig(&self) -> &Signature64 {
        &self.sig
    }
}

impl Signed for Envelope {
    fn sig_mut(&mut self) -> &mut Signature64 {
        &mut self.sig
    }
    fn sig(&self) -> &Signature64 {
        &self.sig
    }
}

impl Signed for Announce {
    fn sig_mut(&mut self) -> &mut Signature64 {
        &mut self.sig
    }
    fn sig(&self) -> &Signature64 {
        &self.sig
    }
}

impl Signed for Revocation {
    fn sig_mut(&mut self) -> &mut Signature64 {
        &mut self.sig
    }
    fn sig(&self) -> &Signature64 {
        &self.sig
    }
}

impl DeviceCert {
    /// 由用户密钥签发设备证书，key_epoch = 0。
    pub fn issue(
        user: &UserKey,
        device: DeviceId,
        device_name: String,
        device_type: crate::types::DeviceType,
        created_at: i64,
    ) -> DeviceCert {
        let mut cert = DeviceCert {
            device,
            user: user.user_id(),
            device_name,
            device_type,
            created_at,
            key_epoch: 0,
            sig: Signature64([0u8; 64]),
        };
        sign_with_zeroed_sig(&mut cert, |b| user.sign(b));
        cert
    }

    /// 用用户公钥验证证书自身签名。
    pub fn verify(&self) -> Result<(), SignError> {
        verify_with_zeroed_sig(self, &self.user.0)
    }
}

impl Envelope {
    /// 用消息作者设备公钥验证信封签名。
    pub fn verify(&self) -> Result<(), SignError> {
        verify_with_zeroed_sig(self, &self.id.author.0)
    }
}

impl Announce {
    /// 先验证随带证书，再用证书中的设备公钥验证通告自身签名。
    pub fn verify(&self) -> Result<(), SignError> {
        self.cert.verify()?;
        verify_with_zeroed_sig(self, &self.cert.device.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ConvId, MsgId};
    use crate::types::{DeviceType, Hlc, MsgBody};

    #[test]
    fn revocation_signed_by_user_verifies() {
        let u = UserKey::generate();
        let d = DeviceKey::generate().device_id();
        let mut r = Revocation::issue(&u, d, 1);
        assert!(r.verify().is_ok());
        r.device = DeviceKey::generate().device_id();
        assert!(r.verify().is_err());
    }

    #[test]
    fn cert_issued_by_user_verifies_and_tamper_fails() {
        let user = UserKey::generate();
        let dev = DeviceKey::generate();
        let mut cert = DeviceCert::issue(
            &user,
            dev.device_id(),
            "台式机".into(),
            DeviceType::Desktop,
            1_700_000_000_000,
        );
        assert!(cert.verify().is_ok());
        cert.device_name = "别人".into();
        assert!(cert.verify().is_err());
    }

    #[test]
    fn envelope_signed_by_author_verifies() {
        let dev = DeviceKey::generate();
        let mut env = Envelope {
            conv: ConvId([0u8; 32]),
            id: MsgId {
                author: dev.device_id(),
                seq: 1,
            },
            conv_seq: 1,
            hlc: Hlc {
                millis: 1,
                counter: 0,
            },
            body: MsgBody::Text {
                text: "你好".into(),
            },
            enc: None,
            sig: Signature64([0u8; 64]),
        };
        sign_with_zeroed_sig(&mut env, |b| dev.sign(b));
        assert!(env.verify().is_ok());
        let mut other = env.clone();
        other.id.author = DeviceKey::generate().device_id();
        assert!(other.verify().is_err());
    }

    #[test]
    fn cbor_roundtrip_envelope() {
        let dev = DeviceKey::generate();
        let mut env = Envelope {
            conv: ConvId([0u8; 32]),
            id: MsgId {
                author: dev.device_id(),
                seq: 1,
            },
            conv_seq: 1,
            hlc: Hlc {
                millis: 1,
                counter: 0,
            },
            body: MsgBody::Text {
                text: "你好".into(),
            },
            enc: None,
            sig: Signature64([0u8; 64]),
        };
        sign_with_zeroed_sig(&mut env, |b| dev.sign(b));
        assert_eq!(from_cbor::<Envelope>(&to_cbor(&env)).unwrap(), env);
    }
}
