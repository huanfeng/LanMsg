package name.apk.lanmsg.ui.web

import org.junit.Assert.assertEquals
import org.junit.Test

class WebTextTest {
    @Test fun group_code_4_4() {
        assertEquals("K7QM 3XAP", groupCode("K7QM3XAP"))
        assertEquals("abc", groupCode("abc"))
    }

    @Test fun browser_summary_cases() {
        assertEquals(
            "Chrome (Android)",
            browserSummary("Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/120.0 Mobile Safari/537.36"),
        )
        assertEquals(
            "Safari (iPhone)",
            browserSummary("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Version/17.0 Mobile Safari/604.1"),
        )
        assertEquals("Edge (Windows)", browserSummary("Mozilla/5.0 (Windows NT 10.0) Chrome/120.0 Safari/537.36 Edg/120.0"))
        assertEquals("Firefox (Linux)", browserSummary("Mozilla/5.0 (X11; Linux x86_64; rv:120.0) Gecko/20100101 Firefox/120.0"))
        assertEquals("curl/8.0", browserSummary("curl/8.0"))
    }
}
