package name.apk.lanmsg.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.unit.dp

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
