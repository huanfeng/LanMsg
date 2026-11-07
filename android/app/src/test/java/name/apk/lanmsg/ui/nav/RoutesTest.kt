package name.apk.lanmsg.ui.nav

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RoutesTest {
    @Test fun builders_encode_ids() {
        assertEquals("chat/a%3A1", Routes.chat("a:1")); assertEquals("files", Routes.files())
        assertEquals("files?conv=c1", Routes.files("c1"))
        assertEquals("image/c%2F1/h", Routes.image("c/1", "h"))
        assertEquals("contact/u%201", Routes.contact("u 1"))
        assertEquals("text?path=%2Fa%20b%2Fx.txt&name=x.txt&mime=text%2Fplain", Routes.text("/a b/x.txt", "x.txt", "text/plain"))
        assertEquals("forward/aa,bb", Routes.forward(listOf("aa", "bb")))
    }
}
