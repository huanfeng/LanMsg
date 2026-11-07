// 文件页：筛选条（类型 / 会话 / 状态 / 名字）+ 列表或网格。
// 类型为图片或视频时用网格（120×120 缩略图 + 文件名与来源），其余用列表；
// 列表每项两行信息（名字 / 大小·时间·来源·会话）+ 右侧统一的小号描边操作按钮。
// 注释中文，标识符英文。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use lanmsg_api::types::{BlobState, ConversationSummary, FileFilter, FileKindFilter, FileView};
use windui::event::MenuItem;
use windui::prelude::*;

use crate::debounce::Debouncer;
use crate::event_text::conv_title;
use crate::model::{effective_state, Model, Page};
use crate::strings;
use crate::style;
use crate::tz;
use crate::ui_state::UiState;
use crate::views::chats::{format_time, now_millis};
use crate::views::confirm::{confirm_host, Confirm, ConfirmState};
use crate::views::forward::forward_host;
use crate::views::message_row::{
    aggregate_progress, duration_label, file_icon, human_size, icon_bytes, parent_dir,
    progress_text, FileIcon,
};
use crate::views::preview::open_local;

/// 内容区最大宽度之外的两侧留白。
const PAGE_PAD: i32 = style::SPACE_XL;
/// 网格每行的缩略图个数：5 × 120 + 4 × 12 = 648，正好放进最小窗口（760）的内容区。
/// windui 取不到容器宽度，列数无法随窗口宽度变化（缺能力），所以固定列数并在行内居中。
const GRID_COLS: usize = 5;
/// 网格缩略图边长。
const TILE: i32 = 120;
/// 列表行高度（名字一行 + 信息最多两行）。
const ROW_H: i32 = 76;
/// 列表行右侧操作区宽度。
const ACTIONS_W: i32 = 208;
/// 搜索框停止输入多久后查询（毫秒）。
const SEARCH_DEBOUNCE_MS: u64 = 300;

/// 类型筛选的段（顺序即界面顺序）。
pub const KINDS: [(FileKindFilter, &str); 6] = [
    (FileKindFilter::All, strings::FILES_KIND_ALL),
    (FileKindFilter::Image, strings::FILES_KIND_IMAGE),
    (FileKindFilter::Video, strings::FILES_KIND_VIDEO),
    (FileKindFilter::Document, strings::FILES_KIND_DOCUMENT),
    (FileKindFilter::Archive, strings::FILES_KIND_ARCHIVE),
    (FileKindFilter::Other, strings::FILES_KIND_OTHER),
];

/// 状态筛选的选项。
pub const STATES: [(Option<BlobState>, &str); 4] = [
    (None, strings::FILES_STATE_ALL),
    (Some(BlobState::Local), strings::FILES_STATE_LOCAL),
    (Some(BlobState::Remote), strings::FILES_STATE_REMOTE),
    (Some(BlobState::SourceOffline), strings::FILES_STATE_OFFLINE),
];

// ---------------------------------------------------------------------------
// 纯函数
// ---------------------------------------------------------------------------

/// 会话下拉的选项：第一项“全部会话”，其余按标题（再按 id）排序——顺序不随新消息变化，
/// 下拉的选中项才不会漂移。
pub fn conv_options(convs: &[ConversationSummary]) -> Vec<(Option<String>, String)> {
    let mut rest: Vec<(Option<String>, String)> = convs
        .iter()
        .map(|c| (Some(c.conv.clone()), conv_title(c)))
        .collect();
    rest.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let mut out = vec![(None, strings::FILES_ALL_CONVS.to_string())];
    out.extend(rest);
    out
}

/// 当前会话筛选在选项里的位置；找不到（会话已不在）回到“全部会话”。
pub fn conv_index(options: &[(Option<String>, String)], conv: &Option<String>) -> usize {
    options.iter().position(|(id, _)| id == conv).unwrap_or(0)
}

/// 类型在 `KINDS` 里的位置。
pub fn kind_index(kind: &FileKindFilter) -> usize {
    KINDS.iter().position(|(k, _)| k == kind).unwrap_or(0)
}

/// 状态在 `STATES` 里的位置。
pub fn state_index(state: &Option<BlobState>) -> usize {
    STATES.iter().position(|(s, _)| s == state).unwrap_or(0)
}

/// 图片与视频用网格，其余用列表。
pub fn is_grid(kind: &FileKindFilter) -> bool {
    matches!(kind, FileKindFilter::Image | FileKindFilter::Video)
}

