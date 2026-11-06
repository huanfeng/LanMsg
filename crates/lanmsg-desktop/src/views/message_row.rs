// 单条消息的渲染：气泡、图片/视频缩略图、文件卡片与各状态交互。
// 视觉规范第 8 节；纯函数（单位换算、图标选择、聚合进度）带单测。注释中文，标识符英文。

use std::collections::BTreeMap;

use lanmsg_api::types::*;
use windui::event::MenuItem;
use windui::prelude::*;

use crate::model::{effective_state, Model, TransferInfo};
use crate::strings;
use crate::style;
use crate::ui_state::UiState;

/// 气泡最大宽度（规范：正文最大行宽约 560px）。
pub const BUBBLE_MAX_W: i32 = 560;
/// 缩略图最长边与最短边下限。
const THUMB_MAX: i32 = 240;
const THUMB_MIN: i32 = 48;
/// 文件卡片宽度。
const CARD_W: i32 = 320;

// ---------------------------------------------------------------------------
// 纯函数
// ---------------------------------------------------------------------------

/// 字节数的人类可读形式：1024 进制，KB/MB 一位小数、GB 两位。
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.2} GB", b / (KB * KB * KB))
    }
}

/// 速率：`human_size` + `/s`。
pub fn human_speed(bps: u64) -> String {
    format!("{}/s", human_size(bps))
}

/// 视频时长：`0:07`、`12:34`、`1:02:03`。
pub fn duration_label(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, sec) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// 是否可撤回：自己发的、未撤回、仍在时限内。
pub fn can_recall(m: &MessageView, now_millis: i64) -> bool {
    m.is_mine
        && m.state != MessageState::Recalled
        && now_millis - m.hlc_millis < lanmsg_proto::consts::RECALL_WINDOW.as_millis() as i64
}

/// 文件类型图标分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIcon {
    Image,
    Video,
    Audio,
    Pdf,
    Doc,
    Sheet,
    Slide,
    Archive,
    Text,
    Generic,
}

/// 按 MIME 优先、扩展名兜底选择图标。
pub fn file_icon(mime: &str, name: &str) -> FileIcon {
    let mime = mime.to_ascii_lowercase();
    if mime.starts_with("image/") {
        return FileIcon::Image;
    }
    if mime.starts_with("video/") {
        return FileIcon::Video;
    }
    if mime.starts_with("audio/") {
        return FileIcon::Audio;
    }
    if mime == "application/pdf" {
        return FileIcon::Pdf;
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => FileIcon::Pdf,
        "doc" | "docx" | "rtf" | "odt" | "pages" => FileIcon::Doc,
        "xls" | "xlsx" | "csv" | "ods" | "numbers" => FileIcon::Sheet,
        "ppt" | "pptx" | "odp" | "key" => FileIcon::Slide,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "tgz" => FileIcon::Archive,
        "txt" | "md" | "log" | "json" | "xml" | "yaml" | "yml" | "ini" => FileIcon::Text,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" => FileIcon::Image,
        "mp4" | "mkv" | "mov" | "avi" | "webm" => FileIcon::Video,
        "mp3" | "wav" | "flac" | "m4a" | "ogg" => FileIcon::Audio,
        _ if mime.starts_with("text/") => FileIcon::Text,
        _ if mime.contains("zip") || mime.contains("rar") => FileIcon::Archive,
        _ => FileIcon::Generic,
    }
}

fn icon_bytes(i: FileIcon) -> &'static [u8] {
    match i {
        FileIcon::Image => style::ICON_FILE_IMAGE,
        FileIcon::Video => style::ICON_FILE_VIDEO,
        FileIcon::Audio => style::ICON_FILE_AUDIO,
        FileIcon::Pdf => style::ICON_FILE_PDF,
        FileIcon::Doc => style::ICON_FILE_DOC,
        FileIcon::Sheet => style::ICON_FILE_SHEET,
        FileIcon::Slide => style::ICON_FILE_SLIDE,
        FileIcon::Archive => style::ICON_FILE_ARCHIVE,
        FileIcon::Text => style::ICON_FILE_TEXT,
        FileIcon::Generic => style::ICON_FILE_GENERIC,
    }
}

