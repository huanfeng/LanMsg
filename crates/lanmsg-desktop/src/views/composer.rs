// 输入区：多行输入框、文件/文件夹按钮、发送按钮；支持粘贴图片与拖入文件。
// 纯函数（空白规整、拖入分流、粘贴文件命名与 PNG 编码）带单测。注释中文，标识符英文。
//
// 粘贴图片的临时文件写在 `<data_dir>/cache/paste/`，发送后保留：核心发送文件时按路径引用
// 原文件（消息的 `local_path` 指向它，“打开”依赖它存在），所以界面侧不能删除已发送的图片。

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use windui::prelude::*;

use crate::strings;
use crate::style;
use crate::tz;
use crate::ui_state::UiState;
use crate::views::chats::now_millis;

/// 输入框高度（约 3 行；windui 的多行输入框不支持随内容增高，超出部分在框内滚动）。
const INPUT_H: i32 = 76;
/// 粘贴预览的最大边长。
const PREVIEW_MAX: i32 = 240;

/// 去掉首尾空白后为空则不发送；内部换行保留。
pub fn normalize_outgoing(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// 拖入的路径分为 `(文件, 目录)`，各自保持原顺序。
pub fn split_drop(
    paths: &[PathBuf],
    is_dir: &dyn Fn(&Path) -> bool,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    paths.iter().cloned().partition(|p| !is_dir(p))
}

/// 从 1970-01-01 起的天数换算公历 (年, 月, 日)（Howard Hinnant 的 civil_from_days）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// 粘贴图片的文件名：`paste-YYYYMMDD-HHMMSS.png`（本地时间）。
pub fn paste_file_name(millis: i64, tz_offset_secs: i32) -> String {
    let secs = millis.div_euclid(1000) + i64::from(tz_offset_secs);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    format!(
        "paste-{y:04}{m:02}{d:02}-{:02}{:02}{:02}.png",
        sod / 3600,
        sod % 3600 / 60,
        sod % 60
    )
}

/// 非预乘 RGBA8 编码为 PNG。
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    // 编码器遇到长度不符会 panic，先自行校验
    if width == 0 || height == 0 || rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return Err(format!("图片数据长度与 {width}x{height} 不符"));
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// 等待用户确认发送的粘贴图片。相等性只看文件与粘贴时刻（供 switcher 判断是否重建）。
#[derive(Clone)]
pub struct PastedImage {
    path: PathBuf,
    stamp: i64,
    width: u32,
    height: u32,
    rgba: Rc<Vec<u8>>,
}

impl PartialEq for PastedImage {
    fn eq(&self, o: &Self) -> bool {
        self.path == o.path && self.stamp == o.stamp
    }
}
impl Eq for PastedImage {}
impl Hash for PastedImage {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.path.hash(h);
        self.stamp.hash(h);
    }
}

/// 处理粘贴的图片：编码并写入临时目录，成功后交给确认小窗。
fn on_paste(
    ctx: &mut windui::core::EventCtx,
    ui: &UiState,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
) {
    let now = now_millis();
    let dir = ui.data_dir().join("cache").join("paste");
    let path = dir.join(paste_file_name(now, tz::local_offset_secs()));
    let saved = encode_png(width, height, &rgba)
        .and_then(|png| {
            std::fs::create_dir_all(&dir)
                .map_err(|e| e.to_string())
                .map(|_| png)
        })
        .and_then(|png| std::fs::write(&path, png).map_err(|e| e.to_string()));
    match saved {
        Ok(()) => ui.paste.set(Some(PastedImage {
            path,
            stamp: now,
            width,
            height,
            rgba: Rc::new(rgba),
        })),
        Err(e) => ctx.toast_err(format!("{}{e}", strings::PASTE_SAVE_FAILED)),
    }
}

/// 粘贴确认小窗宿主：随 `ui.paste` 重建；放在聊天区 stack 的最上层。
pub fn paste_host(ui: &UiState) -> Element {
    let ui = ui.clone();
    Element::switcher(ui.paste, move |p| match p {
        Some(p) => paste_dialog(&ui, p),
        None => Element::leaf().size(0, 0),
    })
}

