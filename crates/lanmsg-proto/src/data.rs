// P2 数据连接帧（Task 1）。
// 与控制连接 Frame 独立的类型号空间；线格式与 C4 相同：
// u32 BE 长度 | u16 BE 类型号 | CBOR，上限 MAX_FRAME_LEN。
// `RangeStart` 之后的原始字节由调用方直接 read_exact/write_all，不经过帧。
// 数据连接不容忍未知帧：未知类型号返回 CodecError::Cbor("unknown data frame")。
// 注释中文，标识符英文。

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::consts::MAX_FRAME_LEN;
use crate::frame::CodecError;
use crate::ids::BlobHash;
use crate::sign::{from_cbor, to_cbor};
use crate::types::DeviceCert;

/// 数据帧类型号。
pub const TYPE_DATA_HELLO: u16 = 1;
pub const TYPE_GET_META: u16 = 2;
pub const TYPE_META: u16 = 3;
pub const TYPE_NOT_FOUND: u16 = 4;
pub const TYPE_GET_RANGE: u16 = 5;
pub const TYPE_RANGE_START: u16 = 6;
pub const TYPE_DIAL_BACK: u16 = 7;
pub const TYPE_DATA_ERROR: u16 = 99;

/// 数据连接帧（独立于控制连接的 `Frame` 类型号空间）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataFrame {
    /// 连接后首帧：出示设备证书。
    DataHello { cert: DeviceCert }, // 1
    /// 查询 Blob 元数据。
    GetMeta { hash: BlobHash }, // 2
    /// Blob 元数据应答。
    Meta {
        hash: BlobHash,
        size: u64,
        segment_hashes: Vec<[u8; 32]>,
    }, // 3
    /// 无此 Blob。
    NotFound { hash: BlobHash }, // 4
    /// 请求一段数据。
    GetRange {
        hash: BlobHash,
        offset: u64,
        len: u64,
    }, // 5
    /// 段开始；之后紧跟 len 个原始字节（不经过帧）。
    RangeStart { offset: u64, len: u64 }, // 6
    /// 反向建立的数据连接：在 DataHello 之后出示请求方给的一次性令牌。
    DialBack { token: [u8; 16] }, // 7
    /// 错误。
    Error { code: u16, message: String }, // 99
}

impl DataFrame {
    /// 返回该帧在数据连接协议中的类型号。
    pub fn type_id(&self) -> u16 {
        match self {
            DataFrame::DataHello { .. } => TYPE_DATA_HELLO,
            DataFrame::GetMeta { .. } => TYPE_GET_META,
            DataFrame::Meta { .. } => TYPE_META,
            DataFrame::NotFound { .. } => TYPE_NOT_FOUND,
            DataFrame::GetRange { .. } => TYPE_GET_RANGE,
            DataFrame::RangeStart { .. } => TYPE_RANGE_START,
            DataFrame::DialBack { .. } => TYPE_DIAL_BACK,
            DataFrame::Error { .. } => TYPE_DATA_ERROR,
        }
    }

    /// CBOR 序列化负载。
    fn encode_payload(&self) -> Vec<u8> {
        match self {
            DataFrame::DataHello { cert } => to_cbor(cert),
            DataFrame::GetMeta { hash } => to_cbor(hash),
            DataFrame::Meta {
                hash,
                size,
                segment_hashes,
            } => to_cbor(&(hash, size, segment_hashes)),
            DataFrame::NotFound { hash } => to_cbor(hash),
            DataFrame::GetRange { hash, offset, len } => to_cbor(&(hash, offset, len)),
            DataFrame::RangeStart { offset, len } => to_cbor(&(*offset, *len)),
            DataFrame::DialBack { token } => to_cbor(token),
            DataFrame::Error { code, message } => to_cbor(&(code, message)),
        }
    }