/// 缩略图显示尺寸：等比缩进 240x240，两边不低于 48；缺尺寸时用 240x160。
pub fn thumb_size(w: Option<u32>, h: Option<u32>) -> (i32, i32) {
    let (Some(w), Some(h)) = (w, h) else {
        return (THUMB_MAX, 160);
    };
    if w == 0 || h == 0 {
        return (THUMB_MAX, 160);
    }
    let (w, h) = (w as f64, h as f64);
    let scale = (THUMB_MAX as f64 / w).min(THUMB_MAX as f64 / h).min(1.0);
    let fit = |v: f64| ((v * scale).round() as i32).clamp(THUMB_MIN, THUMB_MAX);
    (fit(w), fit(h))
}

/// 一组文件的聚合状态：准备中 > 下载中 > 待下载 > 源离线 > 已在本地。
pub fn aggregate_state(items: &[FileItemView]) -> BlobState {
    aggregate_with(items, |i| i.state.clone())
}

/// 同 `aggregate_state`，但用模型推导每项的有效状态（点击下载后立刻显示“下载中”）。
fn aggregate_effective(items: &[FileItemView], model: &Model) -> BlobState {
    aggregate_with(items, |i| effective_state(i, model))
}

fn aggregate_with(
    items: &[FileItemView],
    state_of: impl Fn(&FileItemView) -> BlobState,
) -> BlobState {
    let states: Vec<BlobState> = items.iter().map(state_of).collect();
    let any = |s: BlobState| states.contains(&s);
    if any(BlobState::Preparing) {
        BlobState::Preparing
    } else if any(BlobState::Downloading) {
        BlobState::Downloading
    } else if any(BlobState::Remote) {
        BlobState::Remote
    } else if any(BlobState::SourceOffline) {
        BlobState::SourceOffline
    } else {
        BlobState::Local
    }
}

/// 一组文件的合计进度（0..=1）与合计速率；没有任何传输记录时为 (0, 0)。
pub fn aggregate_progress(
    transfers: &BTreeMap<String, TransferInfo>,
    hashes: &[String],
) -> (f32, u64) {
    let (mut done, mut total, mut speed) = (0u64, 0u64, 0u64);
    for t in hashes.iter().filter_map(|h| transfers.get(h)) {
        done += t.done;
        total += t.total;
        speed += t.bytes_per_sec;
    }
    if total == 0 {
        (0.0, 0)
    } else {
        ((done as f64 / total as f64) as f32, speed)
    }
}

/// 文件所在目录；没有目录部分时为 None。
pub fn parent_dir(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
}

fn message_hashes(m: &MessageView) -> Vec<String> {
    match &m.body {
        MessageBodyView::Files { items } | MessageBodyView::Folder { items, .. } => {
            items.iter().map(|i| i.hash.clone()).collect()
        }
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// 视图
// ---------------------------------------------------------------------------

/// 居中的灰色提示行（系统消息、已撤回、不支持）。
fn notice(text: impl Into<TextContent>) -> Element {
    Element::stack()
        .width_match()
        .padding_xy(0, style::SPACE_XS)
        .child(
            Element::label(text)
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle)
                .align(Align::Center),
        )
}

/// 右键菜单：复制（文字）、转发（文件类）、撤回（时限内自己的）、删除本地副本。
fn menu_builder(ui: &UiState, m: &MessageView) -> impl Fn() -> Vec<MenuItem> + 'static {
    let ui = ui.clone();
    let m = m.clone();
    move || {
        let mut v = Vec::new();
        if let MessageBodyView::Text { text } = &m.body {
            let t = text.clone();
            v.push(MenuItem::run(
                strings::MENU_COPY,
                move |ctx| ctx.clipboard_set(&t),
                false,
            ));
        }
        let hashes = message_hashes(&m);
        if !hashes.is_empty() {
            let ui = ui.clone();
            v.push(MenuItem::run(
                strings::MENU_FORWARD,
                move |_| ui.open_forward(hashes.clone()),
                false,
            ));
        }
        if can_recall(&m, crate::views::chats::now_millis()) {
            let (ui, id) = (ui.clone(), m.id.clone());
            v.push(MenuItem::run(
                strings::MENU_RECALL,
                move |_| ui.act(|md| md.recall(id.clone())),
                false,
            ));
        }
        let (ui, id) = (ui.clone(), m.id.clone());
        v.push(MenuItem::separator());
        v.push(
            MenuItem::run(
                strings::MENU_DELETE,
                move |_| ui.act(|md| md.delete_local(id.clone())),
                false,
            )
            .danger(),
        );
        v
    }
}

