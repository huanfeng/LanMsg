// P2 Task 11：文件哈希（整文件 BLAKE3 + 分段哈希）。
//
// 设计取舍（计划原文）：
// - 整文件 BLAKE3 是 Blob 的身份；
// - 分段哈希表从持有者处获取，用于断点续传与定位坏分段；
// - 下载完成后必须整文件复核，不一致整体作废；
// - 持有者都是已认证设备，不做逐段 Merkle 证明。
//
// hash_file 为阻塞函数：单次顺序读（1 MiB 缓冲），同一遍内同时更新整文件
// 哈希与当前分段哈希；调用方必须在 `spawn_blocking` 中执行。
// 注释中文，标识符英文。

use std::io::Read;
use std::path::Path;

use lanmsg_proto::consts::BLOB_SEGMENT_LEN;
use lanmsg_proto::ids::BlobHash;

/// 哈希结果：整文件哈希、大小、修改时间（Unix 毫秒）与分段哈希表。
pub struct HashedFile {
    pub hash: BlobHash,
    pub size: u64,
    pub mtime: i64,
    pub segment_hashes: Vec<[u8; 32]>,
}

/// 1 MiB 读取缓冲。
const READ_BUF: usize = 1024 * 1024;

/// 阻塞地单次顺序读文件，同时计算整文件 BLAKE3 与分段 BLAKE3（分段长
/// `BLOB_SEGMENT_LEN`）。文件大小 0 时 `segment_hashes` 为空。
pub fn hash_file(path: &Path) -> std::io::Result<HashedFile> {
    let mut file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    let mtime = file_mtime(&meta);

    let mut whole = blake3::Hasher::new();
    let mut seg = blake3::Hasher::new();
    let mut segment_hashes = Vec::new();
    let mut size: u64 = 0;
    let mut buf = vec![0u8; READ_BUF];

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        whole.update(&buf[..n]);
        // 把本批读到的字节按分段边界切开喂给当前分段哈希。
        let mut chunk = &buf[..n];
        while !chunk.is_empty() {
            let into_seg = BLOB_SEGMENT_LEN - size % BLOB_SEGMENT_LEN;
            let take = (into_seg.min(chunk.len() as u64)) as usize;
            seg.update(&chunk[..take]);
            size += take as u64;
            if size.is_multiple_of(BLOB_SEGMENT_LEN) {
                segment_hashes.push(*seg.finalize().as_bytes());
                seg = blake3::Hasher::new();
            }
            chunk = &chunk[take..];
        }
    }
    // 末尾不足整段的部分单独收尾；整段对齐或空文件不再追加。
    if !size.is_multiple_of(BLOB_SEGMENT_LEN) {
        segment_hashes.push(*seg.finalize().as_bytes());
    }

    Ok(HashedFile {
        hash: BlobHash(*whole.finalize().as_bytes()),
        size,
        mtime,
        segment_hashes,
    })
}

/// 文件修改时间（Unix 毫秒）。
pub fn file_mtime(meta: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime() * 1000 + meta.mtime_nsec() / 1_000_000
}

/// 分段数：size == 0 → 0；否则 ceil(size / BLOB_SEGMENT_LEN)。
pub fn segment_count(size: u64) -> usize {
    if size == 0 {
        0
    } else {
        size.div_ceil(BLOB_SEGMENT_LEN) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写 BLOB_SEGMENT_LEN + 10 字节随机文件 → 整文件哈希 == blake3(全部)；
    /// segment_hashes.len() == 2；第 2 段 == blake3(最后 10 字节)。
    #[test]
    fn hash_file_matches_blake3_and_segments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("大文件.bin");
        let mut data = vec![0u8; BLOB_SEGMENT_LEN as usize + 10];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut data);
        std::fs::write(&path, &data).unwrap();

        let h = hash_file(&path).unwrap();
        assert_eq!(h.hash.0, *blake3::hash(&data).as_bytes());
        assert_eq!(h.size, data.len() as u64);
        assert_eq!(h.segment_hashes.len(), 2);
        assert_eq!(
            h.segment_hashes[1],
            *blake3::hash(&data[BLOB_SEGMENT_LEN as usize..]).as_bytes()
        );
        assert_eq!(
            h.segment_hashes[0],
            *blake3::hash(&data[..BLOB_SEGMENT_LEN as usize]).as_bytes()
        );
        assert_eq!(segment_count(h.size), 2);
    }

    #[test]
    fn empty_file_has_no_segments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("空.bin");
        std::fs::write(&path, b"").unwrap();
        let h = hash_file(&path).unwrap();
        assert_eq!(h.size, 0);
        assert!(h.segment_hashes.is_empty());
        assert_eq!(h.hash.0, *blake3::hash(b"").as_bytes());
        assert_eq!(segment_count(0), 0);
    }
}
