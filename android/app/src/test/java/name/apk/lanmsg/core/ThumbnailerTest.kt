package name.apk.lanmsg.core

import android.graphics.Bitmap
import android.graphics.Color
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ThumbnailerTest {
    @Test fun compress_to_limit_fits_32k() {
        val bmp = Bitmap.createBitmap(256, 256, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.RED) }
        val jpeg = Thumbnailer.compressToLimit(bmp, Thumbnailer.THUMB_MAX_BYTES)!!
        assertTrue(jpeg.size <= 32 * 1024)
        assertEquals(0xFF.toByte(), jpeg[0])
        assertEquals(0xD8.toByte(), jpeg[1])
    }

    @Test fun unknown_mime_returns_null() {
        assertNull(Thumbnailer(ApplicationProvider.getApplicationContext()).thumbnail("/nonexistent", "application/zip", 256))
    }
}
