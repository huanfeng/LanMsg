package name.apk.lanmsg.files

import android.content.Intent
import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ShareIntentParserTest {
    private val a = Uri.parse("content://x/a")
    private val b = Uri.parse("content://x/b")

    @Test fun send_single_stream() {
        val i = Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, a)
        assertEquals(listOf(a), ShareIntentParser.parse(i))
    }

    @Test fun send_multiple_streams() {
        val i = Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(Intent.EXTRA_STREAM, arrayListOf(a, b))
        assertEquals(listOf(a, b), ShareIntentParser.parse(i))
    }

    @Test fun send_text_only() {
        val i = Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_TEXT, "hi")
        assertTrue(ShareIntentParser.parse(i).isEmpty())
        assertEquals("hi", ShareText.extract(i))
    }

    @Test fun text_ignored_when_stream_present() {
        val i = Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, a).putExtra(Intent.EXTRA_TEXT, "cap")
        assertNull(ShareText.extract(i))
    }

    @Test fun other_action_empty() {
        val i = Intent(Intent.ACTION_VIEW).putExtra(Intent.EXTRA_STREAM, a).putExtra(Intent.EXTRA_TEXT, "t")
        assertTrue(ShareIntentParser.parse(i).isEmpty())
        assertNull(ShareText.extract(i))
    }
}
