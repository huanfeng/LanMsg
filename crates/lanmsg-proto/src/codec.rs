// P1 帧编解码器（C4）。
// 线格式：u32 BE 长度 | u16 BE 帧类型号 | CBOR 负载，长度 = 2 + 负载长度，上限 MAX_FRAME_LEN。
// 注释中文，标识符英文。

use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::consts::MAX_FRAME_LEN;
use crate::frame::{CodecError, Frame};

/// 帧编解码器。每帧线格式为 u32 BE 长度（= 2 + 负载字节数）| u16 BE 类型号 | CBOR 负载。
pub struct FrameCodec;

impl Encoder<Frame> for FrameCodec {
    type Error = CodecError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload = item.encode_payload();
        let frame_len = 2 + payload.len();
        if frame_len > MAX_FRAME_LEN {
            return Err(CodecError::TooLarge(frame_len));
        }
        dst.put_u32(frame_len as u32);
        dst.put_u16(item.type_id());
        dst.extend_from_slice(&payload);
        Ok(())
    }
}

impl Decoder for FrameCodec {
    type Item = Frame;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // 需要至少 4 字节长度头才能读取长度字段。
        if src.len() < 4 {
            return Ok(None);
        }

        // 读取长度头（不消费字节）。
        let frame_len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;
        if frame_len > MAX_FRAME_LEN {
            // 长度头超过上限，立即报错，不等数据到齐。
            return Err(CodecError::TooLarge(frame_len));
        }
        if frame_len < 2 {
            // 需要凑够完整头部（4 字节长度 + 2 字节类型号）后才能判定 Truncated。
            if src.len() < 6 {
                return Ok(None);
            }
            return Err(CodecError::Truncated);
        }

        let total_len = 4 + frame_len;
        if src.len() < total_len {
            return Ok(None);
        }

        // 消费长度头。
        src.advance(4);
        let type_id = u16::from_be_bytes([src[0], src[1]]);
        src.advance(2);
        let payload = src.split_to(frame_len - 2).freeze();

        // 类型号已知则解码为对应变体，未知则返回 Unknown。
        match Frame::decode(type_id, &payload) {
            Ok(frame) => Ok(Some(frame)),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Frame, Hello};
    use crate::ids::ConvId;
    use crate::types::{Ack, Status, SyncOffer, SyncPush, VersionVector};
    use bytes::BytesMut;

    #[test]
    fn encode_decode_roundtrip() {
        let frames: Vec<Frame> = vec![
            Frame::Ping(42),
            Frame::Pong(42),
            Frame::Hello(Hello {
                proto: 1,
                caps: vec!["sync/1".into()],
                cert: None,
                user_display_name: "alice".into(),
                listen_port: 53320,
            }),
            Frame::SyncOffer(SyncOffer {
                conv: ConvId([0u8; 32]),
                vv: VersionVector(std::collections::BTreeMap::new()),
                meta: None,
            }),
            Frame::SyncPush(SyncPush { envelopes: vec![] }),
            Frame::Ack(Ack { ids: vec![] }),
            Frame::Presence(Status::Online),
            Frame::Error {
                code: 1,
                message: "bad".into(),
            },
        ];

        for frame in frames {
            let mut buf = BytesMut::new();
            FrameCodec.encode(frame.clone(), &mut buf).unwrap();
            let decoded = FrameCodec.decode(&mut buf).unwrap();
            assert_eq!(decoded, Some(frame));
        }
    }

    #[test]
    fn decode_unknown_type_id() {
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
    fn decode_rejects_oversized_length_header() {
        let mut buf = BytesMut::new();
        buf.put_u32((MAX_FRAME_LEN + 1) as u32);
        // 数据体尚未到齐，但长度头已超上限，应立即 TooLarge。
        assert!(matches!(
            FrameCodec.decode(&mut buf),
            Err(CodecError::TooLarge(_))
        ));
    }

    #[test]
    fn decode_partial_frame_returns_none() {
        let frame = Frame::Ping(7);
        let mut buf = BytesMut::new();
        FrameCodec.encode(frame.clone(), &mut buf).unwrap();
        // 只保留长度头和类型号，负载不全。
        buf.truncate(6);
        assert_eq!(FrameCodec.decode(&mut buf).unwrap(), None);
    }

    #[test]
    fn decode_length_less_than_two_is_truncated() {
        // 长度字段为 1（<2），给足 6 字节头部以便读取长度字段，
        // 应返回 Truncated 而非等待更多数据。
        let mut buf = BytesMut::new();
        buf.put_u32(1);
        buf.put_u16(0); // 占位，凑够 6 字节头部
        assert_eq!(FrameCodec.decode(&mut buf), Err(CodecError::Truncated));
    }
}
