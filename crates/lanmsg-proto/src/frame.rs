// P1 帧定义与编解码（C4）。
// 线格式：u32 BE 长度 | u16 BE 帧类型号 | CBOR 负载，长度 = 2 + 负载长度，上限 MAX_FRAME_LEN。
// 注释中文，标识符英文。

use crate::sign::to_cbor;
use crate::types::{Ack, DeviceCert, Status, SyncOffer, SyncPush};
use serde::Serialize;

/// 帧类型号（线协议的一部分，永不复用）。
pub const TYPE_HELLO: u16 = 1;
pub const TYPE_PING: u16 = 2;
pub const TYPE_PONG: u16 = 3;
pub const TYPE_SYNC_OFFER: u16 = 10;
pub const TYPE_SYNC_PUSH: u16 = 11;
pub const TYPE_ACK: u16 = 12;
pub const TYPE_PRESENCE: u16 = 13;
pub const TYPE_ERROR: u16 = 99;

/// 帧枚举。P1 阶段定义以下变体，P2/P5 变体由对应阶段追加。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Hello(Hello),                         // 1
    Ping(u64),                            // 2
    Pong(u64),                            // 3
    SyncOffer(SyncOffer),                 // 10
    SyncPush(SyncPush),                   // 11
    Ack(Ack),                             // 12
    Presence(Status),                     // 13
    Error { code: u16, message: String }, // 99
    Unknown(u16),                         // 解码遇到未知类型号时产生
}

/// Hello 握手帧的负载。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Hello {
    pub proto: u16,
    pub caps: Vec<String>,
    pub cert: Option<DeviceCert>,
    pub user_display_name: String,
    pub listen_port: u16,
}

/// 错误帧的 CBOR 负载。
#[derive(Serialize, serde::Deserialize)]
struct ErrorPayload {
    code: u16,
    message: String,
}

/// 帧编解码错误。
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// 帧超过最大长度。
    #[error("帧超过最大长度 {0} 字节")]
    TooLarge(usize),
    /// CBOR 编解码失败。
    #[error("CBOR 编解码错误: {0}")]
    Cbor(String),
    /// IO 错误。
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    /// 数据被截断。
    #[error("数据被截断")]
    Truncated,
}

impl Clone for CodecError {
    fn clone(&self) -> Self {
        match self {
            CodecError::TooLarge(n) => CodecError::TooLarge(*n),
            CodecError::Cbor(s) => CodecError::Cbor(s.clone()),
            CodecError::Io(e) => CodecError::Io(std::io::Error::other(e.to_string())),
            CodecError::Truncated => CodecError::Truncated,
        }
    }
}

impl PartialEq for CodecError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CodecError::TooLarge(a), CodecError::TooLarge(b)) => a == b,
            (CodecError::Cbor(a), CodecError::Cbor(b)) => a == b,
            (CodecError::Truncated, CodecError::Truncated) => true,
            _ => false,
        }
    }
}

impl Eq for CodecError {}

impl Frame {
    /// 返回该帧在线协议中的类型号。
    pub fn type_id(&self) -> u16 {
        match self {
            Frame::Hello(_) => TYPE_HELLO,
            Frame::Ping(_) => TYPE_PING,
            Frame::Pong(_) => TYPE_PONG,
            Frame::SyncOffer(_) => TYPE_SYNC_OFFER,
            Frame::SyncPush(_) => TYPE_SYNC_PUSH,
            Frame::Ack(_) => TYPE_ACK,
            Frame::Presence(_) => TYPE_PRESENCE,
            Frame::Error { .. } => TYPE_ERROR,
            Frame::Unknown(id) => *id,
        }
    }

    /// CBOR 序列化该帧的负载（不含类型号和长度头）。
    pub fn encode_payload(&self) -> Vec<u8> {
        match self {
            Frame::Hello(hello) => to_cbor(hello),
            Frame::Ping(n) => to_cbor(n),
            Frame::Pong(n) => to_cbor(n),
            Frame::SyncOffer(offer) => to_cbor(offer),
            Frame::SyncPush(push) => to_cbor(push),
            Frame::Ack(ack) => to_cbor(ack),
            Frame::Presence(status) => to_cbor(status),
            Frame::Error { code, message } => to_cbor(&ErrorPayload {
                code: *code,
                message: message.clone(),
            }),
            Frame::Unknown(_) => Vec::new(),
        }
    }

