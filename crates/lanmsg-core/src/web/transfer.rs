//! Web 文件传输：分块上传状态、分块明文格式与磁盘读写。
//!
//! 分块明文：`seq(u64 BE) || fid_len(u8) || fid(ASCII) || chunk_idx(u32 BE) || data`，
//! 与 web/crypto.js 的 `frameChunk` / `parseChunk` 逐字节一致。
//! 磁盘读写一律是阻塞函数，调用方放进 `spawn_blocking`（禁止 tokio::fs::File 并发复用）。

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// 分块大小（字节）。
pub const CHUNK_SIZE: u32 = 1_048_576;
/// 单个文件的大小上限（保证分块数不超过 u32）。
pub const MAX_FILE_SIZE: u64 = 1 << 40;

/// 文件按 `CHUNK_SIZE` 切分后的分块数。
pub fn chunk_count(size: u64) -> u32 {
    size.div_ceil(CHUNK_SIZE as u64) as u32
}

/// 第 `idx` 块的字节数（越界返回 0）。
pub fn chunk_len(size: u64, idx: u32) -> usize {
    let start = idx as u64 * CHUNK_SIZE as u64;
    if start >= size {
        return 0;
    }
    (size - start).min(CHUNK_SIZE as u64) as usize
}

/// 一次进行中（或刚完成）的上传。
#[derive(Debug)]
pub struct UploadState {
    pub fid: String,
    pub client_key: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub chunk_size: u32,
    /// `.part` 临时文件路径。
    pub path_part: PathBuf,
    /// 目标目录（finish 时在此目录下生成最终文件名）。
    pub dir: PathBuf,
    pub have: Vec<bool>,
    /// finish 成功后记录消息 ID，重复 finish 直接返回同一结果。
    pub finished: Option<String>,
}

impl UploadState {
    pub fn new(
        fid: String,
        client_key: String,
        name: String,
        mime: String,
        size: u64,
        dir: PathBuf,
    ) -> Self {
        let path_part = dir.join(format!(".{fid}.part"));
        UploadState {
            have: vec![false; chunk_count(size) as usize],
            fid,
            client_key,
            name,
            mime,
            size,
            chunk_size: CHUNK_SIZE,
            path_part,
            dir,
            finished: None,
        }
    }

    pub fn have_list(&self) -> Vec<u32> {
        indices(&self.have, true)
    }

    pub fn missing_list(&self) -> Vec<u32> {
        indices(&self.have, false)
    }
}

fn indices(v: &[bool], want: bool) -> Vec<u32> {
    v.iter()
        .enumerate()
        .filter(|(_, b)| **b == want)
        .map(|(i, _)| i as u32)
        .collect()
}

/// 组装分块明文。
pub fn frame_chunk(seq: u64, fid: &str, idx: u32, data: &[u8]) -> Vec<u8> {
    let f = fid.as_bytes();
    let mut out = Vec::with_capacity(8 + 1 + f.len() + 4 + data.len());
    out.extend_from_slice(&seq.to_be_bytes());
    // fid 由本模块生成（32 个十六进制字符），长度恒小于 256。
    out.push(f.len().min(255) as u8);
    out.extend_from_slice(f);
    out.extend_from_slice(&idx.to_be_bytes());
    out.extend_from_slice(data);
    out
}

/// 解析后的分块明文。
pub struct ParsedChunk<'a> {
    pub seq: u64,
    pub fid: &'a str,
    pub idx: u32,
    pub data: &'a [u8],
}

/// 解析分块明文；格式错误返回 None。
pub fn parse_chunk(plain: &[u8]) -> Option<ParsedChunk<'_>> {
    if plain.len() < 14 {
        return None;
    }
    let seq = u64::from_be_bytes(plain[0..8].try_into().ok()?);
    let flen = plain[8] as usize;
    if plain.len() < 13 + flen {
        return None;
    }
    let fid = std::str::from_utf8(&plain[9..9 + flen]).ok()?;
    let idx = u32::from_be_bytes(plain[9 + flen..13 + flen].try_into().ok()?);
    Some(ParsedChunk {
        seq,
        fid,
        idx,
        data: &plain[13 + flen..],
    })
}

/// 在 `offset` 处写入一块（阻塞）。
pub fn write_chunk(path: &Path, offset: u64, data: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    f.write_all(data)
}

/// 读取第 `idx` 块（阻塞）。文件被改短时返回错误。
pub fn read_chunk(path: &Path, size: u64, idx: u32) -> std::io::Result<Vec<u8>> {
    let len = chunk_len(size, idx);
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(idx as u64 * CHUNK_SIZE as u64))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_math() {
        assert_eq!(chunk_count(0), 0);
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(CHUNK_SIZE as u64), 1);
        assert_eq!(chunk_count(CHUNK_SIZE as u64 + 1), 2);
        assert_eq!(chunk_len(CHUNK_SIZE as u64 + 5, 1), 5);
        assert_eq!(chunk_len(CHUNK_SIZE as u64 + 5, 0), CHUNK_SIZE as usize);
        assert_eq!(chunk_len(10, 1), 0);
    }

    #[test]
    fn frame_roundtrip() {
        let f = frame_chunk(7, "abcd", 3, b"xyz");
        let p = parse_chunk(&f).unwrap();
        assert_eq!((p.seq, p.fid, p.idx, p.data), (7, "abcd", 3, &b"xyz"[..]));
        assert!(parse_chunk(&f[..10]).is_none());
    }

    #[test]
    fn frame_matches_js_layout() {
        // seq=1 的高位在前；fid_len 为单字节。
        let f = frame_chunk(1, "a", 2, &[9]);
        assert_eq!(f, vec![0, 0, 0, 0, 0, 0, 0, 1, 1, b'a', 0, 0, 0, 2, 9]);
    }

    #[test]
    fn write_then_read_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.part");
        std::fs::File::create(&p).unwrap();
        let size = CHUNK_SIZE as u64 + 3;
        write_chunk(&p, CHUNK_SIZE as u64, b"end").unwrap();
        write_chunk(&p, 0, &vec![7u8; CHUNK_SIZE as usize]).unwrap();
        assert_eq!(read_chunk(&p, size, 1).unwrap(), b"end");
        assert_eq!(read_chunk(&p, size, 0).unwrap().len(), CHUNK_SIZE as usize);
        assert!(read_chunk(&p, size + 10, 1).is_err());
    }
}
