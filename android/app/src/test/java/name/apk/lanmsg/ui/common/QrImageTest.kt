package name.apk.lanmsg.ui.common

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.google.zxing.BinaryBitmap
import com.google.zxing.MultiFormatReader
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.common.HybridBinarizer
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class QrImageTest {
    @Test fun encode_produces_square_bitmap_decodable_by_zxing() {
        val text = "lanmsg://pair?d=abc&s=xyz"
        val bmp = QrEncoder.encode(text, 300)
        assertEquals(300, bmp.width); assertEquals(300, bmp.height)
        val pixels = IntArray(300 * 300).also { bmp.getPixels(it, 0, 300, 0, 0, 300, 300) }
        val source = RGBLuminanceSource(300, 300, pixels)
        val decoded = MultiFormatReader().decode(BinaryBitmap(HybridBinarizer(source)))
        assertEquals(text, decoded.text)
    }

    @Test fun long_text_still_decodes_and_has_white_quiet_zone() {
        val text = "lanmsg://pair?" + "a=192.168.100.200:12345&".repeat(8) + "k=" + "Z".repeat(60)
        val size = 600
        val bmp = QrEncoder.encode(text, size)
        val pixels = IntArray(size * size).also { bmp.getPixels(it, 0, size, 0, 0, size, size) }
        // 最外圈（≥ 4 个模块宽的白边内）全白
        val edge = size / 20
        for (i in 0 until size) for (d in 0 until edge) {
            assertEquals(0xFFFFFFFF.toInt(), pixels[d * size + i])
            assertEquals(0xFFFFFFFF.toInt(), pixels[i * size + d])
        }
        val decoded = MultiFormatReader().decode(BinaryBitmap(HybridBinarizer(RGBLuminanceSource(size, size, pixels))))
        assertEquals(text, decoded.text)
    }
}
