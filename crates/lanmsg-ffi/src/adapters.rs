// 外部（Kotlin）实现到 core 接口的适配：密钥库、网络状态、缩略图。
// 注释中文，标识符英文。

use std::path::Path;
use std::sync::Arc;

use lanmsg_core::error::KeyStoreError;
use lanmsg_core::keystore::KeyStore;
use lanmsg_core::platform::{NetworkState, PlatformHooks, Thumbnail, ThumbnailProvider};
use lanmsg_proto::consts::THUMB_EDGE_PX;
use lanmsg_proto::types::MediaInfo;

use crate::{FfiError, FfiKeyStore, FfiPlatform};

/// 把外部密钥库包装成 core 的 `KeyStore`。
pub struct ForeignKeyStore(pub Arc<dyn FfiKeyStore>);

fn to_keystore_error(e: FfiError) -> KeyStoreError {
    KeyStoreError::Corrupt(e.to_string())
}

impl KeyStore for ForeignKeyStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        self.0.load(name.to_string()).map_err(to_keystore_error)
    }
    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.0
            .save(name.to_string(), bytes.to_vec())
            .map_err(to_keystore_error)
    }
    fn delete(&self, name: &str) -> Result<(), KeyStoreError> {
        self.0.delete(name.to_string()).map_err(to_keystore_error)
    }
}

/// 把外部缩略图能力包装成 core 的 `ThumbnailProvider`。
pub struct ForeignThumbnailer(pub Arc<dyn FfiPlatform>);

impl ThumbnailProvider for ForeignThumbnailer {
    fn thumbnail(&self, path: &Path, mime: &str) -> Option<Thumbnail> {
        let r = self.0.thumbnail(
            path.to_string_lossy().into_owned(),
            mime.to_string(),
            THUMB_EDGE_PX,
        )?;
        Some(Thumbnail {
            jpeg: r.jpeg,
            media: MediaInfo {
                width: r.width,
                height: r.height,
                duration_ms: r.duration_ms,
            },
        })
    }
}

/// 由外部平台实现构造 core 的平台钩子。
pub fn platform_hooks(p: Arc<dyn FfiPlatform>) -> PlatformHooks {
    let net = p.clone();
    PlatformHooks {
        network_state: Some(Arc::new(move || NetworkState {
            is_metered: net.is_metered(),
            is_wifi_or_wired: net.is_wifi(),
        })),
        thumbnailer: Some(Arc::new(ForeignThumbnailer(p))),
    }
}