/// 把“来自 张三 · 存于 我的台式机”拆成独立的几段：界面上各段分开摆放，不用中点拼接。
/// 依赖 core 返回的 `source_label` 以 `" · "`（中点两侧各一个空格）分段；core 改了分隔符这里要同步。
pub fn split_source(label: &str) -> Vec<String> {
    label
        .split(" · ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// 列表里的一行：列表模式一项一行，网格模式 `GRID_COLS` 项一行。
#[derive(Debug, Clone, PartialEq)]
pub struct FileRow {
    pub key: String,
    pub files: Vec<FileView>,
    pub grid: bool,
}

/// 每个文件项的唯一键：消息 + 哈希；同一条消息里内容相同的文件再加序号。
fn item_keys(files: &[FileView]) -> Vec<String> {
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    files
        .iter()
        .map(|f| {
            let n = seen
                .entry((f.msg.clone(), f.item.hash.clone()))
                .and_modify(|n| *n += 1)
                .or_insert(0);
            format!("{}:{}:{}", f.msg, f.item.hash, n)
        })
        .collect()
}

/// 文件列表 -> 视图行。网格行的键取第一项的键：追加下一页时前面的行原样保留，滚动位置不动。
pub fn build_rows(files: &[FileView], grid: bool) -> Vec<FileRow> {
    let keys = item_keys(files);
    let per_row = if grid { GRID_COLS } else { 1 };
    files
        .chunks(per_row)
        .zip(keys.chunks(per_row))
        .map(|(fs, ks)| FileRow {
            key: ks[0].clone(),
            files: fs.to_vec(),
            grid,
        })
        .collect()
}

/// 筛选条件里是否有任何一项在生效（决定空状态的措辞）。
pub fn is_filtered(f: &FileFilter) -> bool {
    f.kind != FileKindFilter::All || f.conv.is_some() || f.state.is_some() || f.name_query.is_some()
}

/// 内容区的状态：0 加载中、1 没有文件、2 有文件。
fn content_state(m: &Model) -> u8 {
    if !m.files_loaded && m.files.is_empty() {
        0
    } else if m.files.is_empty() {
        1
    } else {
        2
    }
}

// ---------------------------------------------------------------------------
// 筛选条状态
// ---------------------------------------------------------------------------

/// 筛选控件的状态：控件绑定的信号 + 上次已同步给 Model 的选择。
/// windui 的分段控件与下拉没有“变更”回调，由 `tick` 周期性比对信号与 Model。
#[derive(Clone)]
pub struct FilesUi {
    pub kind: Signal<usize>,
    pub conv: Signal<usize>,
    pub state: Signal<usize>,
    pub text: Signal<String>,
    seen: Rc<Cell<(usize, usize, usize)>>,
    debouncer: Rc<RefCell<Debouncer>>,
}

impl FilesUi {
    /// 控件初值取自 Model 里的筛选条件（演示页与重建界面时保持一致）。
    pub fn new(filter: &FileFilter, convs: &[ConversationSummary]) -> FilesUi {
        let kind = kind_index(&filter.kind);
        let state = state_index(&filter.state);
        let conv = conv_index(&conv_options(convs), &filter.conv);
        let text = filter.name_query.clone().unwrap_or_default();
        let mut d = Debouncer::new(SEARCH_DEBOUNCE_MS);
        d.reset_to(&text, 0);
        FilesUi {
            kind: signal(kind),
            conv: signal(conv),
            state: signal(state),
            text: signal(text),
            seen: Rc::new(Cell::new((kind, conv, state))),
            debouncer: Rc::new(RefCell::new(d)),
        }
    }

    /// 周期调用：把控件上的变化同步给 Model（名字搜索停止输入 300ms 后才查询）。
    pub fn tick(&self, ui: &UiState, now_ms: u64) {
        let (k, c, s) = (self.kind.get(), self.conv.get(), self.state.get());
        let seen = self.seen.get();
        if k != seen.0 {
            if let Some((kind, _)) = KINDS.get(k) {
                let kind = kind.clone();
                ui.act(|m| m.set_file_kind(kind));
            }
        }
        if s != seen.2 {
            if let Some((state, _)) = STATES.get(s) {
                let state = state.clone();
                ui.act(|m| m.set_file_state(state));
            }
        }
        let options = ui.read(|m| conv_options(&m.convs));
        let mut conv_now = c;
        if c != seen.1 {
            let id = options.get(c).and_then(|o| o.0.clone());
            ui.act(|m| m.set_file_conv(id));
        } else {
            // 会话列表变动后，让下拉的位置继续指向 Model 里选中的那个会话
            let want = ui.read(|m| conv_index(&options, &m.file_filter.conv));
            if want != c {
                self.conv.set(want);
                conv_now = want;
            }
        }
        self.seen.set((k, conv_now, s));
        let fired = self.debouncer.borrow_mut().feed(&self.text.get(), now_ms);
        if let Some(q) = fired {
            ui.act(|m| m.set_file_name(q));
        }
    }
}

// ---------------------------------------------------------------------------
// 视图
// ---------------------------------------------------------------------------

/// switcher 的键要 Eq + Hash，而 `BlobState` 没有：用数字码表示。
fn state_code(s: &BlobState) -> u8 {
    match s {
        BlobState::Remote => 0,
        BlobState::Downloading => 1,
        BlobState::Local => 2,
        BlobState::SourceOffline => 3,
        BlobState::Preparing => 4,
    }
}

/// 列表项第二行的信息段：大小、（视频）时长、时间、“来自 X”；
/// 会话名已在来源里出现（如私聊“来自 张三”）时不再重复。
/// 来源里其余的段（“存于 X”、状态）归右侧操作区，见 `status_parts`。
pub fn meta_parts(f: &FileView, time_text: String) -> Vec<String> {
    let item = &f.item;
    let mut parts = vec![human_size(item.size)];
    if file_icon(&item.mime, &item.name) == FileIcon::Video {
        if let Some(ms) = item.duration_ms {
            parts.push(duration_label(ms));
        }
    }
    parts.push(time_text);
    parts.extend(split_source(&item.source_label).into_iter().take(1));
    if !f.conv_title.is_empty() && !item.source_label.contains(&f.conv_title) {
        parts.push(strings::files_in_conv(&f.conv_title));
    }
    parts
}

/// 放在右侧操作区按钮下方的说明：来源里“来自 X”之后的各段（“存于 X”、“已下载”）。
pub fn status_parts(f: &FileView) -> String {
    split_source(&f.item.source_label)
        .into_iter()
        .skip(1)
        .collect::<Vec<_>>()
        .join("  ")
}

fn meta_label(text: impl Into<TextContent>, role: Role) -> Element {
    Element::label(text)
        .font_size(style::FONT_META)
        .fg_role(role)
        .max_lines(1)
        .truncate(Truncate::End)
}

/// 右键菜单：打开 / 打开所在文件夹 / 下载、转发、定位到会话、删除本地副本。
fn menu_builder(
    ui: &UiState,
    f: &FileView,
    confirm: ConfirmState,
) -> impl Fn() -> Vec<MenuItem> + 'static {
    let ui = ui.clone();
    let f = f.clone();
    move || {
        let mut v = Vec::new();
        let item = &f.item;
        // 点击下载后立刻按“下载中”处理，菜单里不再给“下载”
        let state = ui.read(|m| effective_state(item, m));
        if state == BlobState::Local {
            if let Some(p) = item.local_path.clone() {
                if let Some(d) = parent_dir(&p) {
                    let (ui_open, mime, name) = (ui.clone(), item.mime.clone(), item.name.clone());
                    v.push(MenuItem::run(
                        strings::FILES_MENU_OPEN,
                        move |ctx| open_local(ctx, &ui_open, &mime, &name, &p),
                        false,
                    ));
                    v.push(MenuItem::run(
                        strings::FILES_MENU_OPEN_FOLDER,
                        move |ctx| ctx.open_url(&d),
                        false,
                    ));
                }
            }
        }
        if state == BlobState::Remote {
            let (ui, hash) = (ui.clone(), item.hash.clone());
            v.push(MenuItem::run(
                strings::FILES_MENU_DOWNLOAD,
                move |_| ui.act(|m| m.download(hash.clone())),
                false,
            ));
        }
        if !v.is_empty() {
            v.push(MenuItem::separator());
        }
        let (ui_fwd, hash) = (ui.clone(), item.hash.clone());
        v.push(MenuItem::run(
            strings::FILES_MENU_FORWARD,
            move |_| ui_fwd.open_forward(vec![hash.clone()]),
            false,
        ));
        let (ui_loc, conv) = (ui.clone(), f.conv.clone());
        v.push(MenuItem::run(
            strings::FILES_MENU_LOCATE,
            move |_| {
                ui_loc.act(|m| {
                    let mut fx = m.set_page(Page::Chats);
                    fx.extend(m.open_conv(conv.clone()));
                    fx
                })
            },
            false,
        ));
        v.push(MenuItem::separator());
        // 删除的是整条消息；消息里不止一个文件时在确认框里说明
        let siblings = ui.read(|m| m.files.iter().filter(|x| x.msg == f.msg).count());
        let mut message = strings::FILES_DELETE_MESSAGE.to_string();
        if siblings > 1 {
            message = format!("{message}{}", strings::files_delete_siblings(siblings));
        }
        let msg_id = f.msg.clone();
        v.push(
            MenuItem::run(
                strings::FILES_MENU_DELETE,
                move |_| {
                    confirm.open(Confirm {
                        id: msg_id.clone(),
                        title: strings::FILES_DELETE_TITLE,
                        message: message.clone(),
                        action: strings::FILES_DELETE_ACTION,
                    })
                },
                false,
            )
            .danger(),
        );
        v
    }
}

/// 缩略图（或类型图标）：`size` 见方。
fn thumb_or_icon(item: &lanmsg_api::types::FileItemView, size: i32, radius: f32) -> Element {
    match &item.thumb_path {
        Some(p) => Element::image(p)
            .fit(Fit::Cover)
            .size(size, size)
            .corner(radius),
        None => Element::stack()
            .size(size, size)
            .corner(radius)
            .bg_when(|| style::tokens().signal_soft)
            .child(
                style::icon(
                    icon_bytes(file_icon(&item.mime, &item.name)),
                    size * 3 / 5,
                    Role::AccentText,
                )
                .align(Align::Center),
            ),
    }
}

/// 列表项右侧的操作区：待下载 -> 下载；下载中 -> 进度 + 取消；已在本地 -> 打开 / 打开所在文件夹；
/// 源离线、准备中 -> 文字。按钮一律是小号描边按钮。
fn file_actions(ui: &UiState, item: &lanmsg_api::types::FileItemView, status: String) -> Element {
    let key = {
        let it = item.clone();
        ui.derive(move |m| state_code(&effective_state(&it, m)))
    };
    let (ui, item) = (ui.clone(), item.clone());
    Element::switcher(key, move |st| {
        let button = |text: &'static str| Element::button(text).outline().small();
        let main = match st {
            0 => {
                let (ui, hash) = (ui.clone(), item.hash.clone());
                button(strings::FILE_DOWNLOAD)
                    .on_click(move |_| ui.act(|m| m.download(hash.clone())))
            }
            1 => {
                let hs = vec![item.hash.clone()];
                let frac = ui.derive({
                    let hs = hs.clone();
                    move |m| aggregate_progress(&m.transfers, &hs).0
                });
                let text = ui.derive({
                    let hs = hs.clone();
                    move |m| progress_text(&m.transfers, &hs)
                });
                let (ui, hash) = (ui.clone(), item.hash.clone());
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
                            .child(
                                button(strings::FILE_CANCEL)
                                    .on_click(move |_| ui.act(|m| m.cancel_transfer(&hash))),
                            ),
                    )
            }
            2 => {
                let mut row = Element::row().spacing(style::SPACE_SM).cross(Align::Center);
                if let Some(p) = item.local_path.clone() {
                    if let Some(d) = parent_dir(&p) {
                        let (ui, mime, name) = (ui.clone(), item.mime.clone(), item.name.clone());
                        row = row
                            .child(
                                button(strings::FILE_OPEN)
                                    .on_click(move |ctx| open_local(ctx, &ui, &mime, &name, &p)),
                            )
                            .child(
                                button(strings::FILE_OPEN_FOLDER)
                                    .on_click(move |ctx| ctx.open_url(&d)),
                            );
                    }
                }
                row
            }
            3 => meta_label(strings::FILE_SOURCE_OFFLINE, Role::TextMuted),
            _ => meta_label(strings::FILE_PREPARING, Role::TextMuted),
        };
        // 下载中的进度文字已说明状态；其余状态在按钮下方补一行说明。一律右对齐
        let mut col = Element::col()
            .width_match()
            .cross(if st == 1 { Align::Start } else { Align::End })
            .spacing(style::SPACE_XS)
            .child(main);
        if st != 1 && !status.is_empty() {
            col = col.child(meta_label(status.clone(), Role::TextSubtle));
        }
        col
    })
}