/// 气泡外壳：圆角 12，靠自己一侧的下角 4；我的 signal 底，对方 surface + line。
fn bubble_shell(mine: bool, child: Element) -> Element {
    let corners = if mine {
        Corners::new(12.0, 12.0, 4.0, 12.0)
    } else {
        Corners::new(12.0, 12.0, 12.0, 4.0)
    };
    let shell = Element::col()
        .padding_xy(style::SPACE_MD, style::SPACE_SM)
        .max_width(BUBBLE_MAX_W)
        .corners(corners)
        .child(child);
    if mine {
        shell.bg_role(Role::Accent)
    } else {
        shell.bg_role(Role::Surface).border_role(Role::Border, 1)
    }
}

fn text_bubble(mine: bool, text: &str) -> Element {
    let label = Element::label(text.to_string())
        .font_size(style::FONT_BODY)
        .fg_role(if mine { Role::OnAccent } else { Role::Text });
    bubble_shell(mine, label)
}

/// 下载中的进度与速率文字：`42% · 3.1 MB/s`。
fn progress_text(transfers: &BTreeMap<String, TransferInfo>, hashes: &[String]) -> String {
    let (frac, speed) = aggregate_progress(transfers, hashes);
    format!(
        "{}% · {}",
        (frac * 100.0).round() as u32,
        human_speed(speed)
    )
}

/// switcher 的键要 Eq + Hash，而 `BlobState` 没有：用数字码往返。
fn state_key(s: &BlobState) -> u8 {
    match s {
        BlobState::Remote => 0,
        BlobState::Downloading => 1,
        BlobState::Local => 2,
        BlobState::SourceOffline => 3,
        BlobState::Preparing => 4,
    }
}

fn state_from_key(k: u8) -> BlobState {
    match k {
        0 => BlobState::Remote,
        1 => BlobState::Downloading,
        2 => BlobState::Local,
        3 => BlobState::SourceOffline,
        _ => BlobState::Preparing,
    }
}

/// 文件的状态区：待下载 -> 下载按钮；下载中 -> 进度条 + 百分比与速率 + 取消；
/// 已在本地 -> 打开 / 打开所在文件夹；源离线、准备中 -> 文字。
/// 文件视图页复用（单个文件）。
pub fn file_state_area(ui: &UiState, item: &FileItemView) -> Element {
    state_area(ui, std::slice::from_ref(item))
}

fn state_area(ui: &UiState, items: &[FileItemView]) -> Element {
    // 状态随模型变化（下载请求发出、进度到达），所以按派生状态重建
    let list = items.to_vec();
    let l = list.clone();
    let key = ui.derive(move |m| state_key(&aggregate_effective(&l, m)));
    let ui = ui.clone();
    Element::switcher(key, move |st| {
        state_area_for(&ui, &list, state_from_key(st))
    })
}

