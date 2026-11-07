package name.apk.lanmsg.files

import android.content.ContentProvider
import android.content.ContentUris
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.MediaStore
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.annotation.Config
import java.io.File

/** 内存版 MediaStore：记录插入的 ContentValues 及后续 update，openFile 写到临时文件。 */
class FakeMediaProvider : ContentProvider() {
    companion object {
        val rows = mutableListOf<ContentValues>()
        val files = mutableListOf<File>()
    }

    override fun onCreate() = true

    override fun insert(uri: Uri, values: ContentValues?): Uri? {
        rows += ContentValues(values)
        files += File.createTempFile("media", ".bin")
        return ContentUris.withAppendedId(uri, rows.size.toLong() - 1)
    }

    override fun update(uri: Uri, v: ContentValues?, s: String?, a: Array<String>?): Int {
        rows[ContentUris.parseId(uri).toInt()].putAll(v ?: return 0)
        return 1
    }

    override fun delete(uri: Uri, s: String?, a: Array<String>?): Int {
        rows[ContentUris.parseId(uri).toInt()].put("deleted", 1)
        return 1
    }

    override fun query(uri: Uri, p: Array<String>?, s: String?, a: Array<String>?, o: String?): Cursor =
        MatrixCursor(arrayOf("_id"))

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor =
        ParcelFileDescriptor.open(files[ContentUris.parseId(uri).toInt()], ParcelFileDescriptor.MODE_READ_WRITE)

    override fun getType(uri: Uri): String? = null
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [33])
class MediaStoreSaverTest {
    @get:Rule val tmp = TemporaryFolder()

    @Test fun save_image_inserts_into_pictures_lanmsg() = runTest {
        FakeMediaProvider.rows.clear(); FakeMediaProvider.files.clear()
        Robolectric.setupContentProvider(FakeMediaProvider::class.java, MediaStore.AUTHORITY)
        val src = tmp.newFile("a.jpg").also { it.writeText("img") }
        val uri = MediaStoreSaver(ApplicationProvider.getApplicationContext()).saveToGallery(src.path, "image/jpeg", "a.jpg")
        assertNotNull(uri)
        val row = FakeMediaProvider.rows.single()
        assertEquals("Pictures/LanMsg/", row.getAsString(MediaStore.MediaColumns.RELATIVE_PATH))
        assertEquals("a.jpg", row.getAsString(MediaStore.MediaColumns.DISPLAY_NAME))
        assertEquals(0, row.getAsInteger(MediaStore.MediaColumns.IS_PENDING))
        assertEquals("img", FakeMediaProvider.files.single().readText())
    }

    @Test fun save_video_goes_to_movies_and_other_mime_rejected() = runTest {
        FakeMediaProvider.rows.clear(); FakeMediaProvider.files.clear()
        Robolectric.setupContentProvider(FakeMediaProvider::class.java, MediaStore.AUTHORITY)
        val src = tmp.newFile("v.mp4").also { it.writeText("vid") }
        val saver = MediaStoreSaver(ApplicationProvider.getApplicationContext())
        assertNotNull(saver.saveToGallery(src.path, "video/mp4", "v.mp4"))
        assertEquals("Movies/LanMsg/", FakeMediaProvider.rows.single().getAsString(MediaStore.MediaColumns.RELATIVE_PATH))
        assertNull(saver.saveToGallery(src.path, "application/pdf", "x.pdf"))
    }
}
