//! 媒体解码与缩略图：图片经 `image` 解码，HEIC/视频经 Windows Shell 或外部 ffmpeg。

mod decode;
mod ffmpeg;
#[cfg(windows)]
mod shell_win;
mod sniff;
mod thumb;

pub use decode::{decode_image, Rgba, MAX_DECODE_PIXELS};
pub use ffmpeg::{find_tool, frame_and_probe};
#[cfg(windows)]
pub use shell_win::shell_thumbnail;
pub use sniff::{sniff, Sniffed};
pub use thumb::make_thumbnail;

/// 媒体处理错误。
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("unsupported media format")]
    Unsupported,
    #[error("image dimensions exceed decode limit")]
    TooLarge,
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffmpeg/ffprobe missing")]
    ToolMissing,
    #[error("external tool timed out")]
    Timeout,
}
