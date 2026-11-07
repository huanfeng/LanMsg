package name.apk.lanmsg.files

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import kotlin.coroutines.cancellation.CancellationException
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
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // provider 可能抛 IOException、SecurityException 乃至 IllegalStateException 等，一律跳过该项
                Log.w(TAG, "Import failed, skipped: $uri", e)
                null
            }
        }
    }

    private fun copyOne(uri: Uri, dir: File, root: File): String {
        // 只接受外部 content://；file:// 或本应用自己的 FileProvider 会让我们以自己的身份读私有文件
        if (uri.scheme != "content") throw IOException("Unsupported URI scheme: ${uri.scheme}")
        if (uri.authority == FileOpener.AUTHORITY) throw IOException("Refusing to import own FileProvider file")
        if (!dir.isDirectory && !dir.mkdirs()) throw IOException("Cannot create directory $dir")
        val name = sanitize(displayName(uri))
        val target = reserve(dir, name, root)
        try {
            val input = ctx.contentResolver.openInputStream(uri) ?: throw IOException("Cannot open $uri")
            input.use { src -> target.outputStream().use { dst -> src.copyTo(dst) } }
        } catch (e: Exception) {
            target.delete() // 部分写入的文件不留下
            throw e
        }
        return target.path
    }

    private fun displayName(uri: Uri): String? {
        return try {
            ctx.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
        } catch (e: RuntimeException) {
            Log.w(TAG, "Failed to read display name: $uri", e)
            null
        } ?: uri.lastPathSegment
    }

    /** 用 createNewFile 原子占位；同名则在扩展名前追加 " (n)"。候选先校验位于 outbox 内（防目录穿越的第二道保险）再占位。 */
    private fun reserve(dir: File, name: String, root: File): File {
        val dot = name.lastIndexOf('.')
        val (base, ext) = if (dot > 0) name.substring(0, dot) to name.substring(dot) else name to ""
        var n = 0
        while (true) {
            val f = File(dir, if (n == 0) name else "$base ($n)$ext")
            if (!f.canonicalPath.startsWith(root.path + File.separator)) throw IOException("Target path escapes outbox: ${f.path}")
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
        private const val MAX_NAME_BYTES = 200 // 文件系统上限 255，给重名后缀留余量

        private fun utf8Len(s: String) = s.toByteArray(Charsets.UTF_8).size

        /** 双向文本控制符（可用于伪装扩展名）；ZWJ/ZWNJ 等其他格式字符要保留，否则会破坏 emoji 序列与部分文字。 */
        private fun isBidiControl(cp: Int) =
            cp in 0x202A..0x202E || cp in 0x2066..0x2069 || cp == 0x200E || cp == 0x200F || cp == 0x061C

        /**
         * 去掉路径分隔符、控制字符与双向控制符（如 U+202E，按码点判断；保留 ZWJ/ZWNJ）、首尾空白和点；空则 "file"；
         * 超过 120 字符或 200 字节时按码点截断主名（不拆代理对）并保留扩展名。
         */
        fun sanitize(displayName: String?): String {
            val cleaned = (displayName ?: "")
                .codePoints().toArray()
                .filter { it != '/'.code && it != '\\'.code && !Character.isISOControl(it) && !isBidiControl(it) }
                .joinToString("") { String(Character.toChars(it)) }
                .trim { it.isWhitespace() || it == '.' }
            if (cleaned.isEmpty()) return "file"
            if (cleaned.length <= MAX_NAME && utf8Len(cleaned) <= MAX_NAME_BYTES) return cleaned
            val dot = cleaned.lastIndexOf('.')
            val ext = if (dot > 0 && cleaned.length - dot <= 20 && utf8Len(cleaned.substring(dot)) <= 40) cleaned.substring(dot) else ""
            val base = cleaned.substring(0, if (ext.isEmpty()) cleaned.length else dot)
            var chars = ext.length
            var bytes = utf8Len(ext)
            var end = 0
            while (end < base.length) {
                val cp = base.codePointAt(end)
                val n = Character.charCount(cp)
                val b = String(Character.toChars(cp)).let(::utf8Len)
                if (chars + n > MAX_NAME || bytes + b > MAX_NAME_BYTES) break
                chars += n
                bytes += b
                end += n
            }
            val head = base.substring(0, end).trimEnd { it.isWhitespace() || it == '.' }
            return if (head.isEmpty()) "file$ext" else head + ext
        }
    }
}
