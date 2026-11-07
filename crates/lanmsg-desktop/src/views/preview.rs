// 应用内预览：已下载到本地的常见图片与文本文件，在对话框里直接查看。
// 判定与文本读取是纯函数（见测试）；读取与解码放后台线程，结果经收件槽由 `tick` 取回。
// 注释中文，标识符英文。

use std::cell::RefCell;
use std::io::Read;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
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
/// 预览对话框：距窗口边缘的留白与宽度上限（高度撑满；对 Match 子节点再设 max_height 不会重新测量，故不设）。
const PANEL_MARGIN: i32 = 32;
const PANEL_MAX_W: i32 = 900;
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
        text: split_long_lines(&String::from_utf8_lossy(data)),
        truncated,
    })
}

/// 单行最多显示的字符数；更长的行硬切分，避免压缩 JSON 这类超长单行拖垮排版。
pub const MAX_LINE_CHARS: usize = 4096;

/// 把超过 `MAX_LINE_CHARS` 个字符的行按该长度插入换行（按字符边界切，不拆 UTF-8）。
pub fn split_long_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / MAX_LINE_CHARS + 1);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut count = 0;
        for ch in line.chars() {
            if count == MAX_LINE_CHARS {
                out.push('\n');
                count = 0;
            }
            out.push(ch);
            count += 1;
        }
    }
    out
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
    /// 只增不减的代次：每次打开自增，关闭不回退，旧请求的结果因此永远对不上新请求。
    latest: Arc<AtomicU64>,
    inbox: Inbox,
}

/// 后台线程的收件槽：（代次，结果）。
type Inbox = Arc<Mutex<Option<(u64, Loaded)>>>;

/// 后台线程投递结果：代次已不是最新（又打开过别的文件）就丢弃，不会覆盖新结果。
fn deliver(inbox: &Inbox, latest: &AtomicU64, epoch: u64, loaded: Loaded) {
    if let Ok(mut slot) = inbox.lock() {
        if latest.load(Ordering::SeqCst) == epoch {
            *slot = Some((epoch, loaded));
        }
    }
}

impl PreviewState {
    pub fn new() -> PreviewState {
        PreviewState {
            req: signal(None),
            version: signal(0),
            body: Rc::new(RefCell::new(PreviewBody::Loading)),
            latest: Arc::new(AtomicU64::new(0)),
            inbox: Arc::new(Mutex::new(None)),
        }
    }

