package name.apk.lanmsg

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityDeepLinkTest {
    @Test fun allows_requests_and_chat_with_conversation() {
        assertTrue(MainActivity.isAllowedDeepLink("requests"))
        assertTrue(MainActivity.isAllowedDeepLink("chat/c1"))
        assertTrue(MainActivity.isAllowedDeepLink("chat/a%20b"))
    }

    @Test fun rejects_other_pages_and_malformed_chat() {
        assertFalse(MainActivity.isAllowedDeepLink("settings"))
        assertFalse(MainActivity.isAllowedDeepLink("pair/host"))
        assertFalse(MainActivity.isAllowedDeepLink("forward/x"))
        assertFalse(MainActivity.isAllowedDeepLink("chat/"))
        assertFalse(MainActivity.isAllowedDeepLink("chat/a/b"))
        assertFalse(MainActivity.isAllowedDeepLink("chat/a%2Fb"))
        assertFalse(MainActivity.isAllowedDeepLink(""))
    }
}
