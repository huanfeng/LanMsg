package name.apk.lanmsg.files

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import java.io.File
import java.io.FileNotFoundException

/** 测试用内容提供者：路径段 = 内容；`name` 查询参数 = DISPLAY_NAME；`fail` 参数让打开失败。 */
class FakeSharedProvider : ContentProvider() {
    override fun onCreate() = true

    override fun query(uri: Uri, p: Array<String>?, s: String?, a: Array<String>?, o: String?): Cursor {
        val c = MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME))
        c.addRow(arrayOf(uri.getQueryParameter("name")))
        return c
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (uri.getQueryParameter("fail") != null) throw FileNotFoundException("boom")
        if (uri.getQueryParameter("crash") != null) throw IllegalStateException("provider crashed")
        val f = File.createTempFile("share", ".bin")
        f.writeText(uri.lastPathSegment.orEmpty())
        return ParcelFileDescriptor.open(f, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri): String? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, s: String?, a: Array<String>?) = 0
    override fun update(uri: Uri, v: ContentValues?, s: String?, a: Array<String>?) = 0
}

@RunWith(AndroidJUnit4::class)
class OutboxImporterTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun importer(): OutboxImporter {
        Robolectric.setupContentProvider(FakeSharedProvider::class.java, "share.test")
        return OutboxImporter(ApplicationProvider.getApplicationContext(), File(tmp.root, "outbox"))
    }

    private fun uri(content: String, name: String?, extra: String = ""): Uri =
        Uri.parse("content://share.test/$content?" + (if (name != null) "name=${Uri.encode(name)}" else "x=1") + extra)

    @Test fun import_sanitizes_display_name() {
        assertEquals("evil.txt", OutboxImporter.sanitize("../../evil.txt"))
        assertEquals("ab.txt", OutboxImporter.sanitize("a/b.txt"))
        assertEquals("ab.txt", OutboxImporter.sanitize("a\\b.txt"))
        assertEquals("file", OutboxImporter.sanitize("  ..  "))
        assertEquals("file", OutboxImporter.sanitize(null))
        assertTrue(OutboxImporter.sanitize("a".repeat(300) + ".jpg").let { it.length <= 120 && it.endsWith(".jpg") })
        assertEquals("file", OutboxImporter.sanitize("\u202E\u200F"))
    }

    @Test fun long_cjk_name_is_truncated_by_bytes_and_imports() = runTest {
        val name = "汉".repeat(300) + ".jpg"
        val s = OutboxImporter.sanitize(name)
        assertTrue(s.endsWith(".jpg"))
        assertTrue(s.toByteArray(Charsets.UTF_8).size <= 200)
        val paths = importer().import(listOf(uri("x", name)))
        assertEquals(1, paths.size)
        assertEquals(s, File(paths.single()).name)
    }

    @Test fun truncation_does_not_split_surrogate_pairs() {
        val s = OutboxImporter.sanitize("\uD83D\uDE00".repeat(100) + ".png")
        assertTrue(s.endsWith(".png"))
        assertTrue(s.dropLast(4).chunked(2).all { it == "\uD83D\uDE00" })
    }

    @Test fun sanitize_strips_format_control_characters() {
        assertEquals("exe.txt", OutboxImporter.sanitize("\u202Eexe.txt"))
        assertEquals("ab.txt", OutboxImporter.sanitize("a\u200Eb\u200F.txt"))
    }

    @Test fun non_content_and_own_provider_uris_are_skipped() = runTest {
        val imp = importer()
        val secret = File(tmp.root, "secret.txt").apply { writeText("s") }
        val paths = imp.import(
            listOf(
                Uri.fromFile(secret),
                Uri.parse("content://${FileOpener.AUTHORITY}/lanmsg/x"),
                uri("ok", "ok.bin"),
            ),
        )
        assertEquals(listOf("ok.bin"), paths.map { File(it).name })
    }

    @Test fun provider_runtime_exception_is_skipped() = runTest {
        val imp = importer()
        val paths = imp.import(listOf(uri("bad", "bad.bin", "&crash=1"), uri("ok", "ok.bin")))
        assertEquals(listOf("ok.bin"), paths.map { File(it).name })
    }

    @Test fun import_dedupes_same_name() = runTest {
        val imp = importer()
        val paths = imp.import(listOf(uri("one", "a.jpg"), uri("two", "a.jpg")))
        assertEquals(2, paths.size)
        assertEquals(listOf("a.jpg", "a (1).jpg"), paths.map { File(it).name })
        assertEquals(listOf("one", "two"), paths.map { File(it).readText() })
        val root = File(tmp.root, "outbox").canonicalPath
        assertTrue(paths.all { File(it).canonicalPath.startsWith(root + File.separator) })
    }

    @Test fun malicious_name_stays_inside_outbox() = runTest {
        val imp = importer()
        val paths = imp.import(listOf(uri("x", "../../../escape.txt")))
        assertEquals("escape.txt", File(paths.single()).name)
        assertTrue(File(paths.single()).canonicalPath.startsWith(File(tmp.root, "outbox").canonicalPath + File.separator))
        assertFalse(File(tmp.root, "escape.txt").exists())
    }

    @Test fun failed_uri_is_skipped_and_leaves_no_partial_file() = runTest {
        val imp = importer()
        val paths = imp.import(listOf(uri("bad", "bad.bin", "&fail=1"), uri("ok", "ok.bin")))
        assertEquals(listOf("ok.bin"), paths.map { File(it).name })
        val files = File(tmp.root, "outbox").walkTopDown().filter { it.isFile }.map { it.name }.toList()
        assertEquals(listOf("ok.bin"), files)
    }

    @Test fun size_and_clear() = runTest {
        val imp = importer()
        imp.import(listOf(uri("abcd", "a.txt")))
        assertEquals(4L, imp.outboxSizeBytes())
        imp.clear()
        assertEquals(0L, imp.outboxSizeBytes())
    }
}