fn state_area_for(ui: &UiState, items: &[FileItemView], state: BlobState) -> Element {
    let meta = |t: &'static str| {
        Element::label(t)
            .font_size(style::FONT_META)
            .fg_role(Role::TextMuted)
    };
    match state {
        BlobState::Remote => {
            let hashes: Vec<String> = items
                .iter()
                .filter(|i| i.state == BlobState::Remote)
                .map(|i| i.hash.clone())
                .collect();
            let ui = ui.clone();
            Element::button(strings::FILE_DOWNLOAD)
                .outline()
                .small()
                .on_click(move |_| ui.act(|m| m.download_all(hashes.clone())))
        }
        BlobState::Downloading => {
            let hashes: Vec<String> = items.iter().map(|i| i.hash.clone()).collect();
            let hs = hashes.clone();
            let frac = ui.derive(move |m| aggregate_progress(&m.transfers, &hs).0);
            let hs = hashes.clone();
            let text = ui.derive(move |m| progress_text(&m.transfers, &hs));
            let ui = ui.clone();
            Element::col()
                .width_match()
                .spacing(style::SPACE_XS)
                .child(Element::progress(frac).width_match())
                .child(
                    Element::row()
                        .width_match()
                        .cross(Align::Center)
                        .child(
                            Element::label(text)
                                .font_size(style::FONT_META)
                                .fg_role(Role::TextMuted)
                                .weight(1.0),
                        )
                        .child(Element::link(strings::FILE_CANCEL).on_click(move |_| {
                            ui.act(|m| hashes.iter().flat_map(|h| m.cancel_transfer(h)).collect())
                        })),
                )
        }
        BlobState::Local => {
            let first = items.first().and_then(|i| i.local_path.clone());
            let open_file = if items.len() == 1 {
                first.clone()
            } else {
                None
            };
            let folder = first.as_deref().and_then(parent_dir);
            let mut row = Element::row().spacing(style::SPACE_LG).cross(Align::Center);
            if let Some(p) = open_file {
                row = row
                    .child(Element::link(strings::FILE_OPEN).on_click(move |ctx| ctx.open_url(&p)));
            }
            if let Some(d) = folder {
                row = row.child(
                    Element::link(strings::FILE_OPEN_FOLDER).on_click(move |ctx| ctx.open_url(&d)),
                );
            }
            row
        }
        BlobState::SourceOffline => meta(strings::FILE_SOURCE_OFFLINE),
        BlobState::Preparing => meta(strings::FILE_PREPARING),
    }
}

/// 文件卡片：左侧类型图标，名字单行省略，大小与来源，状态区。
fn file_card(
    ui: &UiState,
    icon: &'static [u8],
    name: &str,
    size_text: String,
    source: &str,
    items: &[FileItemView],
) -> Element {
    let tile = Element::stack()
        .size(32, 32)
        .corner(style::RADIUS_NAV_ITEM)
        .bg_when(|| style::tokens().signal_soft)
        .child(style::icon(icon, 24, Role::AccentText).align(Align::Center));
    let mut info = Element::col()
        .weight(1.0)
        .spacing(style::SPACE_XS)
        .child(
            Element::label(name.to_string())
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End),
        )
        .child(
            Element::label(size_text)
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted),
        );
    if !source.is_empty() {
        info = info.child(
            Element::label(source.to_string())
                .font_size(style::FONT_META)
                .fg_role(Role::TextSubtle)
                .max_lines(1)
                .truncate(Truncate::End),
        );
    }
    info = info.child(state_area(ui, items));
    Element::row()
        .width(CARD_W)
        .padding(style::SPACE_MD)
        .spacing(style::SPACE_MD)
        .corner(style::RADIUS_CARD)
        .bg_role(Role::Surface)
        .border_role(Role::Border, 1)
        .child(tile)
        .child(info)
}

fn single_file_card(ui: &UiState, item: &FileItemView) -> Element {
    file_card(
        ui,
        icon_bytes(file_icon(&item.mime, &item.name)),
        &item.name,
        human_size(item.size),
        &item.source_label,
        std::slice::from_ref(item),
    )
}

fn folder_card(ui: &UiState, name: &str, items: &[FileItemView]) -> Element {
    let total: u64 = items.iter().map(|i| i.size).sum();
    let source = items
        .first()
        .map(|i| i.source_label.clone())
        .unwrap_or_default();
    file_card(
        ui,
        style::ICON_FILES,
        name,
        format!(
            "{} {}，{}",
            items.len(),
            strings::FILE_COUNT_UNIT,
            human_size(total)
        ),
        &source,
        items,
    )
}

/// 半透明深色小块（叠在图片上的标注）。
fn scrim_chip(child: Element) -> Element {
    Element::row()
        .padding_xy(style::SPACE_SM, style::SPACE_XS)
        .corner(style::SPACE_SM as f32)
        .bg_role(Role::Scrim)
        .cross(Align::Center)
        .spacing(style::SPACE_XS)
        .child(child)
}

fn chip_label(text: impl Into<TextContent>) -> Element {
    Element::label(text)
        .font_size(style::FONT_META)
        .fg_role(Role::OnAccent)
}

