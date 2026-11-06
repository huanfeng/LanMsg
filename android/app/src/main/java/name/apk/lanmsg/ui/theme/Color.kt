package name.apk.lanmsg.ui.theme

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

// 视觉令牌，数值以 docs/ui-architecture.md §5.1 为准（与桌面 style.rs 同一套色值）。
// 页面代码只能通过 LanMsgColors 取色，不得出现色值字面量。

/** 一套完整的语义色，浅色与暗色各一份。 */
@Immutable
data class LanMsgColorTokens(
    val ink: Color,
    val inkMuted: Color,
    val inkSubtle: Color,
    val canvas: Color,
    val surface: Color,
    val line: Color,
    /** 强调色：文字、描边、选中。暗色下太亮，不得作白字实底。 */
    val signal: Color,
    val signalSoft: Color,
    /** 实底：主按钮、我的气泡、未读角标，配白字。 */
    val signalFill: Color,
    val signalFillHover: Color,
    val signalFillPressed: Color,
    val online: Color,
    val danger: Color,
    /** 实底上的文字色（恒为白）。 */
    val onFill: Color,
    val isDark: Boolean,
)

val LightTokens = LanMsgColorTokens(
    ink = Color(0xFF16202A),
    inkMuted = Color(0xFF4F5C68),
    inkSubtle = Color(0xFF5F6C78),
    canvas = Color(0xFFF4F6F7),
    surface = Color(0xFFFFFFFF),
    line = Color(0xFFDCE2E6),
    signal = Color(0xFF0B7A75),
    signalSoft = Color(0xFFE8F4F2),
    signalFill = Color(0xFF0B7A75),
    signalFillHover = Color(0xFF096A65),
    signalFillPressed = Color(0xFF075A56),
    online = Color(0xFF1E9E5A),
    danger = Color(0xFFC2410C),
    onFill = Color(0xFFFFFFFF),
    isDark = false,
)

val DarkTokens = LanMsgColorTokens(
    ink = Color(0xFFE6ECEF),
    inkMuted = Color(0xFFA3B0BA),
    inkSubtle = Color(0xFF8D9AA5),
    canvas = Color(0xFF12181D),
    surface = Color(0xFF1C252C),
    line = Color(0xFF2B363F),
    signal = Color(0xFF3FB8AF),
    signalSoft = Color(0xFF1E3533),
    signalFill = Color(0xFF0B7A75),
    signalFillHover = Color(0xFF0B7F79),
    signalFillPressed = Color(0xFF096560),
    online = Color(0xFF3CC27A),
    danger = Color(0xFFF08A5D),
    onFill = Color(0xFFFFFFFF),
    isDark = true,
)

/** 头像 6 色调色板，亮暗主题不变，白字对比度均 >= 4.5:1。顺序即 djb2 取色的下标顺序。 */
val AvatarPalette: List<Color> = listOf(
    Color(0xFF3D6E8F),
    Color(0xFF5B6E3A),
    Color(0xFF7A5A8C),
    Color(0xFF8C5A3D),
    Color(0xFF3D7A6E),
    Color(0xFF6B6478),
)

internal val LocalLanMsgColors = staticCompositionLocalOf { LightTokens }

/** 页面取色入口：`LanMsgColors.current.signal`。 */
object LanMsgColors {
    val current: LanMsgColorTokens
        @Composable
        @ReadOnlyComposable
        get() = LocalLanMsgColors.current
}
