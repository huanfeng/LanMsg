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
class PairJoinScreenTest {
    @get:Rule val compose = createComposeRule()

    @Test fun pair_join_without_camera_permission_offers_sas() {
        var switched = 0
        compose.setContent {
            LanMsgTheme {
                PairJoinContent(
                    state = PairViewModel.UiState(cameraGranted = false, joinMode = PairViewModel.JoinMode.SCAN),
                    onSwitchToSas = { switched++ },
                    onGrantCamera = {},
                    onScanned = {},
                    onPick = {},
                    onConfirm = {},
                    onRetry = {},
                )
            }
        }
        compose.onNodeWithText("需要相机权限才能扫码").assertExists()
        compose.onNodeWithText("改用数字比对").performClick()
        assertEquals(1, switched)
    }

    @Test fun show_sas_renders_grouped_code_and_both_buttons() {
        val confirmed = mutableListOf<Boolean>()
        compose.setContent {
            LanMsgTheme {
                PairJoinContent(
                    state = PairViewModel.UiState(session = "j1", pairState = PairStateDto.ShowSas("123456", "乙机", "phone", "a1b2c3d4")),
                    onSwitchToSas = {},
                    onGrantCamera = {},
                    onScanned = {},
                    onPick = {},
                    onConfirm = { confirmed += it },
                    onRetry = {},
                )
            }
        }
        compose.onNodeWithText("123 456").assertExists()
        compose.onNodeWithText("不一致").performClick()
        compose.onNodeWithText("一致").performClick()
        assertEquals(listOf(false, true), confirmed)
    }

    @Test fun pick_mode_empty_list_shows_guidance_and_refresh_triggers_reload() {
        var refreshed = 0
        compose.setContent {
            LanMsgTheme {
                PairJoinContent(
                    state = PairViewModel.UiState(joinMode = PairViewModel.JoinMode.PICK, candidates = emptyList()),
                    onSwitchToSas = {},
                    onGrantCamera = {},
                    onScanned = {},
                    onPick = {},
                    onConfirm = {},
                    onRetry = {},
                    onRefresh = { refreshed++ },
                )
            }
        }
        compose.onNodeWithText("还没有发现可加入的设备", substring = true).assertExists()
        compose.onNodeWithText("刷新").performClick()
        assertEquals(1, refreshed)
    }
}