/// 缩略图上的状态标注与视频时长。
/// 视频的时长靠右下；与左下“点击下载”等标注并排时至少隔 8px，
/// 缩略图太窄放不下两块时，时长挪到右上角，保证不重叠。
fn thumb_overlay(
    ui: &UiState,
    item: &FileItemView,
    video: bool,
    w: i32,
    state: BlobState,
) -> Element {
    /// 并排放下左下标注与时长所需的最小缩略图宽度。
    const SIDE_BY_SIDE_MIN_W: i32 = 200;
    let duration = if video {
        item.duration_ms
            .map(|ms| scrim_chip(chip_label(duration_label(ms))))
    } else {
        None
    };
    let note = |chip: Element| {
        chip.align_xy(Align::Start, Align::End)
            .margin(style::SPACE_SM)
    };
    let mut layer = Element::stack().fill();
    let left_chip = match state {
        BlobState::Remote => Some(scrim_chip(
            Element::row()
                .spacing(style::SPACE_XS)
                .cross(Align::Center)
                .child(style::icon(style::ICON_DOWNLOAD, 16, Role::OnAccent))
                .child(chip_label(format!(
                    "{} {}",
                    strings::FILE_CLICK_TO_DOWNLOAD,
                    human_size(item.size)
                ))),
        )),
        BlobState::SourceOffline => Some(scrim_chip(chip_label(strings::FILE_SOURCE_OFFLINE))),
        BlobState::Preparing => Some(scrim_chip(chip_label(strings::FILE_PREPARING))),
        BlobState::Downloading => {
            let hashes = vec![item.hash.clone()];
            let hs = hashes.clone();
            let frac = ui.derive(move |m| aggregate_progress(&m.transfers, &hs).0);
            let text = ui.derive(move |m| progress_text(&m.transfers, &hashes));
            let hash = item.hash.clone();
            let ui = ui.clone();
            layer = layer.child(
                Element::col()
                    .width_match()
                    .padding(style::SPACE_SM)
                    .spacing(style::SPACE_XS)
                    .bg_role(Role::Scrim)
                    .align_xy(Align::Start, Align::End)
                    .child(Element::progress(frac).width_match())
                    .child(
                        Element::row()
                            .width_match()
                            .cross(Align::Center)
                            .child(chip_label(text).weight(1.0))
                            .child(
                                Element::link(strings::FILE_CANCEL)
                                    .on_click(move |_| ui.act(|m| m.cancel_transfer(&hash))),
                            ),
                    ),
            );
            None
        }
        BlobState::Local => None,
    };
    match (left_chip, duration) {
        (Some(chip), Some(d)) if w >= SIDE_BY_SIDE_MIN_W => {
            layer = layer.child(
                Element::row()
                    .width_match()
                    .padding(style::SPACE_SM)
                    .spacing(style::SPACE_SM)
                    .cross(Align::Center)
                    .align_xy(Align::Start, Align::End)
                    .child(chip)
                    .child(Element::flex_spacer())
                    .child(d),
            );
        }
        (chip, d) => {
            if let Some(chip) = chip {
                layer = layer.child(note(chip));
            }
            if let Some(d) = d {
                // 下载中底部被进度条占用、窄缩略图放不下并排：时长放右上
                let busy = state != BlobState::Local;
                let pos = if busy { Align::Start } else { Align::End };
                layer = layer.child(d.align_xy(Align::End, pos).margin(style::SPACE_SM));
            }
        }
    }
    layer
}

