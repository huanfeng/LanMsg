package name.apk.lanmsg.files

import android.content.ContentValues
import android.content.Context
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException

/** 把已下载的图片/视频保存到系统相册（图片 Pictures/LanMsg，视频 Movies/LanMsg）。 */
open class MediaStoreSaver(private val ctx: Context) {
    /** 成功返回新条目的 Uri；mime 不是图片/视频或写入失败返回 null。 */
    open suspend fun saveToGallery(localPath: String, mime: String, displayName: String): Uri? = withContext(Dispatchers.IO) {
        val isImage = mime.startsWith("image/")
        if (!isImage && !mime.startsWith("video/")) return@withContext null
        val src = File(localPath)
        if (!src.isFile) return@withContext null
        val name = OutboxImporter.sanitize(displayName)
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) saveScoped(src, mime, name, isImage) else saveLegacy(src, mime, name, isImage)
        } catch (e: IOException) {
            Log.w(TAG, "Save to gallery failed: $localPath", e)
            null
        } catch (e: RuntimeException) {
            // SecurityException、resolver.insert 的 IllegalArgumentException 等
            Log.w(TAG, "Save to gallery failed: $localPath", e)
            null
        }
    }

    private fun saveScoped(src: File, mime: String, name: String, isImage: Boolean): Uri? {
        val resolver = ctx.contentResolver
        val collection = if (isImage) MediaStore.Images.Media.EXTERNAL_CONTENT_URI else MediaStore.Video.Media.EXTERNAL_CONTENT_URI
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, name)
            put(MediaStore.MediaColumns.MIME_TYPE, mime)
            put(MediaStore.MediaColumns.RELATIVE_PATH, (if (isImage) Environment.DIRECTORY_PICTURES else Environment.DIRECTORY_MOVIES) + "/LanMsg/")
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = resolver.insert(collection, values) ?: return null
        try {
            val out = resolver.openOutputStream(uri) ?: throw IOException("Cannot open $uri")
            out.use { dst -> src.inputStream().use { it.copyTo(dst) } }
            resolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            throw e
        }
        return uri
    }

    @Suppress("DEPRECATION")
    private fun saveLegacy(src: File, mime: String, name: String, isImage: Boolean): Uri {
        val pub = Environment.getExternalStoragePublicDirectory(if (isImage) Environment.DIRECTORY_PICTURES else Environment.DIRECTORY_MOVIES)
        val dir = File(pub, "LanMsg")
        if (!dir.isDirectory && !dir.mkdirs()) throw IOException("Cannot create directory $dir")
        val dot = name.lastIndexOf('.')
        val (base, ext) = if (dot > 0) name.substring(0, dot) to name.substring(dot) else name to ""
        var n = 0
        var target = File(dir, name)
        while (!target.createNewFile()) target = File(dir, "$base (${++n})$ext")
        try {
            src.copyTo(target, overwrite = true)
        } catch (e: Exception) {
            target.delete()
            throw e
        }
        MediaScannerConnection.scanFile(ctx, arrayOf(target.path), arrayOf(mime), null)
        return Uri.fromFile(target)
    }

    private companion object {
        const val TAG = "MediaStoreSaver"
    }
}