/// 列表行：缩略图（视频带播放角标）、名字 + 信息行，右侧操作区。
fn list_row(ui: &UiState, f: &FileView, confirm: ConfirmState) -> Element {
    let item = &f.item;
    let time = format_time(f.hlc_millis, now_millis(), tz::local_offset_secs());
    // 信息行用流式布局：窄窗口时自动折到第二行，各段之间只留间距不加符号
    let mut meta = Element::flow()
        .width_match()
        .spacing(style::SPACE_MD)
        .run_spacing(style::SPACE_XS);
    for (i, part) in meta_parts(f, time).into_iter().enumerate() {
        let role = if i == 0 {
            Role::TextMuted
        } else {
            Role::TextSubtle
        };
        meta = meta.child(meta_label(part, role));
    }
    let info = Element::col()
        .weight(1.0)
        .spacing(style::SPACE_XS)
        .child(
            Element::label(item.name.clone())
                .font_size(style::FONT_BODY)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End),
        )
        .child(meta);
    let mut thumb =
        Element::stack()
            .size(40, 40)
            .child(thumb_or_icon(item, 40, style::RADIUS_CONTROL));
    if item.thumb_path.is_some() && file_icon(&item.mime, &item.name) == FileIcon::Video {
        thumb = thumb.child(
            Element::stack()
                .size(20, 20)
                .corner(10.0)
                .bg_role(Role::Scrim)
                .align(Align::Center)
                .child(style::icon(style::ICON_PLAY, 12, Role::OnAccent).align(Align::Center)),
        );
    }
    Element::col()
        .width_match()
        .child(
            Element::row()
                .width_match()
                .height(ROW_H)
                // 右侧多留一截：滚动条槽不压住操作区
                .padding_edges(Insets {
                    top: style::SPACE_SM,
                    bottom: style::SPACE_SM,
                    left: style::SPACE_LG,
                    right: style::SPACE_XL,
                })
                .spacing(style::SPACE_MD)
                .cross(Align::Center)
                .child(thumb)
                .child(info)
                .child(Element::col().width(ACTIONS_W).child(file_actions(
                    ui,
                    item,
                    status_parts(f),
                ))),
        )
        .child(Element::divider())
        .on_context_menu(menu_builder(ui, f, confirm))
}

