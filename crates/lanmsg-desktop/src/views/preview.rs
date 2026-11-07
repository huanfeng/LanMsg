// 应用内预览：已下载到本地的常见图片与文本文件，在对话框里直接查看。
// 判定与文本读取是纯函数（见测试）；读取与解码放后台线程，结果经收件槽由 `tick` 取回。
// 注释中文，标识符英文。

use std::cell::RefCell;
use std::io::Read;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use windui::prelude::*;

use crate::strings;
use crate::style;
use crate::ui_state::UiState;
use crate::views::message_row::parent_dir;

/// 文本预览最多读取的字节数（1 MiB）。
pub const TEXT_MAX_BYTES: usize = 1 << 20;
/// 判定二进制时检查的文件头长度。
const BINARY_SNIFF_BYTES: usize = 8 * 1024;
/// 图片预览允许读入内存的文件上限。
const IMAGE_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// 预览对话框宽度与图片可用区域。
const DIALOG_W: i32 = 720;
const IMAGE_BOX_W: u32 = 672;
const IMAGE_BOX_H: u32 = 440;
const TEXT_BOX_H: i32 = 420;
/// 文本预览用等宽字体；非 Windows 平台交给系统的 monospace 别名。
const MONO_FONT: &str = if cfg!(windows) {
    "Consolas"
} else {
    "monospace"
};

/// 可预览的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewKind {
    Image,
    Text,
}

/// 文本扩展名白名单。
const TEXT_EXTS: &[&str] = &[
    "txt", "md", "log", "csv", "json", "xml", "yaml", "yml", "toml", "ini", "conf", "js", "ts",
    "py", "rs", "kt", "java", "c", "h", "cpp", "go", "sh", "bat", "ps1", "html", "css", "sql",
];
/// 图片扩展名（以解码器能力为准：lanmsg-media 解 png/jpeg/gif/webp，桌面端不含 bmp）。
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
const IMAGE_MIMES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// 按 MIME 优先、扩展名兜底判定能否应用内预览；不能预览返回 `None`。
pub fn preview_kind(mime: &str, name: &str) -> Option<PreviewKind> {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if IMAGE_MIMES.contains(&mime.as_str()) {
        return Some(PreviewKind::Image);
    }
    if mime.starts_with("text/") {
        return Some(PreviewKind::Text);
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if IMAGE_EXTS.contains(&ext.as_str()) {
        return Some(PreviewKind::Image);
    }
    if TEXT_EXTS.contains(&ext.as_str()) {
        return Some(PreviewKind::Text);
    }
    None
}

/// 文本预览内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPreview {
    pub text: String,
    /// 文件超过 1 MiB，只显示了前面一段。
    pub truncated: bool,
}

/// 把读到的字节解成预览文本；文件头 8 KiB 内有 NUL 视为二进制，返回 `None`。
/// `truncated` 表示 `bytes` 只是文件的前一段；此时丢弃末尾被截断的不完整 UTF-8 序列。
pub fn decode_text(bytes: &[u8], truncated: bool) -> Option<TextPreview> {
    if bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
        return None;
    }
    let mut data = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if truncated {
        if let Err(e) = std::str::from_utf8(data) {
            if e.error_len().is_none() {
                data = &data[..e.valid_up_to()];
            }
        }
    }
    Some(TextPreview {
        text: String::from_utf8_lossy(data).into_owned(),
        truncated,
    })
}

/// 读取文件前 1 MiB 并解码；二进制返回 `Ok(None)`。
pub fn read_text_preview(path: &Path) -> std::io::Result<Option<TextPreview>> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(TEXT_MAX_BYTES as u64 + 1)
        .read_to_end(&mut buf)?;
    let truncated = buf.len() > TEXT_MAX_BYTES;
    buf.truncate(TEXT_MAX_BYTES);
    Ok(decode_text(&buf, truncated))
}

/// 只读文件头判断是否二进制（点击时在 UI 线程做，最多读 8 KiB）。
fn looks_binary(path: &Path) -> bool {
    let mut head = Vec::new();
    match std::fs::File::open(path)
        .and_then(|f| f.take(BINARY_SNIFF_BYTES as u64).read_to_end(&mut head))
    {
        Ok(_) => head.contains(&0),
        Err(_) => false,
    }
}

