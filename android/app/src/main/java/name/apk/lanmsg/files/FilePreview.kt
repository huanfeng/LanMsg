package name.apk.lanmsg.files

import java.io.File

/** 可应用内预览的类别。 */
enum class PreviewKind { IMAGE, TEXT }

/** 文本预览内容；[truncated] 表示文件超过 1 MiB，只取了前面一段。 */
data class TextPreview(val text: String, val truncated: Boolean)

/** 应用内预览的判定与文本读取；均为纯函数，不依赖 Android 框架。 */
object FilePreview {
    /** 文本预览最多读取的字节数（1 MiB）。 */
    const val TEXT_MAX_BYTES = 1 shl 20

    /** 判定二进制时检查的文件头长度。 */
    const val BINARY_SNIFF_BYTES = 8 * 1024

    private val textExts = setOf(
        "txt", "md", "log", "csv", "json", "xml", "yaml", "yml", "toml", "ini", "conf", "js", "ts",
        "py", "rs", "kt", "java", "c", "h", "cpp", "go", "sh", "bat", "ps1", "html", "css", "sql",
    )
    private val imageExts = setOf("png", "jpg", "jpeg", "gif", "webp", "bmp")
    private val imageMimes = setOf("image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp")

    /** 按 MIME 优先、扩展名兜底判定；不能预览返回 null。 */
    fun kind(mime: String, name: String): PreviewKind? {
        val m = mime.substringBefore(';').trim().lowercase()
        if (m in imageMimes) return PreviewKind.IMAGE
        if (m.startsWith("text/")) return PreviewKind.TEXT
        val ext = name.substringAfterLast('.', "").lowercase()
        return when (ext) {
            in imageExts -> PreviewKind.IMAGE
            in textExts -> PreviewKind.TEXT
            else -> null
        }
    }

    /**
     * 把读到的字节解成预览文本；文件头 8 KiB 内有 NUL 视为二进制，返回 null。
     * [truncated] 表示 [bytes] 只是文件的前一段，此时丢弃末尾被截断的不完整 UTF-8 序列。
     */
    fun decodeText(bytes: ByteArray, truncated: Boolean): TextPreview? {
        for (i in 0 until minOf(bytes.size, BINARY_SNIFF_BYTES)) if (bytes[i] == 0.toByte()) return null
        val start = if (hasBom(bytes)) 3 else 0
        val end = if (truncated) completeUtf8End(bytes) else bytes.size
        // String(…, UTF_8) 把非法字节替换为 U+FFFD
        return TextPreview(String(bytes, start, maxOf(end - start, 0), Charsets.UTF_8), truncated)
    }

    /** 读取文件前 1 MiB 并解码；二进制返回 null。 */
    fun readText(file: File): TextPreview? {
        val buf = ByteArray(TEXT_MAX_BYTES + 1)
        var n = 0
        file.inputStream().use { input ->
            while (n < buf.size) {
                val r = input.read(buf, n, buf.size - n)
                if (r < 0) break
                n += r
            }
        }
        val truncated = n > TEXT_MAX_BYTES
        return decodeText(buf.copyOf(minOf(n, TEXT_MAX_BYTES)), truncated)
    }

    private fun hasBom(b: ByteArray) =
        b.size >= 3 && b[0] == 0xEF.toByte() && b[1] == 0xBB.toByte() && b[2] == 0xBF.toByte()

    /** 截断位置若落在多字节字符中间，返回该字符的起点；否则返回 bytes.size。 */
    private fun completeUtf8End(b: ByteArray): Int {
        var i = b.size - 1
        var back = 0
        while (i >= 0 && back < 3) {
            val v = b[i].toInt() and 0xFF
            if (v and 0xC0 != 0x80) {
                val need = when {
                    v >= 0xF0 -> 4
                    v >= 0xE0 -> 3
                    v >= 0xC0 -> 2
                    else -> 1
                }
                return if (need > back + 1) i else b.size
            }
            i--
            back++
        }
        return b.size
    }
}
