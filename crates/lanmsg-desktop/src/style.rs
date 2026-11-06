// 视觉规范的落地：色值、间距、圆角、图标资源与主题构造（规范 docs/superpowers/specs/2026-11-06-desktop-visual-guide.md）。
// 视图里不得出现色值字面量：能用 windui `Role` 表达的走 `Role`（跟随主题），
// 规范里 windui 没有对应角色的颜色（rail、signal_soft）经 `tokens()` 按当前主题取值，配合 `bg_when` 使用。
// 注释中文，标识符英文。

use windui::prelude::*;
use windui::theme;

// ---------------------------------------------------------------------------
// 色值（规范第 2、4 节）
// ---------------------------------------------------------------------------

/// 一套完整的语义色。浅色与暗色各一份，字段与规范表一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tokens {
    pub ink: Color,
    pub ink_muted: Color,
    pub ink_subtle: Color,
    pub canvas: Color,
    pub rail: Color,
    pub surface: Color,
    pub line: Color,
    pub signal: Color,
    pub signal_soft: Color,
    pub online: Color,
    pub danger: Color,
}

/// 浅色主题色值。
pub const LIGHT: Tokens = Tokens {
    ink: Color::hex(0x16202A),
    ink_muted: Color::hex(0x4F5C68),
    ink_subtle: Color::hex(0x5F6C78),
    canvas: Color::hex(0xF4F6F7),
    rail: Color::hex(0xE9EDEF),
    surface: Color::hex(0xFFFFFF),
    line: Color::hex(0xDCE2E6),
    signal: Color::hex(0x0B7A75),
    signal_soft: Color::hex(0xE8F4F2),
    online: Color::hex(0x1E9E5A),
    danger: Color::hex(0xC2410C),
};

/// 暗色主题色值（规范第 4 节）。
pub const DARK: Tokens = Tokens {
    ink: Color::hex(0xE6ECEF),
    ink_muted: Color::hex(0xA3B0BA),
    ink_subtle: Color::hex(0x8D9AA5),
    canvas: Color::hex(0x12181D),
    rail: Color::hex(0x182027),
    surface: Color::hex(0x1C252C),
    line: Color::hex(0x2B363F),
    signal: Color::hex(0x3FB8AF),
    signal_soft: Color::hex(0x1E3533),
    online: Color::hex(0x3CC27A),
    danger: Color::hex(0xF08A5D),
};

/// 暗色主题下 `signal` 太亮，白字压在上面只有约 2.3:1，所以实底（主按钮、徽章）另取一档深青，
/// `signal`（浅青）只在暗底上当文字、描边、选中色使用。浅色主题二者同值。
const SIGNAL_FILL_DARK: Color = Color::hex(0x0B7A75);
const SIGNAL_FILL_DARK_HOVER: Color = Color::hex(0x0B7F79);
const SIGNAL_FILL_DARK_ACTIVE: Color = Color::hex(0x096560);
const SIGNAL_FILL_LIGHT_HOVER: Color = Color::hex(0x096A65);
const SIGNAL_FILL_LIGHT_ACTIVE: Color = Color::hex(0x075A56);

/// 头像底色：同明度、低饱和，白字对比 >= 4.5:1；亮暗主题不变。
pub const AVATAR_COLORS: [Color; 6] = [
    Color::hex(0x3D6E8F),
    Color::hex(0x5B6E3A),
    Color::hex(0x7A5A8C),
    Color::hex(0x8C5A3D),
    Color::hex(0x3D7A6E),
    Color::hex(0x6B6478),
];

/// 按 id 哈希取头像底色（同一 id 颜色稳定）。
pub fn avatar_color(id: &str) -> Color {
    let h = id
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32));
    AVATAR_COLORS[h as usize % AVATAR_COLORS.len()]
}

// ---------------------------------------------------------------------------
// 间距与形状（规范第 5、6 节）
// ---------------------------------------------------------------------------

pub const SPACE_XS: i32 = 4;
pub const SPACE_SM: i32 = 8;
pub const SPACE_MD: i32 = 12;
pub const SPACE_LG: i32 = 16;
pub const SPACE_XL: i32 = 24;

/// 输入框与按钮圆角。
pub const RADIUS_CONTROL: f32 = 6.0;
/// 图标栏选中块圆角。
pub const RADIUS_NAV_ITEM: f32 = 8.0;
/// 卡片、对话框圆角。
pub const RADIUS_CARD: f32 = 10.0;
/// 胶囊（徽章）。
pub const RADIUS_PILL: f32 = 999.0;

