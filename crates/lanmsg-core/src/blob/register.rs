// P5：本地文件登记。哈希文件、生成缩略图，并在单事务内写入 Blob 元数据、
// 本地行与本机持有者；中转站、LocalSend、Web 共用。
// 阻塞的文件读取一律走 spawn_blocking + std::fs。
// 注释中文，标识符英文。

use std::path::PathBuf;
use std::sync::Arc;

use lanmsg_proto::consts::THUMB_MAX_BYTES;
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::types::BlobMeta;

use crate::blob::hash::hash_file;
use crate::blob::mime::mime_from_name;
use crate::error::CoreError;
use crate::platform::ThumbnailProvider;
use crate::store::{BlobLocalKind, BlobLocalRow, Store};

/// 登记一个本地文件为本机持有的 Blob。路径不存在或不是普通文件 → InvalidArgument。
pub async fn register_local_file(
    store: Arc<Store>,
    self_device: DeviceId,
    thumbnailer: Option<Arc<dyn ThumbnailProvider>>,
    path: PathBuf,
    kind: BlobLocalKind,
    now: i64,
) -> Result<BlobMeta, CoreError> {
    let p = path.clone();
    let hashed = tokio::task::spawn_blocking(move || {
        let meta = std::fs::metadata(&p).map_err(|_| {
            CoreError::InvalidArgument(format!("path does not exist: {}", p.display()))
        })?;
        if !meta.is_file() {
            return Err(CoreError::InvalidArgument(format!(
                "not a regular file: {}",
                p.display()
            )));
        }
        hash_file(&p).map_err(CoreError::Io)
    })
    .await
    .map_err(|e| CoreError::Proto(format!("hash task failed: {e}")))??;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mime = mime_from_name(&name).to_string();

    let mut media = None;
    let mut thumb = None;
    if let Some(provider) = thumbnailer {
        if mime.starts_with("image/") || mime.starts_with("video/") {
            let p = path.clone();
            let m = mime.clone();
            if let Ok(Some(t)) =
                tokio::task::spawn_blocking(move || provider.thumbnail(&p, &m)).await
            {
                if t.jpeg.len() <= THUMB_MAX_BYTES {
                    thumb = Some(t.jpeg);
                    media = Some(t.media);
                }
            }
        }
    }

    let meta = BlobMeta {
        hash: hashed.hash,
        size: hashed.size,
        name,
        mime,
        media,
        thumb,
    };
    let local = BlobLocalRow {
        hash: hashed.hash,
        path,
        kind,
        size: hashed.size,
        mtime: hashed.mtime,
    };
    let s = store.clone();
    let m = meta.clone();
    tokio::task::spawn_blocking(move || {
        s.register_local_blob(&m, &hashed.segment_hashes, &local, &self_device, now)
    })
    .await
    .map_err(|e| CoreError::Proto(format!("register task failed: {e}")))??;
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_local_file_writes_blob_local_and_holder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"abc").unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let me = DeviceId([7u8; 32]);
        let meta = register_local_file(
            store.clone(),
            me,
            None,
            path.clone(),
            BlobLocalKind::Source,
            10,
        )
        .await
        .unwrap();
        assert_eq!(meta.mime, "text/plain");
        assert_eq!(meta.size, 3);
        assert_eq!(meta.name, "a.txt");
        let local = store.get_local(&meta.hash).unwrap().unwrap();
        assert_eq!(local.path, path);
        assert_eq!(local.kind, BlobLocalKind::Source);
        assert_eq!(store.holders(&meta.hash).unwrap(), vec![me]);
        assert_eq!(store.blob_segments(&meta.hash).unwrap().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn register_local_file_rejects_missing_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        for p in [dir.path().join("nope"), dir.path().to_path_buf()] {
            let err = register_local_file(
                store.clone(),
                DeviceId([1u8; 32]),
                None,
                p,
                BlobLocalKind::Source,
                1,
            )
            .await
            .unwrap_err();
            assert!(matches!(err, CoreError::InvalidArgument(_)));
        }
    }
}
