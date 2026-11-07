package name.apk.lanmsg.ui.devices

import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader

/** 相机帧 → 二维码文本。同一文本 [DEDUP_MILLIS] 内只回调一次。 */
class QrAnalyzer(
    private val onResult: (String) -> Unit,
    private val clock: () -> Long = System::currentTimeMillis,
) : ImageAnalysis.Analyzer {
    private var lastText: String? = null
    private var lastAt = 0L

    override fun analyze(image: ImageProxy) {
        try {
            val plane = image.planes[0]
            val buf = plane.buffer
            val y = ByteArray(buf.remaining()).also { buf.get(it) }
            val text = decodeLuminance(y, image.width, image.height, plane.rowStride) ?: return
            val now = clock()
            if (text == lastText && now - lastAt < DEDUP_MILLIS) return
            lastText = text
            lastAt = now
            onResult(text)
        } finally {
            image.close()
        }
    }

    companion object {
        private const val DEDUP_MILLIS = 2000L

        /** [y] 为亮度平面，行距 [rowStride]（>= width，相机帧常有行尾填充）；没有二维码返回 null。 */
        fun decodeLuminance(y: ByteArray, width: Int, height: Int, rowStride: Int = width): String? {
            if (rowStride < width || y.size < rowStride * (height - 1) + width) return null
            val source = PlanarYUVLuminanceSource(y, rowStride, height, 0, 0, width, height, false)
            return try {
                QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source)), mapOf(DecodeHintType.TRY_HARDER to true)).text
            } catch (e: NotFoundException) {
                null
            } catch (e: ReaderException) {
                null // 校验或格式错误：下一帧再试
            }
        }
    }
}