/// 网格缩略图上的状态标注：待下载 / 源离线 / 准备中是左下角小标签，下载中是底部进度条。
fn tile_overlay(ui: &UiState, item: &lanmsg_api::types::FileItemView) -> Element {
    let key = {
        let it = item.clone();
        ui.derive(move |m| state_code(&effective_state(&it, m)))
    };
    let (ui, item) = (ui.clone(), item.clone());
    Element::switcher(key, move |st| {
        let chip = |child: Element| {
            Element::row()
                .padding_xy(style::SPACE_SM, style::SPACE_XS)
                .corner(style::SPACE_SM as f32)
                .bg_role(Role::Scrim)
                .cross(Align::Center)
                .spacing(style::SPACE_XS)
                .child(child)
                .align_xy(Align::Start, Align::End)
                .margin(style::SPACE_SM)
        };
        let chip_text = |t: String| {
            Element::label(t)
                .font_size(style::FONT_META)
                .fg_role(Role::OnAccent)
        };
        match st {
            0 => chip(
                Element::row()
                    .spacing(style::SPACE_XS)
                    .cross(Align::Center)
                    .child(style::icon(style::ICON_DOWNLOAD, 16, Role::OnAccent))
                    .child(chip_text(human_size(item.size))),
            ),
            1 => {
                let hs = vec![item.hash.clone()];
                let frac = ui.derive(move |m| aggregate_progress(&m.transfers, &hs).0);
                Element::col()
                    .width_match()
                    .padding(style::SPACE_SM)
                    .bg_role(Role::Scrim)
                    .align_xy(Align::Start, Align::End)
                    .child(Element::progress(frac).width_match())
            }
            3 => chip(chip_text(strings::FILE_SOURCE_OFFLINE.to_string())),
            4 => chip(chip_text(strings::FILE_PREPARING.to_string())),
            _ => Element::leaf().size(0, 0),
        }
    })
    .fill()
}

