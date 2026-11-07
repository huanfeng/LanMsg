package name.apk.lanmsg.files

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class FilePreviewTest {
    @get:Rule val tmp = TemporaryFolder()

    private val octet = "application/octet-stream"

    @Test fun kind_by_mime() {
        assertEquals(PreviewKind.IMAGE, FilePreview.kind("image/png", "x"))
        assertEquals(PreviewKind.IMAGE, FilePreview.kind("IMAGE/JPEG", "x"))
        assertEquals(PreviewKind.IMAGE, FilePreview.kind("image/bmp", "x"))
        assertEquals(PreviewKind.TEXT, FilePreview.kind("text/plain; charset=utf-8", "x"))
        assertEquals(PreviewKind.TEXT, FilePreview.kind("text/x-unknown", "x"))
        assertNull(FilePreview.kind("application/pdf", "x.pdf"))
        assertNull(FilePreview.kind("video/mp4", "x.mp4"))
    }

    @Test fun kind_falls_back_to_extension() {
        assertEquals(PreviewKind.IMAGE, FilePreview.kind(octet, "a.WebP"))
        assertEquals(PreviewKind.IMAGE, FilePreview.kind(octet, "a.bmp"))
        for (ext in listOf("txt", "md", "log", "csv", "json", "xml", "yaml", "yml", "toml", "ini", "conf", "js", "ts", "py", "rs", "kt", "java", "c", "h", "cpp", "go", "sh", "bat", "ps1", "html", "css", "sql")) {
            assertEquals(ext, PreviewKind.TEXT, FilePreview.kind(octet, "f.$ext"))
        }
        assertNull(FilePreview.kind(octet, "noext"))
        assertNull(FilePreview.kind(octet, "a.exe"))
    }

    @Test fun nul_in_head_is_binary() {
        assertNull(FilePreview.decodeText(byteArrayOf(97, 0, 98), false))
        // 8 KiB 之后的 NUL 不算
        val late = ByteArray(FilePreview.BINARY_SNIFF_BYTES + 1) { 97 }
        late[FilePreview.BINARY_SNIFF_BYTES] = 0
        assertNotNull(FilePreview.decodeText(late, false))
    }

    @Test fun bom_is_stripped() {
        val t = FilePreview.decodeText(byteArrayOf(0xEF.toByte(), 0xBB.toByte(), 0xBF.toByte(), 'h'.code.toByte(), 'i'.code.toByte()), false)!!
        assertEquals("hi", t.text)
        assertFalse(t.truncated)
    }

    @Test fun invalid_utf8_becomes_replacement_char() {
        val t = FilePreview.decodeText(byteArrayOf(97, 0xFF.toByte(), 98), false)!!
        assertEquals("a�b", t.text)
    }

    @Test fun truncation_drops_split_char_but_keeps_real_errors() {
        // "你" = E4 BD A0，只剩前两个字节：丢弃，不产生替换符
        val cut = FilePreview.decodeText(byteArrayOf(97, 98, 0xE4.toByte(), 0xBD.toByte()), true)!!
        assertEquals("ab", cut.text)
        assertTrue(cut.truncated)
        // 完整的多字节字符保留
        val whole = FilePreview.decodeText("ab你".toByteArray(), true)!!
        assertEquals("ab你", whole.text)
        // 中间的非法字节仍然替换
        assertEquals("a�b", FilePreview.decodeText(byteArrayOf(97, 0xFF.toByte(), 98), true)!!.text)
    }

    @Test fun read_truncates_at_one_mib_and_flags_it() {
        val big = tmp.newFile("big.txt").apply { writeBytes(ByteArray(FilePreview.TEXT_MAX_BYTES + 10) { 'x'.code.toByte() }) }
        val t = FilePreview.readText(big)!!
        assertEquals(FilePreview.TEXT_MAX_BYTES, t.text.length)
        assertTrue(t.truncated)

        val exact = tmp.newFile("exact.txt").apply { writeBytes(ByteArray(FilePreview.TEXT_MAX_BYTES) { 'x'.code.toByte() }) }
        assertFalse(FilePreview.readText(exact)!!.truncated)

        val bin = tmp.newFile("b.bin").apply { writeBytes(byteArrayOf(0x7f, 'E'.code.toByte(), 0, 0)) }
        assertNull(FilePreview.readText(bin))
    }
}
