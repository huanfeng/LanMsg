package name.apk.lanmsg.ui.chat

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.notify.VisibleConversation
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.ui.theme.LanMsgTheme
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ImageViewerVisibleTest {
    @get:Rule val compose = createComposeRule()

    @Before fun fakeCore() {
        ServiceLocator.coreFactory = { FakeCoreApi() }
        VisibleConversation.current.value = null
    }

    @After fun reset() {
        VisibleConversation.current.value = null
    }

    @Test fun image_viewer_keeps_conversation_visible_after_chat_clears_it() {
        var shown by mutableStateOf(true)
        compose.setContent {
            LanMsgTheme { if (shown) ImageViewerScreen("c1", "h1", onBack = {}) }
        }
        compose.waitForIdle()
        // 聊天页被移除时会清掉 current；看图期间该会话仍应视为可见
        VisibleConversation.current.value = null
        assertTrue(VisibleConversation.isVisible("c1"))
        assertFalse(VisibleConversation.isVisible("c2"))
        shown = false
        compose.waitForIdle()
        assertFalse(VisibleConversation.isVisible("c1"))
    }
}
