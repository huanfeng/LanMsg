package name.apk.lanmsg.ui.theme

import androidx.compose.ui.graphics.Color
import kotlin.math.pow
import org.junit.Assert.assertTrue
import org.junit.Test

/** 对应桌面 style.rs 的对比度测试：文字与底色、白字与实底对比度 >= 4.5:1（WCAG AA）。 */
class ContrastTest {
    private fun lin(v: Float): Double {
        val c = v.toDouble()
        return if (c <= 0.03928) c / 12.92 else ((c + 0.055) / 1.055).pow(2.4)
    }

    private fun luminance(c: Color) = 0.2126 * lin(c.red) + 0.7152 * lin(c.green) + 0.0722 * lin(c.blue)

    private fun contrast(a: Color, b: Color): Double {
        val (hi, lo) = luminance(a).let { la -> luminance(b).let { lb -> maxOf(la, lb) to minOf(la, lb) } }
        return (hi + 0.05) / (lo + 0.05)
    }

    private val themes = listOf("light" to LightTokens, "dark" to DarkTokens)

    @Test fun white_on_signal_fill_and_states() {
        val fails = mutableListOf<String>()
        for ((n, t) in themes) {
            for ((fn, fill) in listOf(
                "fill" to t.signalFill, "hover" to t.signalFillHover, "pressed" to t.signalFillPressed,
            )) {
                val r = contrast(t.onFill, fill)
                if (r < 4.5) fails += "$n white x $fn = $r"
            }
        }
        assertTrue(fails.toString(), fails.isEmpty())
    }

    @Test fun text_on_surfaces() {
        val fails = mutableListOf<String>()
        for ((n, t) in themes) {
            for ((fn, fg) in listOf("ink" to t.ink, "ink_muted" to t.inkMuted, "ink_subtle" to t.inkSubtle)) {
                for ((bn, bg) in listOf("surface" to t.surface, "canvas" to t.canvas, "signal_soft" to t.signalSoft)) {
                    val r = contrast(fg, bg)
                    if (r < 4.5) fails += "$n $fn x $bn = $r"
                }
            }
        }
        assertTrue(fails.toString(), fails.isEmpty())
    }

    @Test fun signal_and_danger_text_on_surface_and_canvas() {
        val fails = mutableListOf<String>()
        for ((n, t) in themes) {
            for ((fn, fg) in listOf("signal" to t.signal, "danger" to t.danger)) {
                for ((bn, bg) in listOf("surface" to t.surface, "canvas" to t.canvas)) {
                    val r = contrast(fg, bg)
                    if (r < 4.5) fails += "$n $fn x $bn = $r"
                }
            }
        }
        assertTrue(fails.toString(), fails.isEmpty())
    }

    @Test fun signal_text_on_signal_soft() {
        val fails = mutableListOf<String>()
        for ((n, t) in themes) {
            val r = contrast(t.signal, t.signalSoft)
            if (r < 4.5) fails += "$n signal x signal_soft = $r"
        }
        assertTrue(fails.toString(), fails.isEmpty())
    }

    @Test fun white_on_light_danger_fill_and_avatars() {
        assertTrue(contrast(LightTokens.onFill, LightTokens.danger) >= 4.5)
        for (c in AvatarPalette) assertTrue("$c", contrast(Color.White, c) >= 4.5)
    }
}