/// 图标栏宽度、项宽与项高。
pub const RAIL_WIDTH: i32 = 64;
pub const NAV_ITEM_W: i32 = 48;
pub const NAV_ITEM_H: i32 = 56;
/// 会话列表行高、头像直径、搜索框高。
pub const CONV_ROW_H: i32 = 64;
pub const AVATAR_SIZE: i32 = 40;
pub const SEARCH_H: i32 = 36;
/// 头像内首字字号。
pub const AVATAR_FONT: f32 = 17.0;
/// 在线点：内圆 10、外圈（底色环）14，叠在头像右下角。
pub const ONLINE_DOT: i32 = 10;
pub const ONLINE_RING: i32 = 14;
/// 图标尺寸：搜索框放大镜 16、图标栏 20、应用标识 28、空状态 48。
pub const ICON_SM: i32 = 16;
pub const ICON_MD: i32 = 20;
pub const LOGO_SIZE: i32 = 28;
pub const ICON_EMPTY_SIZE: i32 = 48;
/// 图标栏图标区（含角标探出的余量）。
pub const NAV_ICON_BOX_W: i32 = 48;
pub const NAV_ICON_BOX_H: i32 = 24;
/// 徽章高度（同时是最小宽度）。
pub const BADGE_H: i32 = 16;
/// 图标栏选中指示条：贴图标栏左缘的 3px 竖条（signal），在 rail 上让选中项一眼可辨。
pub const NAV_BAR_W: i32 = 3;
pub const NAV_BAR_H: i32 = 24;
/// 图标栏项外层宽度（块 48 + 左侧指示条所需余量）。
pub const NAV_SLOT_W: i32 = 56;
/// 向导卡片宽度。
pub const WIZARD_CARD_W: i32 = 440;
/// 按钮与输入框高度。
pub const CONTROL_H: i32 = 36;

// ---------------------------------------------------------------------------
// 字号与字重（规范第 3 节）
// ---------------------------------------------------------------------------

pub const FONT_TITLE: f32 = 20.0;
pub const FONT_HEADING: f32 = 15.0;
pub const FONT_BODY: f32 = 14.0;
pub const FONT_META: f32 = 12.0;
pub const WEIGHT_STRONG: u16 = 600;

// ---------------------------------------------------------------------------
// 主题
// ---------------------------------------------------------------------------

fn tokens_of(dark: bool) -> &'static Tokens {
    if dark {
        &DARK
    } else {
        &LIGHT
    }
}

/// 把规范色值映射到 windui 的 `Palette`。
pub fn palette(dark: bool) -> Palette {
    let t = tokens_of(dark);
    let base = if dark {
        Palette::dark()
    } else {
        Palette::default()
    };
    let (fill, hover, active) = if dark {
        (
            SIGNAL_FILL_DARK,
            SIGNAL_FILL_DARK_HOVER,
            SIGNAL_FILL_DARK_ACTIVE,
        )
    } else {
        (
            LIGHT.signal,
            SIGNAL_FILL_LIGHT_HOVER,
            SIGNAL_FILL_LIGHT_ACTIVE,
        )
    };
    Palette {
        accent: fill,
        accent_hover: hover,
        accent_active: active,
        on_accent: Color::WHITE,
        // 强调色当前景（选中文字、焦点环、文字按钮）用 signal 本色
        accent_text: Some(t.signal),
        bg: t.canvas,
        surface: t.surface,
        surface_alt: t.canvas,
        surface_inverse: t.ink,
        on_surface_inverse: t.canvas,
        text: t.ink,
        text_muted: t.ink_muted,
        text_subtle: t.ink_subtle,
        border: t.line,
        track: t.line,
        placeholder: t.ink_subtle,
        divider: t.line,
        danger: t.danger,
        success: t.online,
        // text_disabled / warning 规范未规定，沿用 windui 同明暗的取值
        ..base
    }
}

/// 浅色或暗色的完整主题。
pub fn theme(dark: bool) -> Theme {
    let mut t = if dark {
        Theme::dark()
    } else {
        Theme::default()
    };
    t.palette = palette(dark);
    t.metrics.corner_md = RADIUS_CONTROL;
    t.metrics.corner_lg = RADIUS_CARD;
    t.metrics.control_h = CONTROL_H;
    t
}

