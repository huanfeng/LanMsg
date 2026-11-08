package name.apk.lanmsg.ui.nav

import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RoutesTest {
    @Test fun builders_encode_ids() {
        assertEquals("chat/a%3A1", Routes.chat("a:1")); assertEquals("files", Routes.files())
        assertEquals("files?conv=c1", Routes.files("c1"))
        assertEquals("station/s%2F1", Routes.station("s/1"))
        assertEquals("image/c%2F1/h", Routes.image("c/1", "h"))
        assertEquals("contact/u%201", Routes.contact("u 1"))
        assertEquals("text?path=%2Fa%20b%2Fx.txt&name=x.txt&mime=text%2Fplain", Routes.text("/a b/x.txt", "x.txt", "text/plain"))
        assertEquals("forward/aa,bb", Routes.forward(listOf("aa", "bb")))
    }

    @Test fun text_route_round_trips_special_characters() {
        // 路径里的中文、?、&、%、# 经编码放进路由，再按导航库的方式解码，必须还原
        val paths = listOf("/data/文件/报告 final.txt", "/a?b=c&d=e/x.txt", "/50%/100%25/x.txt", "/a#b/c+d.txt")
        for (p in paths) {
            val route = Routes.text(p, p, "text/plain")
            val query = Uri.parse(route).encodedQuery!!
            val decoded = query.split('&').associate { it.substringBefore('=') to Uri.decode(it.substringAfter('=')) }
            assertEquals(p, decoded["path"])
            assertEquals(p, decoded["name"])
            assertEquals("text/plain", decoded["mime"])
        }
    }
}