fn paste_dialog(ui: &UiState, p: PastedImage) -> Element {
    let show = signal(true);
    let (w, h) = (p.width as f32, p.height as f32);
    let scale = (PREVIEW_MAX as f32 / w.max(h)).min(1.0);
    let preview = Element::image_rgba(p.width, p.height, &p.rgba)
        .size(((w * scale) as i32).max(1), ((h * scale) as i32).max(1))
        .corner(style::RADIUS_CONTROL);
    let body = Element::col()
        .width_match()
        .cross(Align::Center)
        .child(preview);

    let ui_cancel = ui.clone();
    let ui_close = ui.clone();
    let ui_send = ui.clone();
    let (path_cancel, path_close, path_send) = (p.path.clone(), p.path.clone(), p.path.clone());
    let footer = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(Element::flex_spacer())
        .child(
            Element::button(strings::PASTE_CANCEL)
                .outline()
                .on_click(move |_| discard_paste(&ui_cancel, &path_cancel)),
        )
        .child(Element::button(strings::PASTE_SEND).on_click(move |_| {
            let path = path_send.to_string_lossy().into_owned();
            if let Some(conv) = ui_send.read(|m| m.current_conv.clone()) {
                ui_send.act(|m| m.send_files(conv, vec![path]));
                ui_send.jump_to_bottom();
            }
            ui_send.paste.set(None);
        }));
    Element::dialog_panel(
        show,
        strings::PASTE_TITLE,
        320,
        move |_| discard_paste(&ui_close, &path_close),
        body,
        footer,
    )
}

/// 取消发送：删除尚未被任何消息引用的临时图片。
fn discard_paste(ui: &UiState, path: &Path) {
    // 删除失败（已被清理等）不影响取消
    let _ = std::fs::remove_file(path);
    ui.paste.set(None);
}

/// 提交输入框文字：空白不发；发送后清空并回到列表底部。
fn submit(ui: &UiState, conv: &str, text: Signal<String>) {
    if let Some(t) = normalize_outgoing(&text.get()) {
        let conv = conv.to_string();
        ui.act(|m| m.send_text(conv, t));
        text.set(String::new());
        ui.jump_to_bottom();
    }
}

/// 拖入文件：文件合并为一条消息，每个目录各一条。
pub fn on_drop(ui: &UiState, paths: &[PathBuf]) {
    let Some(conv) = ui.read(|m| m.current_conv.clone()) else {
        return;
    };
    let (files, dirs) = split_drop(paths, &|p| p.is_dir());
    let lossy = |v: Vec<PathBuf>| -> Vec<String> {
        v.iter().map(|p| p.to_string_lossy().into_owned()).collect()
    };
    ui.act(|m| m.send_dropped(conv, lossy(files), lossy(dirs)));
    ui.jump_to_bottom();
}

/// 工具按钮：弹出文件 / 目录选择框，结果经 `send` 发送；弹不出来时在 `hint` 里提示。
fn pick_button(
    ui: &UiState,
    conv: &str,
    label: &'static str,
    folder: bool,
    hint: Signal<String>,
) -> Element {
    let (ui, conv) = (ui.clone(), conv.to_string());
    Element::button(label).outline().on_click(move |ctx| {
        hint.set(String::new());
        let (ui, conv) = (ui.clone(), conv.clone());
        let failed = move || {
            if PickDialog::last_pick_failed() {
                hint.set(strings::PICK_UNAVAILABLE.to_string());
            }
        };
        if folder {
            let dlg = PickDialog::new().title(strings::PICK_FOLDER_TITLE);
            ctx.request_pick_folder(dlg, move |p| match p {
                Some(p) => {
                    let path = p.to_string_lossy().into_owned();
                    ui.act(|m| m.send_folder(conv, path));
                    ui.jump_to_bottom();
                }
                None => failed(),
            });
        } else {
            let dlg = PickDialog::new().title(strings::PICK_FILES_TITLE);
            ctx.request_pick_files(dlg, move |ps| match ps {
                Some(ps) if !ps.is_empty() => {
                    let paths = ps
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    ui.act(|m| m.send_files(conv, paths));
                    ui.jump_to_bottom();
                }
                Some(_) => {}
                None => failed(),
            });
        }
    })
}