/// 按当前活动主题取色值；`bg_when` 等每帧求值的闭包里用它，换主题自动跟随。
pub fn tokens() -> &'static Tokens {
    tokens_of(theme::current().palette.bg == DARK.canvas)
}

// ---------------------------------------------------------------------------
// 图标（assets/icons，20px 线性图标，黑色描边作着色模板）
// ---------------------------------------------------------------------------

pub const ICON_CHATS: &[u8] = include_bytes!("../assets/icons/chats.svg");
pub const ICON_CONTACTS: &[u8] = include_bytes!("../assets/icons/contacts.svg");
pub const ICON_FILES: &[u8] = include_bytes!("../assets/icons/files.svg");
pub const ICON_SETTINGS: &[u8] = include_bytes!("../assets/icons/settings.svg");
pub const ICON_SEARCH: &[u8] = include_bytes!("../assets/icons/search.svg");
/// 空状态用的 48px 对话图标。
pub const ICON_EMPTY: &[u8] = include_bytes!("../assets/icons/empty.svg");
/// 应用标识（自带 signal 底色，不着色）。
pub const ICON_LOGO: &[u8] = include_bytes!("../assets/icons/logo.svg");

/// 文件类型图标（24px 视口，单色模板）。
pub const ICON_FILE_GENERIC: &[u8] = include_bytes!("../assets/icons/file_generic.svg");
pub const ICON_FILE_TEXT: &[u8] = include_bytes!("../assets/icons/file_text.svg");
pub const ICON_FILE_DOC: &[u8] = include_bytes!("../assets/icons/file_doc.svg");
pub const ICON_FILE_PDF: &[u8] = include_bytes!("../assets/icons/file_pdf.svg");
pub const ICON_FILE_SHEET: &[u8] = include_bytes!("../assets/icons/file_sheet.svg");
pub const ICON_FILE_SLIDE: &[u8] = include_bytes!("../assets/icons/file_slide.svg");
pub const ICON_FILE_ARCHIVE: &[u8] = include_bytes!("../assets/icons/file_archive.svg");
pub const ICON_FILE_IMAGE: &[u8] = include_bytes!("../assets/icons/file_image.svg");
pub const ICON_FILE_VIDEO: &[u8] = include_bytes!("../assets/icons/file_video.svg");
pub const ICON_FILE_AUDIO: &[u8] = include_bytes!("../assets/icons/file_audio.svg");
pub const ICON_DOWNLOAD: &[u8] = include_bytes!("../assets/icons/download.svg");
pub const ICON_PLAY: &[u8] = include_bytes!("../assets/icons/play.svg");

/// 线性图标：SVG 按 `size` 逻辑像素光栅化，并按主题角色着色（换主题自动跟随）。
pub fn icon(svg: &'static [u8], size: i32, role: Role) -> Element {
    Element::image_svg(svg, None)
        .size(size, size)
        .tint_role(role)
}

/// 实底胶囊徽章（未读数、请求数）：`signal` 实底白字，最小宽 = 高 = 16。
pub fn badge(text: impl Into<TextContent>) -> Element {
    Element::stack()
        .height(BADGE_H)
        .min_width(BADGE_H)
        .padding_xy(5, 0)
        .corner(RADIUS_PILL)
        .bg_role(Role::Accent)
        .child(
            Element::label(text)
                .font_size(FONT_META)
                .font_weight(WEIGHT_STRONG)
                .fg_role(Role::OnAccent)
                .align(Align::Center),
        )
}

/// 1px 竖分隔线（`line` 色），撑满父容器高度。
pub fn vline() -> Element {
    Element::leaf()
        .width(1)
        .height_match()
        .bg_role(Role::Divider)
}

/// 空状态：居中的 48px 线性图标 + 标题 + 一句可行动的说明 + 可选的行动入口（链接或按钮）。
pub fn empty_state(title: &'static str, hint: &'static str, action: Option<Element>) -> Element {
    let mut col = Element::col()
        .spacing(SPACE_SM)
        .cross(Align::Center)
        .child(icon(ICON_EMPTY, ICON_EMPTY_SIZE, Role::TextSubtle))
        .child(Element::leaf().height(SPACE_XS))
        .child(
            Element::label(title)
                .font_size(FONT_HEADING)
                .font_weight(WEIGHT_STRONG)
                .fg_role(Role::Text),
        )
        .child(
            Element::label(hint)
                .font_size(FONT_BODY)
                .fg_role(Role::TextMuted)
                .text_align(Align::Center),
        );
    if let Some(a) = action {
        col = col.child(Element::leaf().height(SPACE_XS)).child(a);
    }
    Element::stack()
        .fill()
        .padding(SPACE_XL)
        .child(col.align(Align::Center))
}