/// 网格里的一格：120×120 缩略图（带状态标注）+ 下方文件名；点击打开或下载。
fn grid_tile(ui: &UiState, f: &FileView, confirm: ConfirmState) -> Element {
    let item = &f.item;
    let mut thumb = Element::stack()
        .size(TILE, TILE)
        .corner(style::RADIUS_CARD)
        .border_role(Role::Border, 1)
        .clickable()
        .child(thumb_or_icon(item, TILE, style::RADIUS_CARD));
    if file_icon(&item.mime, &item.name) == FileIcon::Video {
        let mut play = Element::stack()
            .size(36, 36)
            .corner(18.0)
            .bg_role(Role::Scrim)
            .align(Align::Center)
            .child(style::icon(style::ICON_PLAY, 20, Role::OnAccent).align(Align::Center));
        if item.thumb_path.is_none() {
            play = play.visible(false);
        }
        thumb = thumb.child(play);
    }
    thumb = thumb.child(tile_overlay(ui, item));
    if let (Some(ms), true) = (
        item.duration_ms,
        file_icon(&item.mime, &item.name) == FileIcon::Video,
    ) {
        thumb = thumb.child(
            Element::row()
                .padding_xy(style::SPACE_SM, 2)
                .corner(style::SPACE_SM as f32)
                .bg_role(Role::Scrim)
                .align_xy(Align::End, Align::Start)
                .margin(style::SPACE_SM)
                .child(
                    Element::label(duration_label(ms))
                        .font_size(style::FONT_META)
                        .fg_role(Role::OnAccent),
                ),
        );
    }
    let (ui_click, hash, local) = (ui.clone(), item.hash.clone(), item.local_path.clone());
    let state = item.state.clone();
    let (ui_open, mime, name) = (ui.clone(), item.mime.clone(), item.name.clone());
    thumb = thumb.on_click(move |ctx| match (&state, &local) {
        (BlobState::Local, Some(p)) => open_local(ctx, &ui_open, &mime, &name, p),
        (BlobState::Remote, _) => ui_click.act(|m| m.download(hash.clone())),
        _ => {}
    });
    Element::col()
        .width(TILE)
        .spacing(style::SPACE_XS)
        .child(thumb)
        .child(
            Element::label(item.name.clone())
                .font_size(style::FONT_META)
                .fg_role(Role::Text)
                .max_lines(1)
                .truncate(Truncate::End)
                .width(TILE),
        )
        .child(meta_label(tile_source(&item.source_label), Role::TextSubtle).width(TILE))
        .on_context_menu(menu_builder(ui, f, confirm))
}