/// 把图片等比缩进 `max_w x max_h`，且不放大超过原尺寸。
pub fn fit_within(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 || (w <= max_w && h <= max_h) {
        return (w, h);
    }
    let scale = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

/// 一次预览请求；`epoch` 保证对同一文件再次预览也会重建对话框。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreviewReq {
    pub epoch: u64,
    pub path: String,
    pub name: String,
    pub kind: PreviewKind,
}

/// 后台加载的结果。
#[derive(Clone)]
pub enum PreviewBody {
    Loading,
    Text(TextPreview),
    Image {
        w: u32,
        h: u32,
        rgba: Rc<Vec<u8>>,
    },
    /// 内容判为二进制。
    Binary,
    Failed,
}

/// 后台线程产出的结果（`Rc` 不能跨线程，图片像素这里用 `Vec`）。
enum Loaded {
    Text(TextPreview),
    Image { w: u32, h: u32, rgba: Vec<u8> },
    Binary,
    Failed,
}

fn load(kind: PreviewKind, path: &str) -> Loaded {
    match kind {
        PreviewKind::Text => match read_text_preview(Path::new(path)) {
            Ok(Some(t)) => Loaded::Text(t),
            Ok(None) => Loaded::Binary,
            Err(_) => Loaded::Failed,
        },
        PreviewKind::Image => {
            let ok_size = std::fs::metadata(path)
                .map(|m| m.len() <= IMAGE_MAX_BYTES)
                .unwrap_or(false);
            if !ok_size {
                return Loaded::Failed;
            }
            match std::fs::read(path)
                .ok()
                .and_then(|b| lanmsg_media::decode_image(&b).ok())
            {
                Some(img) => Loaded::Image {
                    w: img.width,
                    h: img.height,
                    rgba: img.pixels,
                },
                None => Loaded::Failed,
            }
        }
    }
}

impl From<Loaded> for PreviewBody {
    fn from(l: Loaded) -> Self {
        match l {
            Loaded::Text(t) => PreviewBody::Text(t),
            Loaded::Image { w, h, rgba } => PreviewBody::Image {
                w,
                h,
                rgba: Rc::new(rgba),
            },
            Loaded::Binary => PreviewBody::Binary,
            Loaded::Failed => PreviewBody::Failed,
        }
    }
}

/// 预览状态：请求、加载结果与后台线程的收件槽。
#[derive(Clone)]
pub struct PreviewState {
    pub req: Signal<Option<PreviewReq>>,
    /// 加载结果每变一次自增，对话框内容按它重建。
    version: Signal<u64>,
    body: Rc<RefCell<PreviewBody>>,
    inbox: Arc<Mutex<Option<(u64, Loaded)>>>,
}

impl PreviewState {
    pub fn new() -> PreviewState {
        PreviewState {
            req: signal(None),
            version: signal(0),
            body: Rc::new(RefCell::new(PreviewBody::Loading)),
            inbox: Arc::new(Mutex::new(None)),
        }
    }

    fn next_epoch(&self) -> u64 {
        self.req.get().map_or(0, |r| r.epoch) + 1
    }

    fn set_body(&self, b: PreviewBody) {
        *self.body.borrow_mut() = b;
        self.version.update(|v| *v += 1);
    }

    /// 打开预览：对话框立即出现，读取与解码在后台线程进行。
    pub fn open(&self, path: String, name: String, kind: PreviewKind) {
        let epoch = self.next_epoch();
        self.set_body(PreviewBody::Loading);
        self.req.set(Some(PreviewReq {
            epoch,
            path: path.clone(),
            name,
            kind,
        }));
        let inbox = self.inbox.clone();
        std::thread::spawn(move || {
            let loaded = load(kind, &path);
            if let Ok(mut slot) = inbox.lock() {
                *slot = Some((epoch, loaded));
            }
        });
    }

