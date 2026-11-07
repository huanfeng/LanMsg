package name.apk.lanmsg.files

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** 把外部来源（content:// 等）的文件落地成 core 可读的本地路径。ViewModel 依赖此窄接口便于测试。 */
interface Importer {
    suspend fun import(uris: List<Uri>): List<String>
}

/**
 * 发件箱：把 content:// URI 复制到 `outbox/<yyyyMMdd>/` 下的普通文件。
 * 文件名来自外部应用，不可信：先 [sanitize]，写入前再断言目标路径位于 outboxRoot 之内。
 */
class OutboxImporter(
    private val ctx: Context,
    private val outboxRoot: File = File(ctx.filesDir, "outbox"),
) : Importer {
    /** 返回落地后的绝对路径，顺序与输入一致；失败的 URI 跳过并记日志。 */
    override suspend fun import(uris: List<Uri>): List<String> = withContext(Dispatchers.IO) {
        val root = outboxRoot.canonicalFile
        val dir = File(root, SimpleDateFormat("yyyyMMdd", Locale.ROOT).format(Date()))
        uris.mapNotNull { uri ->
            try {
                copyOne(uri, dir, root)
            } catch (e: IOException) {
                Log.w(TAG, "导入失败，已跳过：$uri", e)
                null
            } catch (e: SecurityException) {
                Log.w(TAG, "无权读取，已跳过：$uri", e)
                null
            }
        }
    }

    private fun copyOne(uri: Uri, dir: File, root: File): String {
        if (!dir.isDirectory && !dir.mkdirs()) throw IOException("无法创建目录 $dir")
        val name = sanitize(displayName(uri))
        val target = reserve(dir, name)
        try {
            // 防目录穿越的第二道保险：即使 sanitize 有疏漏，也不写到 outbox 之外
            if (!target.canonicalPath.startsWith(root.path + File.separator)) {
                throw IOException("目标路径越界：${target.path}")
            }
            val input = ctx.contentResolver.openInputStream(uri) ?: throw IOException("无法打开 $uri")
            input.use { src -> target.outputStream().use { dst -> src.copyTo(dst) } }
        } catch (e: Exception) {
            target.delete() // 部分写入的文件不留下
            throw e
        }
        return target.path
    }

    private fun displayName(uri: Uri): String? {
        if (uri.scheme == "file") return uri.lastPathSegment
        return try {
            ctx.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
        } catch (e: RuntimeException) {
            Log.w(TAG, "读取文件名失败：$uri", e)
            null
        } ?: uri.lastPathSegment
    }

    /** 用 createNewFile 原子占位；同名则在扩展名前追加 " (n)"。 */
    private fun reserve(dir: File, name: String): File {
        val dot = name.lastIndexOf('.')
        val (base, ext) = if (dot > 0) name.substring(0, dot) to name.substring(dot) else name to ""
        var n = 0
        while (true) {
            val f = File(dir, if (n == 0) name else "$base ($n)$ext")
            if (f.createNewFile()) return f
            n++
        }
    }

    fun outboxSizeBytes(): Long = outboxRoot.walkTopDown().filter { it.isFile }.sumOf { it.length() }

    /** 设置页「清理已分享文件的副本」。 */
    fun clear() {
        outboxRoot.deleteRecursively()
    }

    companion object {
        private const val TAG = "OutboxImporter"
        private const val MAX_NAME = 120

        /** 去掉路径分隔符与控制字符、首尾空白和点；空则 "file"；超长时截断主名并保留扩展名。 */
        fun sanitize(displayName: String?): String {
            val cleaned = (displayName ?: "")
                .filter { it != '/' && it != '\\' && !it.isISOControl() }
                .trim { it.isWhitespace() || it == '.' }
            if (cleaned.isEmpty()) return "file"
            if (cleaned.length <= MAX_NAME) return cleaned
            val dot = cleaned.lastIndexOf('.')
            val ext = if (dot > 0 && cleaned.length - dot <= 20) cleaned.substring(dot) else ""
            return cleaned.substring(0, MAX_NAME - ext.length) + ext
        }
    }
}