/// 磁贴下方的来源：只取第一段（“来自 张三”），磁贴窄放不下全部。
pub fn tile_source(label: &str) -> String {
    split_source(label).into_iter().next().unwrap_or_default()
}

/// 网格一行：windui 取不到容器宽度，无法按宽度算列数，所以整行在内容区里居中。
fn grid_row(ui: &UiState, fs: &[FileView], confirm: ConfirmState) -> Element {
    Element::col().width_match().cross(Align::Center).child(
        Element::row()
            .spacing(style::SPACE_MD)
            .padding_xy(style::SPACE_LG, style::SPACE_SM)
            .children(fs.iter().map(|f| grid_tile(ui, f, confirm)))
            // 末行不足时用空位补齐，缺的磁贴不会让这一行相对上面的行错位
            .children((fs.len()..GRID_COLS).map(|_| Element::leaf().size(TILE, 1))),
    )
}

fn row_view(ui: &UiState, row: FileRow, confirm: ConfirmState) -> Element {
    match (row.grid, row.files.first()) {
        (false, Some(f)) => list_row(ui, f, confirm),
        (true, _) => grid_row(ui, &row.files, confirm),
        (false, None) => Element::leaf().size(0, 0),
    }
}

/// 筛选条：第一行类型分段控件，第二行会话 / 状态下拉 + 名字搜索（窄窗口时自动换行）。
fn filter_bar(ui: &UiState) -> Element {
    let f = ui.files_ui.clone();
    let conv_titles = ui.derive(|m| conv_options(&m.convs).into_iter().map(|o| o.1).collect());
    let kinds = Element::segmented(KINDS.iter().map(|k| k.1).collect(), f.kind);
    let others = Element::flow()
        .width_match()
        .spacing(style::SPACE_MD)
        .run_spacing(style::SPACE_SM)
        .child(
            Element::dropdown_signal(conv_titles, f.conv)
                .width(176)
                .height(style::CONTROL_H),
        )
        .child(
            Element::dropdown(STATES.iter().map(|s| s.1).collect(), f.state)
                .width(136)
                .height(style::CONTROL_H),
        )
        .child(
            Element::text_input(f.text, strings::FILES_SEARCH_PLACEHOLDER)
                .width(240)
                .height(style::CONTROL_H),
        );
    Element::col()
        .width_match()
        .spacing(style::SPACE_SM)
        .child(kinds)
        .child(others)
}