    /// 演示与截图用：同步加载。
    pub fn open_blocking(&self, path: String, name: String, kind: PreviewKind) {
        let epoch = self.next_epoch();
        self.set_body(load(kind, &path).into());
        self.req.set(Some(PreviewReq {
            epoch,
            path,
            name,
            kind,
        }));
    }

    pub fn close(&self) {
        self.req.set(None);
    }

    /// 周期调用：取回后台结果（过期请求的结果丢弃）。
    pub fn tick(&self) {
        let taken = self.inbox.lock().ok().and_then(|mut s| s.take());
        if let Some((epoch, loaded)) = taken {
            if self.req.get().is_some_and(|r| r.epoch == epoch) {
                self.set_body(loaded.into());
            }
        }
    }
}

impl Default for PreviewState {
    fn default() -> Self {
        PreviewState::new()
    }
}

/// 点击一个已下载的文件：能预览就进入应用内预览，否则交给系统程序。
pub fn open_local(
    ctx: &mut windui::core::EventCtx,
    ui: &UiState,
    mime: &str,
    name: &str,
    path: &str,
) {
    match preview_kind(mime, name) {
        // 文本文件头里有 NUL：不预览，回退到系统程序
        Some(PreviewKind::Text) if looks_binary(Path::new(path)) => ctx.open_url(path),
        Some(kind) => ui.preview.open(path.to_string(), name.to_string(), kind),
        None => ctx.open_url(path),
    }
}

/// 对话框宿主：放在最上层；随 `req` 重建。
pub fn preview_host(ui: &UiState) -> Element {
    let state = ui.preview.clone();
    Element::switcher(state.req, move |req| match req {
        None => Element::leaf().size(0, 0),
        Some(r) => preview_dialog(&state, r),
    })
}

fn muted(text: impl Into<TextContent>) -> Element {
    Element::label(text)
        .font_size(style::FONT_BODY)
        .fg_role(Role::TextMuted)
}

fn preview_dialog(state: &PreviewState, req: PreviewReq) -> Element {
    let show = signal(true);
    let st = state.clone();
    let body = Element::switcher(state.version, move |_| body_view(&st.body.borrow()));

    let (path_open, path_dir) = (req.path.clone(), parent_dir(&req.path));
    let mut footer = Element::row().width_match().spacing(style::SPACE_SM);
    if let Some(d) = path_dir {
        footer = footer.child(
            Element::button(strings::PREVIEW_SHOW_IN_FOLDER)
                .outline()
                .on_click(move |ctx| ctx.open_url(&d)),
        );
    }
    let (st_close, st_x) = (state.clone(), state.clone());
    let footer = footer
        .child(Element::flex_spacer())
        .child(
            Element::button(strings::PREVIEW_OPEN_DEFAULT)
                .outline()
                .on_click(move |ctx| ctx.open_url(&path_open)),
        )
        .child(Element::button(strings::PREVIEW_CLOSE).on_click(move |_| st_close.close()));
    Element::dialog_panel(
        show,
        req.name,
        DIALOG_W,
        move |_| st_x.close(),
        body,
        footer,
    )
}

