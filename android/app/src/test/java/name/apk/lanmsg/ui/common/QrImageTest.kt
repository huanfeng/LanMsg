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
}