/// 内容区：随“加载中 / 空 / 有文件”以及视图序号（改筛选回到顶部）重建。
fn content(ui: &UiState, confirm: ConfirmState) -> Element {
    let key = ui.derive(|m| {
        (
            m.files_view_epoch,
            content_state(m),
            is_filtered(&m.file_filter),
        )
    });
    let ui = ui.clone();
    Element::switcher(key, move |(_, state, filtered)| match state {
        0 => style::empty_state(strings::FILES_LOADING, "", None),
        1 if filtered => style::empty_state(
            strings::FILES_EMPTY_FILTERED,
            strings::FILES_EMPTY_FILTERED_HINT,
            None,
        ),
        1 => style::empty_state(strings::FILES_EMPTY, strings::FILES_EMPTY_HINT, None),
        _ => {
            let rows = ui.derive(|m| build_rows(&m.files, is_grid(&m.file_filter.kind)));
            let ui_row = ui.clone();
            let ui_edge = ui.clone();
            Element::list_signal(
                rows,
                |r: &FileRow| r.key.clone(),
                move |r| row_view(&ui_row, r, confirm),
            )
            // 滚到底且上次是满页时追加下一页；Model 内部防重复
            .on_scroll_edge(move |_ctx, edge| {
                if matches!(edge, ScrollEdge::Bottom) {
                    ui_edge.act(|m| m.load_more_files());
                }
            })
            .bg_role(Role::Surface)
            .border_role(Role::Border, 1)
            .corner(style::RADIUS_CARD)
            // 上下留白：网格的第一排与最后一排不贴着卡片边
            .padding_xy(0, style::SPACE_SM)
            .width_match()
            .weight(1.0)
        }
    })
    .fill()
}