    /// 根据类型号和 CBOR 负载解码帧。未知类型号返回 `Ok(Frame::Unknown(type_id))`。
    pub fn decode(type_id: u16, payload: &[u8]) -> Result<Frame, CodecError> {
        match type_id {
            TYPE_HELLO => {
                let hello: Hello =
                    ciborium::from_reader(payload).map_err(|e| CodecError::Cbor(e.to_string()))?;
                Ok(Frame::Hello(hello))
            }
            TYPE_PING => {
                let n: u64 =
                    ciborium::from_reader(payload).map_err(|e| CodecError::Cbor(e.to_string()))?;
                Ok(Frame::Ping(n))
            }
            TYPE_PONG => {
                let n: u64 =
                    ciborium::from_reader(payload).map_err(|e| CodecError::Cbor(e.to_string()))?;
                Ok(Frame::Pong(n))
            }
            TYPE_SYNC_OFFER => {
                let offer: SyncOffer =
                    ciborium::from_reader(payload).map_err(|e| CodecError::Cbor(e.to_string()))?;
                Ok(Frame::SyncOffer(offer))
            }
            TYPE_SYNC_PUSH => {
                let push: SyncPush =
                    ciborium::from_reader(payload).map_err(|e| CodecError::Cbor(e.to_string()))?;
                Ok(Frame::SyncPush(push))
            }
            TYPE_ACK => {
                let ack: Ack =
                    ciborium::from_reader(payload).map_err(|e| CodecError::Cbor(e.to_string()))?;
                Ok(Frame::Ack(ack))
            }
            TYPE_PRESENCE => {
                let status: Status =
                    ciborium::from_reader(payload).map_err(|e| CodecError::Cbor(e.to_string()))?;
                Ok(Frame::Presence(status))
            }
            TYPE_ERROR => {
                let err: ErrorPayload =
                    ciborium::from_reader(payload).map_err(|e| CodecError::Cbor(e.to_string()))?;
                Ok(Frame::Error {
                    code: err.code,
                    message: err.message,
                })
            }
            _ => Ok(Frame::Unknown(type_id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::FrameCodec;
    use crate::consts::MAX_FRAME_LEN;
    use bytes::{BufMut, BytesMut};
    use tokio_util::codec::Decoder;

    #[test]
    fn roundtrip_ping_and_hello() {
        // Ping(7)
        let ping = Frame::Ping(7);
        let payload = ping.encode_payload();
        let decoded = Frame::decode(2, &payload).unwrap();
        assert_eq!(decoded, ping);
        assert_eq!(decoded.type_id(), 2);

        // Hello with cert: None
        let hello = Frame::Hello(Hello {
            proto: 1,
            caps: vec!["sync/1".into(), "blob/1".into()],
            cert: None,
            user_display_name: "test".into(),
            listen_port: 53320,
        });
        let payload = hello.encode_payload();
        let decoded = Frame::decode(1, &payload).unwrap();
        assert_eq!(decoded, hello);
        assert_eq!(decoded.type_id(), 1);
    }

    #[test]
    fn unknown_type_id_decodes_to_unknown() {
        let mut buf = BytesMut::new();
        buf.put_u32(2 + 1);
        buf.put_u16(777);
        buf.put_u8(0xa0); // CBOR 空 map
        assert_eq!(
            FrameCodec.decode(&mut buf).unwrap(),
            Some(Frame::Unknown(777))
        );
    }

    #[test]
    fn rejects_frame_larger_than_max() {
        let mut buf = BytesMut::new();
        buf.put_u32((MAX_FRAME_LEN + 1) as u32);
        assert!(matches!(
            FrameCodec.decode(&mut buf),
            Err(CodecError::TooLarge(_))
        ));
    }

    #[test]
    fn partial_frame_waits_for_more_bytes() {
        let hello = Frame::Hello(Hello {
            proto: 1,
            caps: vec![],
            cert: None,
            user_display_name: "test".into(),
            listen_port: 53320,
        });
        let payload = hello.encode_payload();
        let len = 2 + payload.len();

        let mut buf = BytesMut::new();
        buf.put_u32(len as u32);
        buf.put_u16(1);
        // 只写入一半负载
        let half = payload.len() / 2;
        buf.extend_from_slice(&payload[..half]);
        assert_eq!(FrameCodec.decode(&mut buf).unwrap(), None);

        // 补齐剩余负载
        buf.extend_from_slice(&payload[half..]);
        assert_eq!(FrameCodec.decode(&mut buf).unwrap(), Some(hello));
    }

    #[test]
    fn type_id_values_match_spec() {
        assert_eq!(
            Frame::Hello(Hello {
                proto: 1,
                caps: vec![],
                cert: None,
                user_display_name: "t".into(),
                listen_port: 0,
            })
            .type_id(),
            1
        );
        assert_eq!(Frame::Ping(0).type_id(), 2);
        assert_eq!(Frame::Pong(0).type_id(), 3);
        assert_eq!(
            Frame::SyncOffer(crate::types::SyncOffer {
                conv: crate::ids::ConvId([0u8; 32]),
                vv: crate::types::VersionVector(std::collections::BTreeMap::new()),
            })
            .type_id(),
            10
        );
        assert_eq!(
            Frame::Error {
                code: 1,
                message: "e".into()
            }
            .type_id(),
            99
        );
        assert_eq!(Frame::Unknown(777).type_id(), 777);
    }
}
