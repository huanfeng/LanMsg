//! 按魔数判断媒体类型，魔数无法判断时才回退到扩展名。

/// 嗅探结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Jpeg,
    Png,
    Gif,
    Webp,
    Heic,
    Video,
    Other,
}

/// 根据文件头（建议至少 32 字节）与文件名判断类型。
pub fn sniff(head: &[u8], name: &str) -> Sniffed {
    if let Some(s) = sniff_magic(head) {
        return s;
    }
    // 魔数无法识别：已知是 zip/ELF 等其它格式时不应被扩展名欺骗
    if head.starts_with(b"PK\x03\x04") {
        return Sniffed::Other;
    }
    sniff_extension(name)
}

fn sniff_magic(head: &[u8]) -> Option<Sniffed> {
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Sniffed::Jpeg);
    }
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Sniffed::Png);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(Sniffed::Gif);
    }
    if head.len() >= 12 && &head[0..4] == b"RIFF" {
        if &head[8..12] == b"WEBP" {
            return Some(Sniffed::Webp);
        }
        if &head[8..12] == b"AVI " {
            return Some(Sniffed::Video);
        }
    }
    if head.starts_with(b"\x1aE\xdf\xa3") {
        return Some(Sniffed::Video);
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return Some(match &head[8..12] {
            b"heic" | b"heix" | b"mif1" | b"msf1" => Sniffed::Heic,
            // isom/mp42/qt 等一律按视频；未知 brand 也按视频交给 ffmpeg 判断
            _ => Sniffed::Video,
        });
    }
    None
}

fn sniff_extension(name: &str) -> Sniffed {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Sniffed::Jpeg,
        "png" => Sniffed::Png,
        "gif" => Sniffed::Gif,
        "webp" => Sniffed::Webp,
        "heic" | "heif" => Sniffed::Heic,
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" => Sniffed::Video,
        _ => Sniffed::Other,
    }
}