    /// 根据类型号与负载解码。未知类型号返回 `CodecError::Cbor("unknown data frame")`。
    pub fn decode(type_id: u16, payload: &[u8]) -> Result<DataFrame, CodecError> {
        let fail = || CodecError::Cbor("unknown data frame".into());
        Ok(match type_id {
            TYPE_DATA_HELLO => DataFrame::DataHello {
                cert: from_cbor(payload).map_err(|_| fail())?,
            },
            TYPE_GET_META => DataFrame::GetMeta {
                hash: from_cbor(payload).map_err(|_| fail())?,
            },
            TYPE_META => {
                let (hash, size, segment_hashes) = from_cbor(payload).map_err(|_| fail())?;
                DataFrame::Meta {
                    hash,
                    size,
                    segment_hashes,
                }
            }
            TYPE_NOT_FOUND => DataFrame::NotFound {
                hash: from_cbor(payload).map_err(|_| fail())?,
            },
            TYPE_GET_RANGE => {
                let (hash, offset, len) = from_cbor(payload).map_err(|_| fail())?;
                DataFrame::GetRange { hash, offset, len }
            }
            TYPE_RANGE_START => {
                let (offset, len) = from_cbor(payload).map_err(|_| fail())?;
                DataFrame::RangeStart { offset, len }
            }
            TYPE_DIAL_BACK => DataFrame::DialBack {
                token: from_cbor(payload).map_err(|_| fail())?,
            },
            TYPE_DATA_ERROR => {
                let (code, message) = from_cbor(payload).map_err(|_| fail())?;
                DataFrame::Error { code, message }
            }
            _ => return Err(fail()),
        })
    }
}

/// 写一个数据帧（不含 RangeStart 之后的原始字节）。
pub async fn write_data_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    f: &DataFrame,
) -> Result<(), CodecError> {
    let payload = f.encode_payload();
    if payload.len() + 2 > MAX_FRAME_LEN {
        return Err(CodecError::TooLarge(payload.len() + 2));
    }
    let header = ((payload.len() + 2) as u32).to_be_bytes();
    w.write_all(&header).await.map_err(CodecError::Io)?;
    w.write_all(&f.type_id().to_be_bytes())
        .await
        .map_err(CodecError::Io)?;
    w.write_all(&payload).await.map_err(CodecError::Io)?;
    Ok(())
}

/// 读一个数据帧。
pub async fn read_data_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<DataFrame, CodecError> {
    let mut header = [0u8; 4];
    r.read_exact(&mut header)
        .await
        .map_err(|e| truncated_or_io(e, &header, 4))?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_LEN {
        return Err(CodecError::TooLarge(len));
    }
    if len < 2 {
        return Err(CodecError::Truncated);
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)
        .await
        .map_err(|e| truncated_or_io(e, &buf, len))?;
    let type_id = u16::from_be_bytes([buf[0], buf[1]]);
    DataFrame::decode(type_id, &buf[2..])
}

/// read_exact 的短读映射为 Truncated，其余原样映射 Io。
fn truncated_or_io(e: io::Error, _buf: &[u8], _want: usize) -> CodecError {
    if e.kind() == io::ErrorKind::UnexpectedEof {
        CodecError::Truncated
    } else {
        CodecError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn data_frame_roundtrip() {
        let frames = vec![
            DataFrame::GetMeta {
                hash: BlobHash([7u8; 32]),
            },
            DataFrame::Meta {
                hash: BlobHash([7u8; 32]),
                size: 42,
                segment_hashes: vec![[1u8; 32], [2u8; 32]],
            },
            DataFrame::DialBack { token: [9u8; 16] },
            DataFrame::Error {
                code: 5,
                message: "出错".into(),
            },
        ];
        for f in frames {
            let (mut tx, mut rx) = duplex(4096);
            write_data_frame(&mut tx, &f).await.unwrap();
            let got = read_data_frame(&mut rx).await.unwrap();
            assert_eq!(got, f);
        }
    }

    #[tokio::test]
    async fn data_frame_roundtrip_and_raw_bytes_follow() {
        let (mut tx, mut rx) = duplex(64);
        write_data_frame(&mut tx, &DataFrame::RangeStart { offset: 0, len: 5 })
            .await
            .unwrap();
        tx.write_all(b"hello").await.unwrap();
        let got = read_data_frame(&mut rx).await.unwrap();
        assert_eq!(got, DataFrame::RangeStart { offset: 0, len: 5 });
        let mut raw = [0u8; 5];
        rx.read_exact(&mut raw).await.unwrap();
        assert_eq!(&raw, b"hello");
    }

    #[tokio::test]
    async fn data_frame_rejects_unknown_type() {
        // 手写：长度 2 | 类型号 500 | 空 map 负载。
        let (mut tx, mut rx) = duplex(64);
        tx.write_all(&2u32.to_be_bytes()).await.unwrap();
        tx.write_all(&500u16.to_be_bytes()).await.unwrap();
        let got = read_data_frame(&mut rx).await;
        assert!(matches!(got, Err(CodecError::Cbor(s)) if s.contains("unknown data frame")));
    }
}
