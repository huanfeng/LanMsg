package name.apk.lanmsg.ui.chat

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.ui.theme.LanMsgTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.MessageBodyView
import uniffi.lanmsg_api.MessageState

@RunWith(AndroidJUnit4::class)
class ChatScreenTest {
    @get:Rule val compose = createComposeRule()

    @Test fun recalled_message_shows_placeholder_text() {
        val recalled = Fixtures.recalledMsg("m1", mine = false, authorName = "对端")
        compose.setContent {
            LanMsgTheme {
                ChatContent(
                    state = ChatViewModel.UiState(title = "对端", kind = ConvKind.DM, peerOnline = true, messages = listOf(recalled)),
                    canRecall = { false },
                    actions = ChatActions(onAttachMedia = {}, onAttachFile = {}),
                )
            }
        }
        compose.onNodeWithText("对端 撤回了一条消息").assertExists()
    }

    @Test fun own_recalled_message_shows_own_placeholder() {
        val recalled = Fixtures.recalledMsg("m1", mine = true, authorName = "小李")
        compose.setContent {
            LanMsgTheme {
                ChatContent(
                    state = ChatViewModel.UiState(title = "对端", kind = ConvKind.DM, peerOnline = true, messages = listOf(recalled)),
                    canRecall = { false },
                    actions = ChatActions(onAttachMedia = {}, onAttachFile = {}),
                )
            }
        }
        compose.onNodeWithText("你撤回了一条消息").assertExists()
    }

    @Test fun self_devices_conversation_shows_source_device_name() {
        compose.setContent {
            LanMsgTheme {
                ChatContent(
                    state = ChatViewModel.UiState(kind = ConvKind.SELF_DEVICES, messages = listOf(Fixtures.textMsg("m1", text = "你好"))),
                    canRecall = { false },
                    actions = ChatActions(onAttachMedia = {}, onAttachFile = {}),
                )
            }
        }
        compose.onNodeWithText("来自 设备一").assertExists()
        compose.onNodeWithText("你好").assertExists()
    }

    @Test fun remote_file_card_shows_not_downloaded_and_click_downloads() {
        val item = (Fixtures.fileMsg("m1", "h1", BlobState.REMOTE).body as MessageBodyView.Files).items[0]
        var downloaded: String? = null
        compose.setContent {
            LanMsgTheme {
                FileCard(item, null, content = Color.Black, muted = Color.Gray,
                    onDownload = { downloaded = it }, onCancel = {}, onOpen = {})
            }
        }
        compose.onNodeWithText("未下载").assertExists()
        compose.onNodeWithText("未下载").performClick()
        assertEquals("h1", downloaded)
    }
}
