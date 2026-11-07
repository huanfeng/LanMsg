package name.apk.lanmsg.ui.preview

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import name.apk.lanmsg.ui.theme.LanMsgTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class TextPreviewTest {
    @get:Rule val compose = createComposeRule()

    @Test fun split_lines_strips_cr_and_keeps_empty_lines() {
        assertEquals(listOf("a", "", "b", ""), TextPreviewViewModel.splitLines("a\r\n\nb\n"))
    }

    @Test fun ready_shows_lines_and_truncation_hint() {
        compose.setContent {
            LanMsgTheme {
                TextPreviewContent(
                    TextPreviewViewModel.UiState.Ready(listOf("hello world", "second"), truncated = true),
                    name = "a.txt",
                    onBack = {},
                    onOpenWith = {},
                )
            }
        }
        compose.onNodeWithText("hello world").assertExists()
        compose.onNodeWithText("仅显示前 1 MB").assertExists()
    }

    @Test fun complete_file_has_no_truncation_hint() {
        compose.setContent {
            LanMsgTheme {
                TextPreviewContent(
                    TextPreviewViewModel.UiState.Ready(listOf("only"), truncated = false),
                    name = "a.txt",
                    onBack = {},
                    onOpenWith = {},
                )
            }
        }
        compose.onNodeWithText("仅显示前 1 MB").assertDoesNotExist()
    }

    @Test fun failed_shows_message() {
        compose.setContent {
            LanMsgTheme {
                TextPreviewContent(TextPreviewViewModel.UiState.Failed, name = "a.txt", onBack = {}, onOpenWith = {})
            }
        }
        compose.onNodeWithText("无法预览这个文件").assertExists()
    }
}