/// 底部输入区：surface 底、顶部 1px 分隔线；Enter 发送，Shift+Enter 换行。
pub fn composer(ui: &UiState, conv: String) -> Element {
    let text = signal(String::new());
    let hint = signal(String::new());
    let has_hint = hint.map(|h| !h.is_empty());
    let blank = text.map(|t| normalize_outgoing(t).is_none());

    let (ui_submit, conv_submit) = (ui.clone(), conv.clone());
    let ui_paste = ui.clone();
    let input = Element::text_input(text, strings::COMPOSER_PLACEHOLDER)
        .multiline()
        .submit_on_enter(true)
        .on_submit(move |_| submit(&ui_submit, &conv_submit, text))
        // 剪贴板只有图片时触发；原生 Wayland 不支持读剪贴板图片，此时回调不会触发
        .on_paste_image(move |ctx, img| on_paste(ctx, &ui_paste, img.width, img.height, img.rgba))
        .autofocus()
        .width_match()
        .height(INPUT_H);

    let (ui_send, conv_send) = (ui.clone(), conv.clone());
    let send = Element::button(strings::COMPOSER_SEND)
        .enabled_when(move || normalize_outgoing(&text.get()).is_some())
        .on_click(move |_| submit(&ui_send, &conv_send, text));

    let tools = Element::row()
        .width_match()
        .spacing(style::SPACE_SM)
        .cross(Align::Center)
        .child(pick_button(ui, &conv, strings::COMPOSER_FILE, false, hint))
        .child(pick_button(ui, &conv, strings::COMPOSER_FOLDER, true, hint))
        .child(
            Element::label(hint)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle)
                .visible_signal(has_hint),
        )
        // 发送按钮禁用时说明原因（规范第 7 节）
        .child(
            Element::label(strings::COMPOSER_EMPTY_HINT)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle)
                .visible_signal(blank),
        )
        .child(Element::flex_spacer())
        .child(send);

    Element::col()
        .width_match()
        .bg_role(Role::Surface)
        .child(Element::divider())
        .child(
            Element::col()
                .width_match()
                .padding(style::SPACE_MD)
                .spacing(style::SPACE_SM)
                .child(input)
                .child(tools),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn normalize_outgoing_cases() {
        assert_eq!(normalize_outgoing("  \n "), None);
        assert_eq!(normalize_outgoing(" a\nb "), Some("a\nb".to_string()));
    }

    #[test]
    fn split_drop_separates_dirs() {
        let paths: Vec<PathBuf> = ["a.txt", "dir1", "b.png"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let is_dir = |p: &Path| p == Path::new("dir1");
        let (files, dirs) = split_drop(&paths, &is_dir);
        assert_eq!(files, [PathBuf::from("a.txt"), PathBuf::from("b.png")]);
        assert_eq!(dirs, [PathBuf::from("dir1")]);
    }

    #[test]
    fn paste_file_name_uses_local_time() {
        // 2026-03-15 09:05:07 UTC，+08:00 -> 17:05:07
        assert_eq!(
            paste_file_name(1_773_565_507_000, 8 * 3600),
            "paste-20260315-170507.png"
        );
    }

    #[test]
    fn encode_png_round_trips() {
        let png = encode_png(2, 1, &[255, 0, 0, 255, 0, 255, 0, 128]).expect("编码");
        let img = image::load_from_memory(&png).expect("解码").to_rgba8();
        assert_eq!(img.dimensions(), (2, 1));
        assert_eq!(img.get_pixel(1, 0).0, [0, 255, 0, 128]);
        assert!(encode_png(2, 2, &[0; 4]).is_err(), "长度不符报错");
    }
}