    fn next_epoch(&self) -> u64 {
        self.latest.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn set_body(&self, b: PreviewBody) {
        *self.body.borrow_mut() = b;
        self.version.update(|v| *v += 1);
    }

    /// 打开预览：对话框立即出现，读取与解码在后台线程进行。
    pub fn open(&self, path: String, name: String, kind: PreviewKind) {
        let epoch = self.next_epoch();
        if let Ok(mut slot) = self.inbox.lock() {
            slot.take();
        }
        self.set_body(PreviewBody::Loading);
        self.req.set(Some(PreviewReq {
            epoch,
            path: path.clone(),
            name,
            kind,
        }));
        let (inbox, latest) = (self.inbox.clone(), self.latest.clone());
        std::thread::spawn(move || {
            let loaded = load(kind, &path);
            deliver(&inbox, &latest, epoch, loaded);
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
    // 面板随窗口伸缩：撑满可用区域，再由上限收住（拿不到窗口尺寸，故用 Match + max）
    let th = windui::theme::current();
    let header = Element::row()
        .width_match()
        .cross(Align::Center)
        .child(
            Element::label(req.name)
                .font_size(th.metrics.font_lg)
                .font_weight(windui::text::WEIGHT_SEMIBOLD)
                .max_lines(1)
                .truncate(Truncate::End)
                .fg_role(Role::Text)
                .weight(1.0),
        )
        .child(
            Element::icon_button("\u{2715}")
                .size(28, 28)
                .fg_role(Role::TextMuted)
                .on_click(move |_| st_x.close()),
        );
    let panel = Element::col()
        .width_match()
        .max_width(PANEL_MAX_W)
        .height_match()
        .margin(PANEL_MARGIN)
        .bg_role(Role::Surface)
        .corner(th.metrics.corner_lg)
        .padding(th.metrics.space_xl)
        .spacing(th.metrics.space_lg)
        .child(header)
        .child(body.fill().weight(1.0))
        .child(footer);
    Element::dialog(show, panel)
}

fn body_view(b: &PreviewBody) -> Element {
    match b {
        PreviewBody::Loading => muted(strings::PREVIEW_LOADING).fill(),
        PreviewBody::Binary => muted(strings::PREVIEW_BINARY).width_match(),
        PreviewBody::Failed => muted(strings::PREVIEW_FAILED).width_match(),
        PreviewBody::Text(t) => {
            let mut col = Element::col().fill().spacing(style::SPACE_SM).child(
                Element::text_input(signal(t.text.clone()), "")
                    .multiline()
                    .wrap(true)
                    .read_only()
                    .select_range(0, 0)
                    .font_family(MONO_FONT)
                    .font_size(style::FONT_BODY)
                    .width_match()
                    .weight(1.0),
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
        // 铺满可用区域、等比适配；封顶于原尺寸，小图不放大
        PreviewBody::Image { w, h, rgba } => Element::stack().fill().child(
            Element::image_rgba(*w, *h, rgba)
                .fit(Fit::Contain)
                .fill()
                .max_width(*w as i32)
                .max_height(*h as i32)
                .align(Align::Center),
        ),
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
        // 超长单行会被硬切分，换行符不计入
        assert_eq!(t.text.replace('\n', "").len(), TEXT_MAX_BYTES);
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
    fn long_lines_are_split_on_char_boundaries() {
        let short = "ab\ncd\n";
        assert_eq!(split_long_lines(short), short);
        let line: String = "你".repeat(MAX_LINE_CHARS * 2 + 5);
        let out = split_long_lines(&line);
        let parts: Vec<&str> = out.split('\n').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].chars().count(), MAX_LINE_CHARS);
        assert_eq!(parts[1].chars().count(), MAX_LINE_CHARS);
        assert_eq!(parts[2].chars().count(), 5);
        assert_eq!(out.replace('\n', ""), line);
        // 恰好等于上限不切
        let exact = "x".repeat(MAX_LINE_CHARS);
        assert_eq!(split_long_lines(&exact), exact);
    }

    fn text_loaded(s: &str) -> Loaded {
        Loaded::Text(TextPreview {
            text: s.into(),
            truncated: false,
        })
    }

    fn shown_text(st: &PreviewState) -> Option<String> {
        match &*st.body.borrow() {
            PreviewBody::Text(t) => Some(t.text.clone()),
            _ => None,
        }
    }

    #[test]
    fn epoch_never_repeats_after_close() {
        let st = PreviewState::new();
        st.open_blocking("/nonexistent/a".into(), "a".into(), PreviewKind::Text);
        let a = st.req.get().expect("req").epoch;
        st.close();
        st.open_blocking("/nonexistent/b".into(), "b".into(), PreviewKind::Text);
        assert!(st.req.get().expect("req").epoch > a);
    }

    #[test]
    fn stale_result_after_close_and_reopen_is_discarded() {
        let st = PreviewState::new();
        // 打开 A（后台线程尚未返回），关闭，再打开 B
        let a = st.next_epoch();
        st.close();
        let b = st.next_epoch();
        st.req.set(Some(PreviewReq {
            epoch: b,
            path: "/b".into(),
            name: "b".into(),
            kind: PreviewKind::Text,
        }));
        // A 的结果迟到：因代次已不是最新，不会进入收件槽
        deliver(&st.inbox, &st.latest, a, text_loaded("from A"));
        st.tick();
        assert_eq!(shown_text(&st), None);
        deliver(&st.inbox, &st.latest, b, text_loaded("from B"));
        st.tick();
        assert_eq!(shown_text(&st).as_deref(), Some("from B"));
    }

    #[test]
    fn old_result_cannot_overwrite_new_one() {
        let st = PreviewState::new();
        let a = st.next_epoch();
        let b = st.next_epoch();
        st.req.set(Some(PreviewReq {
            epoch: b,
            path: "/b".into(),
            name: "b".into(),
            kind: PreviewKind::Text,
        }));
        // B 先返回，A 后到：A 不能覆盖 B
        deliver(&st.inbox, &st.latest, b, text_loaded("from B"));
        deliver(&st.inbox, &st.latest, a, text_loaded("from A"));
        st.tick();
        assert_eq!(shown_text(&st).as_deref(), Some("from B"));
    }
}
