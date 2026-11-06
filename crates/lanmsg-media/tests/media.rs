//! 解码与缩略图集成测试；测试图片现场生成，不提交二进制夹具。

use image::{ImageEncoder, RgbImage};
use lanmsg_media::{decode_image, make_thumbnail, sniff, MediaError, Sniffed};
use lanmsg_proto::consts::{THUMB_EDGE_PX, THUMB_MAX_BYTES};
use lanmsg_proto::types::MediaInfo;

fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, 7])
    });
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

fn jpeg_bytes(w: u32, h: u32) -> Vec<u8> {
    // 用噪声般的纹理，避免过于平滑导致缩略图体积不具代表性
    let img = RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([
            (x.wrapping_mul(31) ^ y) as u8,
            (y.wrapping_mul(17) ^ x) as u8,
            ((x + y) % 256) as u8,
        ])
    });
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

#[test]
fn sniff_by_magic_not_extension() {
    assert_eq!(sniff(&png_bytes(2, 2), "a.jpg"), Sniffed::Png);
    assert_eq!(sniff(b"PK\x03\x04rest-of-zip", "a.jpg"), Sniffed::Other);
}

#[test]
fn sniff_video_and_heic_brands() {
    let mut mp4 = vec![0, 0, 0, 0x18];
    mp4.extend_from_slice(b"ftypisom\0\0\0\0");
    assert_eq!(sniff(&mp4, "x.bin"), Sniffed::Video);
    let mut heic = vec![0, 0, 0, 0x18];
    heic.extend_from_slice(b"ftypheic\0\0\0\0");
    assert_eq!(sniff(&heic, "x.bin"), Sniffed::Heic);
    assert_eq!(sniff(b"\x1aE\xdf\xa3....", "x.bin"), Sniffed::Video);
    assert_eq!(sniff(b"hello", "clip.mp4"), Sniffed::Video);
}

#[test]
fn thumbnail_of_large_jpeg_fits_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.jpg");
    std::fs::write(&path, jpeg_bytes(3000, 2000)).unwrap();
    let (jpeg, info) = make_thumbnail(&path, "image/jpeg").unwrap();
    assert!(jpeg.len() <= THUMB_MAX_BYTES, "缩略图 {} 字节", jpeg.len());
    let decoded = decode_image(&jpeg).unwrap();
    assert_eq!(decoded.width.max(decoded.height), THUMB_EDGE_PX);
    assert_eq!(
        info,
        MediaInfo {
            width: 3000,
            height: 2000,
            duration_ms: None
        }
    );
}

#[test]
fn rejects_oversized_dimensions() {
    // 只构造合法 IHDR 声明 20000x20000 的 PNG（无像素数据），必须在解码分配前被拒绝
    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&20000u32.to_be_bytes());
    ihdr.extend_from_slice(&20000u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    png.extend_from_slice(&13u32.to_be_bytes());
    let mut chunk = b"IHDR".to_vec();
    chunk.extend_from_slice(&ihdr);
    png.extend_from_slice(&chunk);
    png.extend_from_slice(&crc32(&chunk).to_be_bytes());
    assert!(matches!(decode_image(&png), Err(MediaError::TooLarge)));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.png");
    std::fs::write(&path, &png).unwrap();
    assert!(matches!(
        make_thumbnail(&path, "image/png"),
        Err(MediaError::TooLarge)
    ));
}

#[test]
fn garbage_bytes_return_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.png");
    let junk: Vec<u8> = (0..4096u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    std::fs::write(&path, junk).unwrap();
    assert!(make_thumbnail(&path, "image/png").is_err());
    // 伪造扩展名的 zip
    let zip = dir.path().join("fake.jpg");
    std::fs::write(&zip, b"PK\x03\x04\x14\0\0\0\0\0 not really a zip").unwrap();
    assert!(make_thumbnail(&zip, "image/jpeg").is_err());
}

#[test]
fn truncated_png_returns_err() {
    let mut png = png_bytes(64, 64);
    png.truncate(png.len() / 2);
    assert!(decode_image(&png).is_err());
}

/// PNG 用的 CRC32（测试内自带，避免多引依赖）。
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
