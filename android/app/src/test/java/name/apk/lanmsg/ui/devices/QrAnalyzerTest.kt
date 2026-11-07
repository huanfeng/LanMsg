package name.apk.lanmsg.ui.devices

import androidx.test.ext.junit.runners.AndroidJUnit4
import name.apk.lanmsg.ui.common.QrEncoder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class QrAnalyzerTest {
    private fun luminance(text: String, size: Int): ByteArray {
        val bmp = QrEncoder.encode(text, size)
        val pixels = IntArray(size * size).also { bmp.getPixels(it, 0, size, 0, 0, size, size) }
        return ByteArray(size * size) { (pixels[it] and 0xFF).toByte() } // 黑白图，取蓝色通道即亮度
    }

    @Test fun decodes_luminance_of_generated_qr() {
        assertEquals("lanmsg://pair?d=1", QrAnalyzer.decodeLuminance(luminance("lanmsg://pair?d=1", 400), 400, 400))
    }

    @Test fun decodes_when_row_stride_is_wider_than_width() {
        val w = 300
        val stride = 320
        val src = luminance("lanmsg://pair?d=2", w)
        val padded = ByteArray(stride * w) { 0 }
        for (y in 0 until w) System.arraycopy(src, y * w, padded, y * stride, w)
        assertEquals("lanmsg://pair?d=2", QrAnalyzer.decodeLuminance(padded, w, w, stride))
    }

    @Test fun returns_null_when_no_code() {
        assertNull(QrAnalyzer.decodeLuminance(ByteArray(200 * 200) { 0x80.toByte() }, 200, 200))
    }
}