/// 窗口图标：logo.svg 按系统要的像素数现画，每档 1:1；解码失败时退回 windui 内置品牌图标。
pub fn window_icon() -> windui::icon::IconSource {
    windui::icon::IconSource::sized(|size| {
        windui::render::image::Image::from_svg_bytes(ICON_LOGO, Some(size))
            .ok()
            .and_then(|img| windui::icon::WindowIcon::from_image(&img))
            .unwrap_or_else(|| windui::icon::brand_icon_at(size))
    })
}

/// 正文行高（字号的倍数）：多行消息与输入框用，约 1.45 倍。
pub const LINE_HEIGHT_BODY: f32 = 1.45;

/// 应用标识：保持自身颜色。
pub fn logo(size: i32) -> Element {
    Element::image_svg(ICON_LOGO, None).size(size, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 相对亮度。
    fn luminance(c: Color) -> f64 {
        let lin = |v: u8| {
            let s = v as f64 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn avatar_colors_hold_white_text_contrast() {
        for c in AVATAR_COLORS {
            assert!(contrast(c, Color::WHITE) >= 4.5, "{c:?}");
        }
    }

    #[test]
    fn accent_fill_holds_white_text_contrast_in_both_themes() {
        for dark in [false, true] {
            assert!(contrast(palette(dark).accent, Color::WHITE) >= 4.5);
        }
    }

    /// 规范第 11 节：正文三档文字 × 四种底色（浅暗两套）、白字 × 主按钮实底/悬停，全部 >= 4.5:1。
    #[test]
    fn text_and_button_contrast_meets_wcag_aa() {
        let mut fails = Vec::new();
        for (name, t) in [("light", &LIGHT), ("dark", &DARK)] {
            for (tn, fg) in [
                ("ink", t.ink),
                ("ink_muted", t.ink_muted),
                ("ink_subtle", t.ink_subtle),
            ] {
                for (bn, bg) in [
                    ("surface", t.surface),
                    ("canvas", t.canvas),
                    ("rail", t.rail),
                    ("signal_soft", t.signal_soft),
                ] {
                    let r = contrast(fg, bg);
                    println!("{name} {tn} on {bn}: {r:.2}");
                    if r < 4.5 {
                        fails.push(format!("{name} {tn} on {bn}: {r:.2}"));
                    }
                }
            }
            // signal 当文字用时（选中项）落在 signal_soft / surface 上
            for (bn, bg) in [("signal_soft", t.signal_soft), ("surface", t.surface)] {
                let r = contrast(t.signal, bg);
                println!("{name} signal on {bn}: {r:.2}");
                if r < 4.5 {
                    fails.push(format!("{name} signal on {bn}: {r:.2}"));
                }
            }
            let p = palette(name == "dark");
            for (bn, bg) in [
                ("fill", p.accent),
                ("hover", p.accent_hover),
                ("active", p.accent_active),
            ] {
                let r = contrast(Color::WHITE, bg);
                println!("{name} white on button {bn}: {r:.2}");
                if r < 4.5 {
                    fails.push(format!("{name} white on button {bn}: {r:.2}"));
                }
            }
        }
        assert!(fails.is_empty(), "对比度不足: {fails:?}");
    }

    #[test]
    fn avatar_color_is_stable_per_id() {
        assert_eq!(avatar_color("c-dm"), avatar_color("c-dm"));
    }

    #[test]
    fn layers_are_ordered_by_brightness_in_light_theme() {
        // rail < canvas < surface：层级靠底色拉开
        assert!(luminance(LIGHT.rail) < luminance(LIGHT.canvas));
        assert!(luminance(LIGHT.canvas) < luminance(LIGHT.surface));
        // 暗色相反：rail < canvas 不成立，但三层须互不相同且 surface 最亮
        assert!(luminance(DARK.canvas) < luminance(DARK.rail));
        assert!(luminance(DARK.rail) < luminance(DARK.surface));
    }
}
