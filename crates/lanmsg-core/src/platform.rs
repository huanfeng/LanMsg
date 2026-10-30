// P2 核心扩展：平台钩子（Task 4）。
// 网络状态与缩略图生成由平台层注入；core 只定义接口与默认值。
// 注释中文，标识符英文。

use std::path::Path;

use std::sync::Arc;

use lanmsg_proto::types::MediaInfo;

/// 网络状态（Task 4）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkState {
    pub is_metered: bool,
    pub is_wifi_or_wired: bool,
}

/// 缩略图输出。
pub struct Thumbnail {
    pub jpeg: Vec<u8>,
    pub media: MediaInfo,
}

/// 缩略图生成器（平台层实现）。
pub trait ThumbnailProvider: Send + Sync {
    /// 阻塞调用，core 会在 spawn_blocking 中调用；失败返回 None。
    fn thumbnail(&self, path: &Path, mime: &str) -> Option<Thumbnail>;
}

/// 平台钩子集合。
#[derive(Default, Clone)]
pub struct PlatformHooks {
    pub network_state: Option<Arc<dyn Fn() -> NetworkState + Send + Sync>>,
    pub thumbnailer: Option<Arc<dyn ThumbnailProvider>>,
}

impl PlatformHooks {
    /// 查询网络状态；未注入时按不计费、wifi/有线 的默认。
    pub fn network(&self) -> NetworkState {
        match &self.network_state {
            Some(f) => f(),
            None => NetworkState {
                is_metered: false,
                is_wifi_or_wired: true,
            },
        }
    }
}
