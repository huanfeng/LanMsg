// P1 核心骨架：IPC 帧与编解码（C7）。
// 线格式：u32 BE 长度 | CBOR(IpcFrame)，长度上限 MAX_FRAME_LEN。
// 注释中文，标识符英文。

use bytes::{Buf, BytesMut};
use lanmsg_proto::consts::MAX_FRAME_LEN;
use tokio_util::codec::{Decoder, Encoder};

use crate::types::IpcFrame;

/// IPC 编解码错误。
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// 帧长度超过上限。
    #[error("帧长度 {0} 超过上限 {MAX_FRAME_LEN}")]
    TooLarge(usize),
    /// CBOR 编解码失败。
    #[error("CBOR 错误: {0}")]
    Cbor(String),
    /// IO 错误。
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

/// IPC 编解码器：u32 BE 长度 | CBOR(IpcFrame)。
pub struct IpcCodec;

impl Encoder<IpcFrame> for IpcCodec {
    type Error = IpcError;

    fn encode(&mut self, item: IpcFrame, dst: &mut BytesMut) -> Result<(), IpcError> {
        let mut buf = Vec::new();
        ciborium::into_writer(&item, &mut buf).map_err(|e| IpcError::Cbor(e.to_string()))?;
        if buf.len() > MAX_FRAME_LEN {
            return Err(IpcError::TooLarge(buf.len()));
        }
        dst.extend_from_slice(&(buf.len() as u32).to_be_bytes());
        dst.extend_from_slice(&buf);
        Ok(())
    }
}

impl Decoder for IpcCodec {
    type Item = IpcFrame;
    type Error = IpcError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<IpcFrame>, IpcError> {
        if src.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;
        if len > MAX_FRAME_LEN {
            return Err(IpcError::TooLarge(len));
        }
        if src.len() < 4 + len {
            return Ok(None);
        }
        src.advance(4);
        let frame_bytes = src.split_to(len);
        let frame: IpcFrame =
            ciborium::from_reader(&frame_bytes[..]).map_err(|e| IpcError::Cbor(e.to_string()))?;
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ApiRequest, Event, IpcFrame, Request, Response};
    use lanmsg_proto::consts::MAX_FRAME_LEN;
    use tokio_util::codec::Decoder;

    #[test]
    fn ipc_frame_roundtrip_request_and_event() {
        let frames = vec![
            IpcFrame::Request(ApiRequest {
                id: 1,
                body: Request::SendText {
                    conv: "ab".into(),
                    text: "你好".into(),
                },
            }),
            IpcFrame::Event(Event::PeerOffline {
                device: "cd".into(),
            }),
        ];

        let mut codec = IpcCodec;
        for frame in &frames {
            let mut dst = BytesMut::new();
            codec.encode(frame.clone(), &mut dst).expect("编码应成功");
            let mut src = dst;
            let decoded = codec.decode(&mut src).expect("解码应成功");
            assert_eq!(decoded, Some(frame.clone()));
        }
    }

    #[test]
    fn response_uses_named_value_field() {
        let r = Response::MsgId {
            value: "x:1".into(),
        };
        assert_eq!(
            r,
            Response::MsgId {
                value: "x:1".into()
            }
        );
    }

    #[test]
    fn ipc_rejects_oversized_frame() {
        let mut src = BytesMut::new();
        // 长度头为 MAX_FRAME_LEN + 1，无有效负载
        src.extend_from_slice(&((MAX_FRAME_LEN + 1) as u32).to_be_bytes());
        let mut codec = IpcCodec;
        let result = codec.decode(&mut src);
        assert!(matches!(result, Err(IpcError::TooLarge(_))));
    }
}
