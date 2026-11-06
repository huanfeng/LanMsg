//! 缩略图生成：缩放到长边 `THUMB_EDGE_PX` 并编码为不超过 `THUMB_MAX_BYTES` 的 JPEG。

use crate::decode::decode_reader;
use crate::{ffmpeg, sniff, MediaError, Rgba, Sniffed};
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use lanmsg_proto::consts::{THUMB_EDGE_PX, THUMB_MAX_BYTES};
use lanmsg_proto::types::MediaInfo;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// 为 `path` 生成 JPEG 缩略图与媒体信息；`_mime` 仅作调用方提示，类型以文件头为准。
pub fn make_thumbnail(path: &Path, _mime: &str) -> Result<(Vec<u8>, MediaInfo), MediaError> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut file = File::open(path)?;
    let mut head = [0u8; 32];
    let n = read_head(&mut file, &mut head)?;
    let kind = sniff(&head[..n], name);
    match kind {
        Sniffed::Jpeg | Sniffed::Png | Sniffed::Gif | Sniffed::Webp => {
            // 直接从文件流解码，不把整个原文件读进内存
            let img = decode_reader(BufReader::new(File::open(path)?), kind)?;
            let info = MediaInfo {
                width: img.width,
                height: img.height,
                duration_ms: None,
            };
            Ok((encode_thumb(img)?, info))
        }
        Sniffed::Heic | Sniffed::Video => {
            #[cfg(windows)]
            if let Ok(img) = crate::shell_win::shell_thumbnail(path, THUMB_EDGE_PX) {
                let info = MediaInfo {
                    width: img.width,
                    height: img.height,
                    duration_ms: None,
                };
                return Ok((encode_thumb(img)?, info));
            }
            let (img, info) = ffmpeg::frame_and_probe(path)?;
            Ok((encode_thumb(img)?, info))
        }
        Sniffed::Other => Err(MediaError::Unsupported),
    }
}

fn read_head(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = file.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

/// 缩放（不放大）并编码，质量 80 起、超限每次降 10，最低 40。
fn encode_thumb(img: Rgba) -> Result<Vec<u8>, MediaError> {
    let src = RgbaImage::from_raw(img.width, img.height, img.pixels)
        .ok_or_else(|| MediaError::Decode("像素缓冲长度与尺寸不符".into()))?;
    let long = src.width().max(src.height());
    let small = if long > THUMB_EDGE_PX {
        let scale = f64::from(THUMB_EDGE_PX) / f64::from(long);
        let w = ((f64::from(src.width()) * scale).round() as u32).max(1);
        let h = ((f64::from(src.height()) * scale).round() as u32).max(1);
        image::imageops::resize(&src, w, h, FilterType::Triangle)
    } else {
        src
    };
    // JPEG 无 alpha：透明区域铺白底
    let mut rgb = Vec::with_capacity(small.as_raw().len() / 4 * 3);
    for p in small.pixels() {
        let a = u32::from(p[3]);
        for c in &p.0[..3] {
            rgb.push(((u32::from(*c) * a + 255 * (255 - a)) / 255) as u8);
        }
    }
    let mut quality = 80u8;
    loop {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
            .write_image(&rgb, small.width(), small.height(), ExtendedColorType::Rgb8)
            .map_err(|e| MediaError::Decode(e.to_string()))?;
        if out.len() <= THUMB_MAX_BYTES {
            return Ok(out);
        }
        if quality <= 40 {
            return Err(MediaError::TooLarge);
        }
        quality -= 10;
    }
}
