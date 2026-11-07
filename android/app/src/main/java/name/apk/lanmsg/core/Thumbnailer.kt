package name.apk.lanmsg.core

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ImageDecoder
import android.media.MediaMetadataRetriever
import android.os.Build
import android.util.Log
import uniffi.lanmsg_ffi.ThumbnailResult
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.math.max

/** 生成图片/视频缩略图；任何异常都返回 null（缩略图是可选的）。 */
class Thumbnailer(@Suppress("unused") private val ctx: Context) {

    fun thumbnail(path: String, mime: String, maxEdge: Int): ThumbnailResult? = try {
        when {
            mime.startsWith("image/") -> imageThumbnail(path, maxEdge)
            mime.startsWith("video/") -> videoThumbnail(path, maxEdge)
            else -> null
        }
    } catch (t: Throwable) {
        Log.w(TAG, "Thumbnail generation failed: $path", t)
        null
    }

    private fun imageThumbnail(path: String, maxEdge: Int): ThumbnailResult? {
        val bitmap = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            ImageDecoder.decodeBitmap(ImageDecoder.createSource(File(path))) { decoder, info, _ ->
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                val longest = max(info.size.width, info.size.height)
                if (longest > maxEdge) {
                    val scale = maxEdge.toFloat() / longest
                    decoder.setTargetSize(
                        max(1, (info.size.width * scale).toInt()),
                        max(1, (info.size.height * scale).toInt()),
                    )
                }
            }
        } else {
            decodeLegacy(path, maxEdge)
        }
        return toResult(bitmap, null)
    }

    // API 26–27：先用 inSampleSize 预缩放，再精确缩放到长边 maxEdge
    private fun decodeLegacy(path: String, maxEdge: Int): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(path, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        var sample = 1
        while (max(bounds.outWidth, bounds.outHeight) / (sample * 2) >= maxEdge) sample *= 2
        val raw = BitmapFactory.decodeFile(path, BitmapFactory.Options().apply { inSampleSize = sample }) ?: return null
        return scaleDown(raw, maxEdge)
    }

    private fun videoThumbnail(path: String, maxEdge: Int): ThumbnailResult? {
        val retriever = MediaMetadataRetriever()
        try {
            retriever.setDataSource(path)
            val frame = retriever.getFrameAtTime(0) ?: return null
            val duration = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()
            return toResult(scaleDown(frame, maxEdge), duration?.toULong())
        } finally {
            retriever.release()
        }
    }

    private fun scaleDown(bmp: Bitmap, maxEdge: Int): Bitmap {
        val longest = max(bmp.width, bmp.height)
        if (longest <= maxEdge) return bmp
        val scale = maxEdge.toFloat() / longest
        val scaled = Bitmap.createScaledBitmap(
            bmp,
            max(1, (bmp.width * scale).toInt()),
            max(1, (bmp.height * scale).toInt()),
            true,
        )
        if (scaled !== bmp) bmp.recycle() // 原图不再使用
        return scaled
    }

    private fun toResult(bmp: Bitmap?, durationMs: ULong?): ThumbnailResult? {
        if (bmp == null) return null
        try {
            val jpeg = compressToLimit(bmp, THUMB_MAX_BYTES) ?: return null
            return ThumbnailResult(jpeg, bmp.width.toUInt(), bmp.height.toUInt(), durationMs)
        } finally {
            bmp.recycle() // 调用方交出所有权，此后不再使用
        }
    }

    companion object {
        private const val TAG = "Thumbnailer"

        /** 与 `lanmsg_proto::consts::THUMB_MAX_BYTES` 一致。 */
        const val THUMB_MAX_BYTES = 32 * 1024

        /** JPEG 质量 85→75→…→35 依次尝试，仍超限返回 null。 */
        fun compressToLimit(bitmap: Bitmap, maxBytes: Int): ByteArray? {
            // JPEG 无透明通道，带 alpha 的图直接压会变黑底，先铺白底
            val src = if (bitmap.hasAlpha()) flattenOnWhite(bitmap) else bitmap
            try {
                return compressLoop(src, maxBytes)
            } finally {
                if (src !== bitmap) src.recycle()
            }
        }

        private fun flattenOnWhite(bitmap: Bitmap): Bitmap {
            val out = Bitmap.createBitmap(bitmap.width, bitmap.height, Bitmap.Config.ARGB_8888)
            val canvas = Canvas(out)
            canvas.drawColor(Color.WHITE)
            canvas.drawBitmap(bitmap, 0f, 0f, null)
            return out
        }

        private fun compressLoop(bitmap: Bitmap, maxBytes: Int): ByteArray? {
            var quality = 85
            while (quality >= 35) {
                val out = ByteArrayOutputStream()
                bitmap.compress(Bitmap.CompressFormat.JPEG, quality, out)
                if (out.size() <= maxBytes) return out.toByteArray()
                quality -= 10
            }
            return null
        }
    }
}