/// 文件页。
pub fn files_page(ui: &UiState) -> Element {
    let confirm = ConfirmState::new();
    let ui_ok = ui.clone();
    let loading_more = ui.derive(|m| m.files_loading);
    let body = Element::col()
        .fill()
        .padding_edges(Insets {
            top: style::SPACE_XL,
            bottom: style::SPACE_LG,
            left: PAGE_PAD,
            right: PAGE_PAD,
        })
        .spacing(style::SPACE_LG)
        .child(
            Element::label(strings::PAGE_FILES)
                .font_size(style::FONT_TITLE)
                .font_weight(style::WEIGHT_STRONG)
                .fg_role(Role::Text),
        )
        .child(filter_bar(ui))
        .child(content(ui, confirm).weight(1.0))
        .child(
            Element::label(strings::FILES_LOADING_MORE)
                .font_size(style::FONT_META)
                .fg_role(Role::TextMuted)
                .visible_signal(loading_more),
        );
    Element::stack()
        .fill()
        .bg_role(Role::Bg)
        .child(body)
        .child(confirm_host(confirm, move |msg| {
            ui_ok.act(|m| m.delete_local(msg));
        }))
        .child(forward_host(ui))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanmsg_api::types::{ConvKind, FileItemView};

    fn conv(id: &str, title: &str) -> ConversationSummary {
        ConversationSummary {
            conv: id.into(),
            kind: ConvKind::Dm,
            title: title.into(),
            last_text: None,
            last_kind: None,
            last_hlc_millis: None,
            unread: 0,
            peer_online: false,
        }
    }

    fn fv(msg: &str, hash: &str) -> FileView {
        FileView {
            item: FileItemView {
                hash: hash.into(),
                name: format!("{hash}.png"),
                size: 1,
                mime: "image/png".into(),
                width: None,
                height: None,
                duration_ms: None,
                thumb_path: None,
                local_path: None,
                state: BlobState::Remote,
                source_label: String::new(),
            },
            conv: "c".into(),
            conv_title: "c".into(),
            msg: msg.into(),
            hlc_millis: 1,
        }
    }

    #[test]
    fn conv_options_all_first_then_sorted_by_title() {
        let o = conv_options(&[conv("c2", "老张"), conv("c1", "小王"), conv("c0", "小王")]);
        assert_eq!(o[0], (None, strings::FILES_ALL_CONVS.to_string()));
        let ids: Vec<_> = o[1..].iter().map(|x| x.0.clone().unwrap()).collect();
        // 标题相同按 id 排，顺序与会话列表的时间顺序无关
        assert_eq!(ids, vec!["c0", "c1", "c2"]);
    }

    #[test]
    fn conv_index_finds_selection_or_falls_back_to_all() {
        let o = conv_options(&[conv("c1", "小王"), conv("c2", "老张")]);
        let idx = conv_index(&o, &Some("c1".into()));
        assert_eq!(o[idx].0.as_deref(), Some("c1"));
        assert_eq!(conv_index(&o, &None), 0);
        assert_eq!(conv_index(&o, &Some("gone".into())), 0);
    }

    #[test]
    fn kind_and_state_indexes_round_trip() {
        for (i, (k, _)) in KINDS.iter().enumerate() {
            assert_eq!(kind_index(k), i);
        }
        for (i, (s, _)) in STATES.iter().enumerate() {
            assert_eq!(state_index(s), i);
        }
    }

    #[test]
    fn grid_only_for_image_and_video() {
        assert!(is_grid(&FileKindFilter::Image));
        assert!(is_grid(&FileKindFilter::Video));
        assert!(!is_grid(&FileKindFilter::All));
        assert!(!is_grid(&FileKindFilter::Document));
    }

    #[test]
    fn split_source_separates_parts_without_dot() {
        assert_eq!(
            split_source("来自 张三 · 存于 我的台式机"),
            vec!["来自 张三", "存于 我的台式机"]
        );
        assert_eq!(
            split_source("来自 张三 · 已下载"),
            vec!["来自 张三", "已下载"]
        );
        assert!(split_source("").is_empty());
    }

    #[test]
    fn split_source_handles_missing_and_repeated_separators() {
        // 无分隔：整段作为一项
        assert_eq!(split_source("来自 张三"), vec!["来自 张三"]);
        // 多个分隔：每段各成一项
        assert_eq!(
            split_source("来自 张三 · 存于 台式机 · 已下载"),
            vec!["来自 张三", "存于 台式机", "已下载"]
        );
        // 首尾 / 连续分隔产生的空段被丢弃
        assert_eq!(
            split_source(" · 来自 张三 ·  · 已下载 · "),
            vec!["来自 张三", "已下载"]
        );
        // 没有空格的中点不是分隔符
        assert_eq!(split_source("甲·乙"), vec!["甲·乙"]);
    }

    #[test]
    fn meta_parts_skip_conv_already_in_source_and_add_video_duration() {
        let mut f = fv("m", "h");
        f.item.source_label = "来自 张三 · 存于 我的台式机".into();
        f.conv_title = "张三".into();
        let p = meta_parts(&f, "昨天".into());
        assert_eq!(p, vec!["1 B", "昨天", "来自 张三"]);
        assert_eq!(status_parts(&f), "存于 我的台式机");
        // 会话名没出现在来源里时补一段
        f.conv_title = "项目群".into();
        let p = meta_parts(&f, "昨天".into());
        assert_eq!(p.last().map(String::as_str), Some("会话 项目群"));
        // 视频带时长
        f.item.mime = "video/mp4".into();
        f.item.name = "a.mp4".into();
        f.item.duration_ms = Some(7_000);
        assert_eq!(meta_parts(&f, "昨天".into())[1], "0:07");
    }

    #[test]
    fn tile_source_takes_first_segment() {
        assert_eq!(tile_source("来自 张三 · 存于 我的台式机"), "来自 张三");
        assert_eq!(tile_source(""), "");
    }

    #[test]
    fn rows_keys_are_unique_even_for_duplicate_blobs() {
        let files = vec![fv("m1", "h"), fv("m1", "h"), fv("m2", "h")];
        let rows = build_rows(&files, false);
        let keys: std::collections::HashSet<_> = rows.iter().map(|r| r.key.clone()).collect();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn grid_rows_chunk_and_keep_first_key_stable_on_append() {
        let files: Vec<_> = (0..7)
            .map(|i| fv(&format!("m{i}"), &format!("h{i}")))
            .collect();
        let before = build_rows(&files[..5], true);
        let after = build_rows(&files, true);
        assert_eq!(before.len(), 1);
        assert_eq!(after.len(), 2);
        assert_eq!(after[1].files.len(), 2);
        // 追加下一页后，已有整行的键不变，列表不会重建这些行
        assert_eq!(before[0], after[0]);
        assert_eq!(before[0].key, after[0].key);
    }

    #[test]
    fn is_filtered_detects_any_condition() {
        let mut f = Model::new().file_filter;
        assert!(!is_filtered(&f));
        f.name_query = Some("a".into());
        assert!(is_filtered(&f));
    }

    #[test]
    fn content_state_separates_loading_empty_and_filled() {
        let mut m = Model::new();
        assert_eq!(content_state(&m), 0);
        m.files_loaded = true;
        assert_eq!(content_state(&m), 1);
        m.files.push(fv("m", "h"));
        assert_eq!(content_state(&m), 2);
    }
}