fn body_view(b: &PreviewBody) -> Element {
    match b {
        PreviewBody::Loading => muted(strings::PREVIEW_LOADING)
            .width_match()
            .height(TEXT_BOX_H),
        PreviewBody::Binary => muted(strings::PREVIEW_BINARY).width_match(),
        PreviewBody::Failed => muted(strings::PREVIEW_FAILED).width_match(),
        PreviewBody::Text(t) => {
            let mut col = Element::col().width_match().spacing(style::SPACE_SM).child(
                Element::text_input(signal(t.text.clone()), "")
                    .multiline()
                    .wrap(true)
                    .read_only()
                    .select_range(0, 0)
                    .font_family(MONO_FONT)
                    .font_size(style::FONT_BODY)
                    .width_match()
                    .height(TEXT_BOX_H),
            );
            if t.truncated {
                col = col.child(
                    Element::label(strings::PREVIEW_TRUNCATED)
                        .font_size(style::FONT_META)
                        .fg_role(Role::TextSubtle),
                );
            }
            col
        }
        PreviewBody::Image { w, h, rgba } => {
            let (dw, dh) = fit_within(*w, *h, IMAGE_BOX_W, IMAGE_BOX_H);
            Element::stack()
                .width_match()
                .height(IMAGE_BOX_H as i32)
                .child(
                    Element::image_rgba(*w, *h, rgba)
                        .size(dw as i32, dh as i32)
                        .align(Align::Center),
                )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_by_mime() {
        assert_eq!(preview_kind("image/png", "x"), Some(PreviewKind::Image));
        assert_eq!(preview_kind("IMAGE/JPEG", "x"), Some(PreviewKind::Image));
        assert_eq!(
            preview_kind("text/plain; charset=utf-8", "x"),
            Some(PreviewKind::Text)
        );
        assert_eq!(preview_kind("text/x-unknown", "x"), Some(PreviewKind::Text));
        assert_eq!(preview_kind("application/pdf", "x.pdf"), None);
        assert_eq!(preview_kind("video/mp4", "x.mp4"), None);
    }

    #[test]
    fn kind_falls_back_to_extension() {
        let octet = "application/octet-stream";
        assert_eq!(preview_kind(octet, "a.WebP"), Some(PreviewKind::Image));
        for ext in TEXT_EXTS {
            assert_eq!(
                preview_kind(octet, &format!("f.{ext}")),
                Some(PreviewKind::Text),
                "{ext}"
            );
        }
        assert_eq!(preview_kind(octet, "noext"), None);
        assert_eq!(preview_kind(octet, "a.exe"), None);
        // 桌面端没有 bmp 解码器，不进应用内预览
        assert_eq!(preview_kind(octet, "a.bmp"), None);
    }

    #[test]
    fn nul_in_head_is_binary() {
        assert!(decode_text(b"ab\0cd", false).is_none());
        // 8 KiB 之后的 NUL 不算
        let mut late = vec![b'a'; BINARY_SNIFF_BYTES];
        late.push(0);
        assert!(decode_text(&late, false).is_some());
    }

    #[test]
    fn bom_is_stripped() {
        let t = decode_text(b"\xEF\xBB\xBFhello", false).expect("text");
        assert_eq!(t.text, "hello");
        assert!(!t.truncated);
    }

    #[test]
    fn invalid_utf8_becomes_replacement_char() {
        let t = decode_text(b"a\xFFb", false).expect("text");
        assert_eq!(t.text, "a\u{FFFD}b");
    }

    #[test]
    fn truncation_drops_split_char_but_keeps_real_errors() {
        // "你" 的前两个字节被截断：丢弃，不产生替换符
        let t = decode_text(b"ab\xE4\xBD", true).expect("text");
        assert_eq!(t.text, "ab");
        assert!(t.truncated);
        // 中间的非法字节仍然替换
        let t = decode_text(b"a\xFFb", true).expect("text");
        assert_eq!(t.text, "a\u{FFFD}b");
    }

    #[test]
    fn read_truncates_at_one_mib_and_flags_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let big = dir.path().join("big.txt");
        std::fs::write(&big, vec![b'x'; TEXT_MAX_BYTES + 10]).expect("write");
        let t = read_text_preview(&big).expect("read").expect("text");
        assert_eq!(t.text.len(), TEXT_MAX_BYTES);
        assert!(t.truncated);

        let exact = dir.path().join("exact.txt");
        std::fs::write(&exact, vec![b'x'; TEXT_MAX_BYTES]).expect("write");
        let t = read_text_preview(&exact).expect("read").expect("text");
        assert!(!t.truncated);

        let bin = dir.path().join("b.bin");
        std::fs::write(&bin, b"\x7fELF\0\0").expect("write");
        assert!(read_text_preview(&bin).expect("read").is_none());
        assert!(looks_binary(&bin));
        assert!(!looks_binary(&exact));
    }

    #[test]
    fn fit_never_upscales() {
        assert_eq!(fit_within(100, 50, 672, 440), (100, 50));
        assert_eq!(fit_within(1344, 440, 672, 440), (672, 220));
        assert_eq!(fit_within(800, 1760, 672, 440), (200, 440));
        assert_eq!(fit_within(0, 0, 10, 10), (0, 0));
    }
}
