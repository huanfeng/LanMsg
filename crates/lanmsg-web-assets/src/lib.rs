//! 内嵌网页传输的静态页面（仓库根 `web/`，不含 `web/test/`）。

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// 按请求路径取内嵌资源，返回 (内容, MIME)。
/// `""` 与 `"/"` 映射到 `index.html`；含 `..` 或反斜杠的路径返回 `None`。
pub fn get(path: &str) -> Option<(&'static [u8], &'static str)> {
    if path.contains("..") || path.contains('\\') {
        return None;
    }
    let p = path.trim_start_matches('/');
    let p = if p.is_empty() { "index.html" } else { p };
    ASSETS
        .iter()
        .find(|(name, _, _)| *name == p)
        .map(|(_, body, mime)| (*body, *mime))
}

/// 全部内嵌资源的总字节数。
pub fn total_size() -> usize {
    ASSETS.iter().map(|(_, body, _)| body.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_maps_to_index() {
        assert_eq!(get("/").unwrap().1, "text/html; charset=utf-8");
        assert_eq!(get("").unwrap().1, "text/html; charset=utf-8");
    }

    #[test]
    fn favicon_is_served_and_linked_from_index() {
        // 浏览器会自动请求图标；页面显式引用同源 SVG，避免 CSP 下的 404/违规噪声。
        let (icon, mime) = get("/favicon.svg").expect("favicon.svg embedded");
        assert_eq!(mime, "image/svg+xml");
        assert!(icon.starts_with(b"<svg"));
        let (index, _) = get("/").expect("index embedded");
        let html = std::str::from_utf8(index).expect("index is utf-8");
        assert!(html.contains(r#"href="favicon.svg""#));
    }

    #[test]
    fn traversal_rejected() {
        assert!(get("/../Cargo.toml").is_none());
    }

    #[test]
    fn test_dir_not_embedded() {
        assert!(get("/test/vectors.json").is_none());
    }

    #[test]
    fn page_under_300kb() {
        assert!(
            total_size() < 300 * 1024,
            "web/ exceeds 300KB: {}",
            total_size()
        );
    }
}
