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

        /** 前 N 次启动请求被系统拒绝（返回 false）。 */
        var rejectStarts = 0

        override fun setServiceRunning(run: Boolean): Boolean {
            service += run
            if (run && rejectStarts > 0) {
                rejectStarts--
                return false
            }
            return true
        }

        override fun setWorkerScheduled(scheduled: Boolean) {
            worker += scheduled
        }
    }

    private fun settings(mode: BackgroundMode) = SettingsDto(
        deviceName = "p", downloadDir = "/d", autoImageMax = 0uL, autoFileMax = 0uL, autoFileWifiOnly = true,
        localsendEnabled = false, unicastTargets = emptyList(), backgroundMode = mode, localsendListening = null, localsendPort = 53317u, autoCheckUpdate = true,
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

    @Test fun rejected_start_is_retried_only_when_app_becomes_visible() = runTest {
        val actions = RecordingActions().apply { rejectStarts = 1 }
        val holder = CoreHolder({ FakeCoreApi() }, backgroundScope, StandardTestDispatcher(testScheduler))
        val cond = MutableStateFlow(Conditions(false, false, false, false))
        BackgroundController(holder, cond, backgroundScope, actions).start()
        runCurrent()
        holder.onBackgroundModeChanged(BackgroundMode.REALTIME)
        runCurrent()
        assertEquals(listOf(true), actions.service) // 后台启动被拒
        cond.value = cond.value.copy(charging = true)
        runCurrent()
        assertEquals(listOf(true), actions.service) // 仍在后台：不重试
        cond.value = cond.value.copy(appVisible = true)
        runCurrent()
        assertEquals(listOf(true, true), actions.service) // 回到前台：重试成功
        cond.value = cond.value.copy(screenOn = true)
        runCurrent()
        assertEquals(listOf(true, true), actions.service)
    }

    @Test fun mode_seen_callback_receives_each_mode() = runTest {
        val seen = mutableListOf<BackgroundMode>()
        val holder = CoreHolder({ FakeCoreApi() }, backgroundScope, StandardTestDispatcher(testScheduler))
        BackgroundController(
            holder, MutableStateFlow(Conditions(false, false, false, false)), backgroundScope, RecordingActions(),
            onModeSeen = { seen += it },
        ).start()
        runCurrent()
        holder.onBackgroundModeChanged(BackgroundMode.OFF)
        runCurrent()
        holder.onBackgroundModeChanged(BackgroundMode.REALTIME)
        runCurrent()
        assertEquals(listOf(BackgroundMode.OFF, BackgroundMode.REALTIME), seen)
    }

    @Test fun mode_seen_is_written_only_when_mode_changes_and_failure_does_not_stop_collector() = runTest {
        val actions = RecordingActions()
        val holder = CoreHolder({ FakeCoreApi() }, backgroundScope, StandardTestDispatcher(testScheduler))
        val cond = MutableStateFlow(Conditions(false, false, false, false))
        val seen = mutableListOf<BackgroundMode>()
        val c = BackgroundController(
            holder, cond, backgroundScope, actions,
            onModeSeen = { m ->
                seen += m
                if (m == BackgroundMode.POWER_SAVING) throw java.io.IOException("disk full")
            },
        )
        c.start()
        holder.onBackgroundModeChanged(BackgroundMode.POWER_SAVING)
        runCurrent()
        cond.value = cond.value.copy(charging = true)
        runCurrent()
        cond.value = cond.value.copy(appVisible = true)
        runCurrent()
        assertEquals(listOf(BackgroundMode.POWER_SAVING), seen) // 条件变化不重复写
        assertEquals(listOf(false, true), actions.service) // 写入抛异常后 collector 仍在工作
        holder.onBackgroundModeChanged(BackgroundMode.OFF)
        runCurrent()
        assertEquals(listOf(BackgroundMode.POWER_SAVING, BackgroundMode.OFF), seen)
        assertEquals(listOf(false, true, false), actions.service)
    }
}
