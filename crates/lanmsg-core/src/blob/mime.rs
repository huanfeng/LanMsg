// P2 Task 11：扩展名 → MIME 映射。
// 只按文件扩展名判定，不读取内容；未识别的扩展名返回 application/octet-stream。
// 注释中文，标识符英文。

/// 按文件名扩展名（不区分大小写）返回 MIME 类型；未识别返回 octet-stream。
pub fn mime_from_name(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" | "heif" => "image/heic",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "zip" => "application/zip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "apk" => "application/vnd.android.package-archive",
        "exe" => "application/vnd.microsoft.portable-executable",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_lookup_case_insensitive() {
        assert_eq!(mime_from_name("A.JPG"), "image/jpeg");
        assert_eq!(mime_from_name("x.unknown"), "application/octet-stream");
        assert_eq!(mime_from_name("视频.MP4"), "video/mp4");
        assert_eq!(mime_from_name("无扩展名"), "application/octet-stream");
    }
}
