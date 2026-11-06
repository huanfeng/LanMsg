package name.apk.lanmsg.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CheckboxColors
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Shapes
import androidx.compose.material3.SwitchColors
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.TextFieldColors
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

// 控件配色约定（暗色下 primary = signalFill 是实底深青，对 canvas/surface 仅约 3:1，不能当强调色用）：
// - 按钮：TextButton 用 LanMsgButtons.text()，危险按钮用 LanMsgButtons.danger()；
// - 输入框、开关、复选框、进度条：用 LanMsgControls（强调一律取 signal）；
// - 不要依赖 Material 默认 primary。
// ColorScheme 映射：primary 只给实底按钮（signalFill + 白字）；secondary / tertiary 映射为 signal，
// 使依赖它们的默认控件（如 FilterChip、Slider 等）拿到可读的强调色。

/** 令牌到 Material3 ColorScheme 的映射。primary 取 signalFill，使默认 Button 就是“实底白字”。 */
fun LanMsgColorTokens.toColorScheme(): ColorScheme {
    val base = if (isDark) {
        darkColorScheme(onError = canvas)
    } else {
        lightColorScheme(onError = onFill)
    }
    return base.copy(
        primary = signalFill,
        onPrimary = onFill,
        primaryContainer = signalSoft,
        onPrimaryContainer = signal,
        secondary = signal,
        onSecondary = onFill,
        secondaryContainer = signalSoft,
        onSecondaryContainer = signal,
        tertiary = signal,
        background = canvas,
        onBackground = ink,
        surface = surface,
        onSurface = ink,
        surfaceVariant = canvas,
        onSurfaceVariant = inkMuted,
        surfaceTint = surface,
        surfaceBright = surface,
        surfaceDim = canvas,
        surfaceContainerLowest = surface,
        surfaceContainerLow = surface,
        surfaceContainer = surface,
        surfaceContainerHigh = surface,
        surfaceContainerHighest = canvas,
        outline = line,
        outlineVariant = line,
        error = danger,
    )
}

private val LanMsgShapes = Shapes(
    extraSmall = RoundedCornerShape(4.dp),
    small = RoundedCornerShape(6.dp),
    medium = RoundedCornerShape(10.dp),
    large = RoundedCornerShape(10.dp),
    extraLarge = RoundedCornerShape(16.dp),
)

/** Material3，跟随系统深浅色；不用动态取色（品牌色固定，见 docs/ui-architecture.md §5.1）。 */
@Composable
fun LanMsgTheme(content: @Composable () -> Unit) {
    val tokens = if (isSystemInDarkTheme()) DarkTokens else LightTokens
    CompositionLocalProvider(LocalLanMsgColors provides tokens) {
        MaterialTheme(colorScheme = tokens.toColorScheme(), shapes = LanMsgShapes, content = content)
    }
}

/**
 * 按钮配色。暗色下 primary 是实底深青，不能当文字色，所以文字按钮必须用 [text]（signal 字）。
 */
object LanMsgButtons {
    /** 文字按钮：仅 signal 字。 */
    @Composable
    fun text(): ButtonColors = ButtonDefaults.textButtonColors(contentColor = LanMsgColors.current.signal)

    /** 危险确认：浅色 danger 实底白字；暗色 surface 底 + danger 字（调用方另加 1px danger 描边）。 */
    @Composable
    fun danger(): ButtonColors {
        val c = LanMsgColors.current
        return if (c.isDark) {
            ButtonDefaults.outlinedButtonColors(containerColor = c.surface, contentColor = c.danger)
        } else {
            ButtonDefaults.buttonColors(containerColor = c.danger, contentColor = c.onFill)
        }
    }
}

/**
 * 输入类控件配色：焦点描边、光标、选中态统一取 signal（暗色 #3FB8AF），保证对 canvas/surface 对比度。
 */
object LanMsgControls {
    @Composable
    fun textField(): TextFieldColors {
        val c = LanMsgColors.current
        return OutlinedTextFieldDefaults.colors(
            focusedBorderColor = c.signal,
            unfocusedBorderColor = c.line,
            cursorColor = c.signal,
            focusedLabelColor = c.signal,
        )
    }

    @Composable
    fun switch(): SwitchColors {
        val c = LanMsgColors.current
        // 暗色 signal 偏亮，滑块用 canvas 深色才与轨道有反差
        val thumb = if (c.isDark) c.canvas else c.onFill
        return SwitchDefaults.colors(
            checkedThumbColor = thumb,
            checkedTrackColor = c.signal,
            checkedBorderColor = c.signal,
            uncheckedThumbColor = c.inkMuted,
            uncheckedTrackColor = c.surface,
            uncheckedBorderColor = c.inkMuted,
        )
    }

    @Composable
    fun checkbox(): CheckboxColors {
        val c = LanMsgColors.current
        return CheckboxDefaults.colors(
            checkedColor = c.signal,
            checkmarkColor = if (c.isDark) c.canvas else c.onFill,
            uncheckedColor = c.inkMuted,
        )
    }

    /** 进度指示器颜色（CircularProgressIndicator / LinearProgressIndicator 的 color 参数）。 */
    val progress: Color
        @Composable
        get() = LanMsgColors.current.signal
}
