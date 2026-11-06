// 把 lanmsg-media 的图片解码接入 windui 的 register_decoder 扩展点。
// PNG 已由 windui 内置，不重复注册。
// 注释中文，标识符英文。

use lanmsg_media::{decode_image, sniff, Sniffed};
use windui::render::image::{register_decoder, DecodedImage, ImageDecoder, ImageError};

/// Jpeg / Gif / Webp 解码器（含 EXIF 方向与尺寸上限，由 lanmsg-media 保证）。
struct MediaDecoder;

impl ImageDecoder for MediaDecoder {
    fn probe(&self, bytes: &[u8]) -> bool {
        let head = &bytes[..bytes.len().min(32)];
        matches!(
            sniff(head, ""),
            Sniffed::Jpeg | Sniffed::Gif | Sniffed::Webp
        )
    }

    fn decode(&self, bytes: &[u8]) -> Result<DecodedImage, ImageError> {
        let img = decode_image(bytes).map_err(|e| ImageError::Decode(e.to_string()))?;
        Ok(DecodedImage {
            width: img.width,
            height: img.height,
            rgba: img.pixels,
        })
    }

    fn name(&self) -> &'static str {
        "lanmsg-media"
    }
}

/// 注册媒体解码器；须在 UI 线程、`main` 开头调用一次（注册表为线程局部）。
pub fn register_media_decoders() {
    register_decoder(Box::new(MediaDecoder));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_accepts_jpeg_gif_webp_but_not_png() {
        let d = MediaDecoder;
        assert!(d.probe(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]));
        assert!(d.probe(b"GIF89a\x01\x00"));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        webp.resize(32, 0);
        assert!(d.probe(&webp));
        assert!(!d.probe(b"\x89PNG\r\n\x1a\n"));
        assert!(!d.probe(b"plain text"));
    }

    #[test]
    fn decode_garbage_returns_error_not_panic() {
        let d = MediaDecoder;
        assert!(d.decode(&[0xFF, 0xD8, 0xFF, 0x00]).is_err());
    }
}
