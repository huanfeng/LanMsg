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
import java.util.Calendar
import java.util.Date
import java.util.Locale

/** 把外部来源（content:// 等）的文件落地成 core 可读的本地路径。ViewModel 依赖此窄接口便于测试。 */
interface Importer {
    suspend fun import(uris: List<Uri>): List<String>
}

/** 设置页清理发件箱副本用的窄接口，便于 ViewModel 测试注入内存实现。 */
interface OutboxStore {
    fun outboxSizeBytes(): Long

    /** 发件箱根目录的绝对路径，供向 core 查询“哪些副本已全部送达”。 */
    fun rootPath(): String

    /**
     * 清理副本：[deliveredPaths]（core 判定已全部送达）里位于发件箱内的普通文件，
     * 以及整天都早于 [olderThanMillis] 的日期子目录。返回释放的字节数。
     */
    fun clear(deliveredPaths: Set<String>, olderThanMillis: Long): Long
}

/**
 * 发件箱：把 content:// URI 复制到 `outbox/<yyyyMMdd>/` 下的普通文件。
 * 文件名来自外部应用，不可信：先 [sanitize]，写入前再断言目标路径位于 outboxRoot 之内。
 */
class OutboxImporter(
    private val ctx: Context,
    private val outboxRoot: File = File(ctx.filesDir, "outbox"),
) : Importer, OutboxStore {
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

    override fun outboxSizeBytes(): Long = outboxRoot.walkTopDown().filter { it.isFile }.sumOf { it.length() }

    /** 规范化路径：与 [import] 交给 core 的路径同一形态（经过符号链接时前缀才对得上）。 */
    override fun rootPath(): String = outboxRoot.canonicalPath

    /**
     * 设置页「清理已分享文件的副本」。core 发送文件只记录副本路径，对端之后下载仍读该路径，
     * 所以只清理两类：core 判定已全部送达的文件；早于 cutoff 的日期目录（yyyyMMdd，该日结束时刻 <= cutoff）。
     * 非日期命名或解析失败的目录、符号链接、发件箱之外的路径一律不动。
     */
    override fun clear(deliveredPaths: Set<String>, olderThanMillis: Long): Long {
        var freed = clearDelivered(deliveredPaths)
        freed += clearOlderThan(olderThanMillis)
        return freed
    }

    /** 删除已送达的单个文件；删空的日期目录一并移除。 */
    private fun clearDelivered(paths: Set<String>): Long {
        val root = outboxRoot.canonicalFile
        var freed = 0L
        for (p in paths) {
            val f = File(p)
            if (java.nio.file.Files.isSymbolicLink(f.toPath()) || !f.isFile) continue
            // 路径必须真实位于发件箱内（防 core 之外传入的路径逃逸）
            if (!f.canonicalPath.startsWith(root.path + File.separator)) continue
            val size = f.length()
            val parent = f.parentFile
            if (f.delete()) {
                freed += size
                if (parent != null && parent != root && DATE_DIR.matches(parent.name) && parent.listFiles().isNullOrEmpty()) parent.delete()
            }
        }
        return freed
    }

    private fun clearOlderThan(cutoffMillis: Long): Long {
        val fmt = SimpleDateFormat("yyyyMMdd", Locale.ROOT).apply { isLenient = false }
        var freed = 0L
        for (dir in outboxRoot.listFiles().orEmpty()) {
            if (!dir.isDirectory || java.nio.file.Files.isSymbolicLink(dir.toPath()) || !DATE_DIR.matches(dir.name)) continue
            val dayStart = try {
                fmt.parse(dir.name)
            } catch (e: java.text.ParseException) {
                Log.w(TAG, "Skipped unparsable outbox directory: ${dir.name}", e)
                null
            } ?: continue
            val dayEnd = Calendar.getInstance().apply { time = dayStart; add(Calendar.DAY_OF_MONTH, 1) }.timeInMillis
            if (dayEnd > cutoffMillis) continue
            val size = dir.walkTopDown().filter { it.isFile }.sumOf { it.length() }
            if (dir.deleteRecursively()) freed += size
        }
        return freed
    }

    companion object {
        private const val TAG = "OutboxImporter"
        private val DATE_DIR = Regex("""\d{8}""")
        private const val MAX_NAME = 120
        private const val MAX_NAME_BYTES = 200 // 文件系统上限 255，给重名后缀留余量

        private fun utf8Len(s: String) = s.toByteArray(Charsets.UTF_8).size

        /**
         * 双向文本控制符（可用于伪装扩展名）与不可见零宽字符（ZWSP/WJ/BOM，可造出视觉相同的文件名）；
         * ZWJ/ZWNJ 与 emoji tag 序列（U+E0020–E007F）要保留，否则会破坏 emoji 序列与部分文字。
         */
        private fun isBidiControl(cp: Int) =
            cp in 0x202A..0x202E || cp in 0x2066..0x2069 || cp == 0x200E || cp == 0x200F || cp == 0x061C ||
                cp == 0x200B || cp == 0x2060 || cp == 0xFEFF

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
