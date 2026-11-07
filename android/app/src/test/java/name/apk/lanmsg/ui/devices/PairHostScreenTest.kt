package name.apk.lanmsg.ui.devices

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import name.apk.lanmsg.ui.theme.LanMsgTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.PairStateDto

@RunWith(AndroidJUnit4::class)
class PairHostScreenTest {
    @get:Rule val compose = createComposeRule()

    /** 二维码页上对端按候选设备以数字比对加入：页面切到数字确认。 */
    @Test fun qr_page_switches_to_sas_confirmation_when_peer_joins_by_digits() {
        val confirmed = mutableListOf<Boolean>()
        compose.setContent {
            LanMsgTheme {
                PairHostContent(
                    state = PairViewModel.UiState(
                        session = "s1",
                        qrUri = "lanmsg://pair?s=x",
                        pairState = PairStateDto.ShowSas("123456", "我的电脑", "desktop", "a1b2c3d4"),
                    ),
                    onBack = {},
                    onSwitchToSas = {},
                    onConfirm = { confirmed += it },
                    onRetry = {},
                )
            }
        }
        compose.onNodeWithText("123 456").assertExists()
        compose.onNodeWithText("对方自称：我的电脑（台式机）").assertExists()
        compose.onNodeWithText("指纹 a1b2c3d4").assertExists()
        compose.onNodeWithText("请到该设备上核对数字").assertExists()
        compose.onNodeWithText("一致").performClick()
        assertEquals(listOf(true), confirmed)
    }
}
