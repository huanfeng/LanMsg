//! 图片解码；尺寸检查在分配像素缓冲之前完成。

use crate::{sniff, MediaError, Sniffed};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use std::io::{BufRead, Cursor, Read, Seek, SeekFrom};

/// 宽×高超过该像素数即拒绝解码。
pub const MAX_DECODE_PIXELS: u64 = 64_000_000;

/// 非预乘 RGBA8 像素。
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// 解码 Jpeg/Png/Gif（首帧）/Webp，其它格式返回 `Unsupported`。
pub fn decode_image(bytes: &[u8]) -> Result<Rgba, MediaError> {
    let head = &bytes[..bytes.len().min(32)];
    decode_reader(Cursor::new(bytes), sniff(head, ""))
}

/// 从可回退的读取器解码；`kind` 决定解码格式。
pub(crate) fn decode_reader<R: BufRead + Seek>(
    mut reader: R,
    kind: Sniffed,
) -> Result<Rgba, MediaError> {
    let format = match kind {
        Sniffed::Jpeg => ImageFormat::Jpeg,
        Sniffed::Png => ImageFormat::Png,
        Sniffed::Gif => ImageFormat::Gif,
        Sniffed::Webp => ImageFormat::WebP,
        _ => return Err(MediaError::Unsupported),
    };
    // PNG/GIF 的尺寸在固定偏移处，先直接读出检查，不依赖解码器能否读完整个头
    if let Some((w, h)) = fixed_header_dimensions(&mut reader, kind) {
        check_dimensions(w, h)?;
    }
    // 创建解码器只读文件头，此时不设分配上限，以便由下面的尺寸检查给出 TooLarge
    let mut decoder = ImageReader::with_format(reader, format)
        .into_decoder()
        .map_err(map_image_error)?;
    let (w, h) = decoder.dimensions();
    check_dimensions(w, h)?;
    // 尺寸合规后才真正解码；分配上限作为第二道防线（约 256MB）
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_PIXELS * 4);
    decoder.set_limits(limits).map_err(map_image_error)?;
    let img = image::DynamicImage::from_decoder(decoder).map_err(map_image_error)?;
    let rgba = img.into_rgba8();
    Ok(Rgba {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
    })
}

fn map_image_error(e: image::ImageError) -> MediaError {
    match e {
        image::ImageError::Limits(_) => MediaError::TooLarge,
        other => MediaError::Decode(other.to_string()),
    }
}

fn check_dimensions(w: u32, h: u32) -> Result<(), MediaError> {
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_DECODE_PIXELS {
        return Err(MediaError::TooLarge);
    }
    Ok(())
}

/// 读取 PNG（IHDR）/ GIF（逻辑屏幕描述符）头中的尺寸，读完后回到起点；读不到返回 `None`。
fn fixed_header_dimensions<R: Read + Seek>(reader: &mut R, kind: Sniffed) -> Option<(u32, u32)> {
    let mut buf = [0u8; 24];
    let dims = match kind {
        Sniffed::Png => {
            reader.read_exact(&mut buf).ok()?;
            (
                u32::from_be_bytes(buf[16..20].try_into().ok()?),
                u32::from_be_bytes(buf[20..24].try_into().ok()?),
            )
        }
        Sniffed::Gif => {
            reader.read_exact(&mut buf[..10]).ok()?;
            (
                u32::from(u16::from_le_bytes([buf[6], buf[7]])),
                u32::from(u16::from_le_bytes([buf[8], buf[9]])),
            )
        }
        _ => return None,
    };
    reader.seek(SeekFrom::Start(0)).ok()?;
    Some(dims)
}