/// 单张图片 / 视频：缩略图 + 状态标注；点击打开原文件或开始下载。
fn media_thumb(ui: &UiState, item: &FileItemView, video: bool) -> Element {
    let (w, h) = thumb_size(item.width, item.height);
    let radius = style::RADIUS_CARD;
    let picture = match &item.thumb_path {
        Some(p) => Element::image(p).fit(Fit::Cover).size(w, h).corner(radius),
        None => Element::stack()
            .size(w, h)
            .corner(radius)
            .bg_role(Role::Surface)
            .child(
                style::icon(
                    icon_bytes(file_icon(&item.mime, &item.name)),
                    32,
                    Role::TextSubtle,
                )
                .align(Align::Center),
            ),
    };
    let mut stack = Element::stack()
        .size(w, h)
        .corner(radius)
        .border_role(Role::Border, 1)
        .clickable()
        .child(picture);
    if video {
        stack = stack.child(
            Element::stack()
                .size(44, 44)
                .corner(22.0)
                .bg_role(Role::Scrim)
                .align(Align::Center)
                .child(style::icon(style::ICON_PLAY, 24, Role::OnAccent).align(Align::Center)),
        );
    }
    // 状态标注随派生状态重建；铺满缩略图，内部再按角落定位
    let key = {
        let it = item.clone();
        ui.derive(move |m| state_key(&effective_state(&it, m)))
    };
    let (ui_o, it_o) = (ui.clone(), item.clone());
    stack = stack.child(
        Element::switcher(key, move |st| {
            thumb_overlay(&ui_o, &it_o, video, w, state_from_key(st))
        })
        .fill(),
    );
    let (state, hash, local) = (
        item.state.clone(),
        item.hash.clone(),
        item.local_path.clone(),
    );
    let ui = ui.clone();
    stack.on_click(move |ctx| match (&state, &local) {
        (BlobState::Local, Some(p)) => ctx.open_url(p),
        (BlobState::Remote, _) => ui.act(|m| m.download(hash.clone())),
        _ => {}
    })
}

fn is_media(item: &FileItemView) -> Option<bool> {
    match file_icon(&item.mime, &item.name) {
        FileIcon::Image => Some(false),
        FileIcon::Video => Some(true),
        _ => None,
    }
}

/// 是否为单张图片 / 视频缩略图消息（用于决定与相邻气泡的间距）。
pub fn is_thumb_message(m: &MessageView) -> bool {
    matches!(&m.body, MessageBodyView::Files { items } if matches!(items.as_slice(), [one] if is_media(one).is_some()))
}

fn files_body(ui: &UiState, items: &[FileItemView]) -> Element {
    if let [one] = items {
        if let Some(video) = is_media(one) {
            return media_thumb(ui, one, video);
        }
    }
    Element::col()
        .spacing(style::SPACE_XS)
        .children(items.iter().map(|i| single_file_card(ui, i)))
}

