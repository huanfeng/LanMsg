package name.apk.lanmsg.ui.common

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import name.apk.lanmsg.R
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

object QrEncoder {
    private const val BLACK = 0xFF000000.toInt()
    private const val WHITE = 0xFFFFFFFF.toInt()

    /** ZXing QRCodeWriter，纠错级别 M，边距 1 个模块；黑码白底，与主题无关（扫码需要）。 */
    fun encode(text: String, sizePx: Int): Bitmap {
        val hints = mapOf(
            EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M,
            EncodeHintType.MARGIN to 1,
            EncodeHintType.CHARACTER_SET to "UTF-8",
        )
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, sizePx, sizePx, hints)
        val pixels = IntArray(sizePx * sizePx)
        for (y in 0 until sizePx) for (x in 0 until sizePx) {
            pixels[y * sizePx + x] = if (matrix[x, y]) BLACK else WHITE
        }
        return Bitmap.createBitmap(pixels, sizePx, sizePx, Bitmap.Config.ARGB_8888)
    }
}

@Composable
fun QrImage(text: String, size: Dp = 240.dp) {
    val desc = stringResource(R.string.pair_qr_desc)
    val px = with(LocalDensity.current) { size.roundToPx() }
    val bitmap = remember(text, px) { QrEncoder.encode(text, px).asImageBitmap() }
    // 白色底板保证暗色主题下也能扫
    Image(bitmap, contentDescription = desc, modifier = Modifier.size(size).background(Color.White))
}
