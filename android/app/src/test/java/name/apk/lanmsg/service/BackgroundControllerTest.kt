@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.service

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.CoreHolder
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.BackgroundMode
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SettingsDto

@RunWith(AndroidJUnit4::class)
class BackgroundControllerTest {
    private class RecordingActions : ServiceActions {
        val service = mutableListOf<Boolean>()
        val worker = mutableListOf<Boolean>()

        override fun setServiceRunning(run: Boolean) {
            service += run
        }

        override fun setWorkerScheduled(scheduled: Boolean) {
            worker += scheduled
        }
    }

    private fun settings(mode: BackgroundMode) = SettingsDto(
        deviceName = "p", downloadDir = "/d", autoImageMax = 0uL, autoFileMax = 0uL, autoFileWifiOnly = true,
        localsendEnabled = false, unicastTargets = emptyList(), backgroundMode = mode, localsendListening = null,
    )

    @Test fun mode_and_conditions_drive_service_and_worker() = runTest {
        val actions = RecordingActions()
        val holder = CoreHolder({ FakeCoreApi() }, backgroundScope, StandardTestDispatcher(testScheduler))
        val cond = MutableStateFlow(Conditions(false, false, false, false))
        val c = BackgroundController(holder, cond, backgroundScope, actions)
        c.start()
        runCurrent()
        assertEquals(emptyList<Boolean>(), actions.service) // 模式未知（未初始化）：什么都不做

        holder.onBackgroundModeChanged(BackgroundMode.POWER_SAVING)
        runCurrent()
        assertEquals(listOf(false), actions.service)
        assertEquals(listOf(true), actions.worker)

        cond.value = cond.value.copy(charging = true)
        runCurrent()
        assertEquals(listOf(false, true), actions.service)

        holder.onBackgroundModeChanged(BackgroundMode.OFF)
        runCurrent()
        assertEquals(listOf(false, true, false), actions.service)
        assertEquals(listOf(true, false), actions.worker)
    }

    @Test fun start_reads_mode_from_core_and_releases_its_owner() = runTest {
        val actions = RecordingActions()
        val fake = FakeCoreApi().apply {
            handler = { req ->
                when (req) {
                    Request.GetSelf -> Response.SelfInfo(Fixtures.selfInfo(initialized = true))
                    Request.GetSettings -> Response.Settings(settings(BackgroundMode.REALTIME))
                    else -> Response.Ok
                }
            }
        }
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        val c = BackgroundController(holder, MutableStateFlow(Conditions(false, false, false, false)), backgroundScope, actions, serviceWaitMillis = 1000)
        c.start()
        runCurrent()
        assertEquals(listOf(true), actions.service)
        advanceTimeBy(1001)
        runCurrent()
        // 服务没有真正起来（测试里没有 LanMsgService）：等待超时后控制器放手，core 随之关闭
        assertEquals(1, fake.shutdownCount)
    }
}