/// 一条消息的完整一行（不含与上一条的间距、时间分隔）。
/// `show_author`：在气泡上方标注来源设备名（“我的设备”会话里作者都是自己，靠它区分）。
pub fn message_row(ui: &UiState, m: &MessageView, show_author: bool) -> Element {
    if m.state == MessageState::Recalled {
        return notice(strings::MSG_RECALLED);
    }
    let mine = m.is_mine;
    let body = match &m.body {
        MessageBodyView::System { text } => return notice(text.clone()),
        MessageBodyView::Unsupported => return notice(strings::MSG_UNSUPPORTED),
        MessageBodyView::Text { text } => text_bubble(mine, text),
        MessageBodyView::Files { items } => files_body(ui, items),
        MessageBodyView::Folder { name, items } => folder_card(ui, name, items),
    }
    .on_context_menu(menu_builder(ui, m));

    let side = if mine { Align::End } else { Align::Start };
    let mut col = Element::col().cross(side).spacing(style::SPACE_XS);
    if show_author {
        col = col.child(
            Element::label(m.author_device_name.clone())
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted),
        );
    }
    col = col.child(body);
    if mine {
        match m.state {
            MessageState::Sending => {
                col = col.child(
                    Element::label(strings::MSG_SENDING)
                        .font_size(style::FONT_META)
                        .fg_role(Role::TextSubtle),
                );
            }
            MessageState::Failed => {
                col = col.child(
                    Element::label(strings::MSG_FAILED)
                        .font_size(style::FONT_META)
                        .fg_role(Role::Danger),
                );
            }
            _ => {}
        }
    }
    let mut row = Element::row().width_match();
    if mine {
        row = row.child(Element::flex_spacer()).child(col);
    } else {
        row = row.child(col).child(Element::flex_spacer());
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn mv(mine: bool, ago_ms: i64, state: MessageState) -> MessageView {
        MessageView {
            id: "m".into(),
            conv: "c".into(),
            author_user: "u".into(),
            author_device: "d".into(),
            author_device_name: "dev".into(),
            is_mine: mine,
            hlc_millis: 1_000_000 - ago_ms,
            body: MessageBodyView::Text { text: "x".into() },
            state,
        }
    }

    #[test]
    fn human_size_cases() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(12_900_000), "12.3 MB");
        assert_eq!(human_size(1_610_612_736), "1.50 GB");
        assert_eq!(human_speed(3_250_000), "3.1 MB/s");
    }

    #[test]
    fn duration_label_cases() {
        assert_eq!(duration_label(7_000), "0:07");
        assert_eq!(duration_label(754_000), "12:34");
        assert_eq!(duration_label(3_723_000), "1:02:03");
    }

    #[test]
    fn can_recall_only_own_and_within_window() {
        let now = 1_000_000;
        assert!(!can_recall(&mv(false, 1_000, MessageState::Sent), now));
        assert!(can_recall(&mv(true, 60_000, MessageState::Sent), now));
        assert!(!can_recall(&mv(true, 180_000, MessageState::Sent), now));
        assert!(!can_recall(&mv(true, 1_000, MessageState::Recalled), now));
    }

    #[test]
    fn file_icon_by_mime_and_ext() {
        assert_eq!(file_icon("application/pdf", "a"), FileIcon::Pdf);
        assert_eq!(
            file_icon("application/octet-stream", "a.xlsx"),
            FileIcon::Sheet
        );
        assert_eq!(
            file_icon("application/octet-stream", "x.7z"),
            FileIcon::Archive
        );
        assert_eq!(file_icon("image/png", "x"), FileIcon::Image);
        assert_eq!(file_icon("video/mp4", "x"), FileIcon::Video);
        assert_eq!(
            file_icon("application/vnd.openxmlformats", "纪要.docx"),
            FileIcon::Doc
        );
        assert_eq!(
            file_icon("application/x-unknown", "noext"),
            FileIcon::Generic
        );
    }

    #[test]
    fn thumb_size_fits_240_box_keeping_aspect() {
        assert_eq!(thumb_size(Some(1920), Some(1080)), (240, 135));
        assert_eq!(thumb_size(Some(1000), Some(2000)), (120, 240));
        assert_eq!(thumb_size(Some(100), Some(80)), (100, 80));
        assert_eq!(thumb_size(None, None), (240, 160));
        // 极端长图不会缩到看不见
        assert_eq!(thumb_size(Some(10_000), Some(100)).1, 48);
    }

    fn item(hash: &str, size: u64, state: BlobState) -> FileItemView {
        FileItemView {
            hash: hash.into(),
            name: hash.into(),
            size,
            mime: String::new(),
            width: None,
            height: None,
            duration_ms: None,
            thumb_path: None,
            local_path: None,
            state,
            source_label: String::new(),
        }
    }

    #[test]
    fn aggregate_state_priority() {
        let l = |s| item("a", 1, s);
        assert_eq!(aggregate_state(&[l(BlobState::Local)]), BlobState::Local);
        assert_eq!(
            aggregate_state(&[l(BlobState::Local), l(BlobState::Remote)]),
            BlobState::Remote
        );
        assert_eq!(
            aggregate_state(&[l(BlobState::Remote), l(BlobState::Downloading)]),
            BlobState::Downloading
        );
        assert_eq!(
            aggregate_state(&[l(BlobState::Local), l(BlobState::SourceOffline)]),
            BlobState::SourceOffline
        );
        assert_eq!(
            aggregate_state(&[l(BlobState::Preparing), l(BlobState::Downloading)]),
            BlobState::Preparing
        );
    }

    #[test]
    fn aggregate_progress_sums_present_transfers() {
        let mut t = BTreeMap::new();
        t.insert(
            "a".to_string(),
            crate::model::TransferInfo {
                hash: "a".into(),
                done: 50,
                total: 100,
                bytes_per_sec: 10,
            },
        );
        t.insert(
            "b".to_string(),
            crate::model::TransferInfo {
                hash: "b".into(),
                done: 150,
                total: 300,
                bytes_per_sec: 5,
            },
        );
        let h = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let (f, sp) = aggregate_progress(&t, &h(&["a", "b", "c"]));
        assert!((f - 0.5).abs() < 1e-6);
        assert_eq!(sp, 15);
        assert_eq!(aggregate_progress(&t, &h(&["c"])), (0.0, 0));
    }

    #[test]
    fn parent_dir_cases() {
        assert_eq!(parent_dir("/tmp/a/b.txt").as_deref(), Some("/tmp/a"));
        assert_eq!(parent_dir("b.txt"), None);
    }
}
