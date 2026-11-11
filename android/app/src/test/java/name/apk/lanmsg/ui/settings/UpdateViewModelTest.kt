@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.settings

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.InstallChannel
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.UpdateAsset
import uniffi.lanmsg_api.UpdateCheckView
import uniffi.lanmsg_api.UpdateInfo

/** UpdateCheckView 测试样本。 */
private fun view(update: UpdateInfo?) = UpdateCheckView(
    update = update,
    current = "0.1.0",
    channel = InstallChannel.MANUAL,
    releaseUrl = "https://example.com/releases/tag/v0.2.0",
)

/** UpdateInfo 测试样本。 */
private fun info(version: String) = UpdateInfo(
    version = version,
    notes = "bug fixes",
    publishedAt = 1000L,
    assets = listOf(UpdateAsset("lanmsg-android-arm64.apk", 42uL, "https://example.com/a.apk")),
)

class UpdateViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun vm(fake: FakeCoreApi) = UpdateViewModel(fake, Dispatchers.Unconfined)

    @Test fun dialog_when_update_available() = runTest {
        val fake = FakeCoreApi()
        fake.handler = { _ -> Response.UpdateCheck(view(info("0.2.0"))) }
        val v = vm(fake)
        v.checkUpdate()
        advanceUntilIdle()
        assertEquals("0.2.0", v.state.value.view?.update?.version)
        assertFalse(v.state.value.upToDate)
        assertFalse(v.state.value.failed)
    }

    @Test fun up_to_date_shows_snackbar_state() = runTest {
        val fake = FakeCoreApi()
        fake.handler = { _ -> Response.UpdateCheck(view(null)) }
        val v = vm(fake)
        v.checkUpdate()
        advanceUntilIdle()
        assertTrue(v.state.value.upToDate)
        assertNull(v.state.value.view)
        v.shown()
        assertFalse(v.state.value.upToDate)
    }

    @Test fun failed_when_request_throws() = runTest {
        val fake = FakeCoreApi()
        fake.handler = { throw ApiException(ErrorCode.INTERNAL, "offline") }
        val v = vm(fake)
        v.checkUpdate()
        advanceUntilIdle()
        assertTrue(v.state.value.failed)
        assertNull(v.state.value.view)
        v.shown()
        assertFalse(v.state.value.failed)
    }
}
