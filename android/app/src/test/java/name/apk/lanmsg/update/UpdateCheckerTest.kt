@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.update

import android.app.NotificationManager
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreHolder
import name.apk.lanmsg.core.UpdatePrefs
import name.apk.lanmsg.notify.NotificationChannels
import name.apk.lanmsg.testutil.FakeCoreApi
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import uniffi.lanmsg_api.BackgroundMode
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.InstallChannel
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SettingsDto
import uniffi.lanmsg_api.UpdateCheckView
import uniffi.lanmsg_api.UpdateInfo
import uniffi.lanmsg_api.UpdateAsset

@RunWith(AndroidJUnit4::class)
@Config(sdk = [34])
class UpdateCheckerTest {
    private val ctx: Context = ApplicationProvider.getApplicationContext()
    private val nm = ctx.getSystemService(NotificationManager::class.java)
    private val fake = FakeCoreApi()
    private val prefs = MemoryPrefs()
    private var now = 1_000_000_000_000L

    @Before fun setUp() {
        nm.cancelAll()
        fake.handler = { req ->
            when (req) {
                is Request.GetSettings -> Response.Settings(initialSettings(autoCheckUpdate = true))
                is Request.CheckUpdate -> Response.UpdateCheck(UpdateCheckView(update = null, current = "0.1.0", channel = InstallChannel.MANUAL, releaseUrl = "https://example.com"))
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected ${req.javaClass.simpleName}")
            }
        }
    }

    @After fun tearDown() {
        nm.cancelAll()
    }

    private fun initialSettings(autoCheckUpdate: Boolean) = SettingsDto(
        deviceName = "p", downloadDir = "/d", autoImageMax = 0uL, autoFileMax = 0uL, autoFileWifiOnly = true,
        localsendEnabled = false, unicastTargets = emptyList(), backgroundMode = BackgroundMode.REALTIME,
        localsendListening = null, localsendPort = 53317u, autoCheckUpdate = autoCheckUpdate,
    )

    private class MemoryPrefs : UpdatePrefs {
        override val lastAutoUpdateCheck = MutableStateFlow<Long?>(null)

        override suspend fun setLastAutoUpdateCheck(millis: Long) {
            lastAutoUpdateCheck.value = millis
        }
    }

    private suspend fun TestScope.checker(): UpdateChecker {
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire(CoreHolder.UI)
        runCurrent()
        return UpdateChecker(ctx, holder, prefs, backgroundScope, { now })
    }

    private fun updateInfo(version: String) = UpdateInfo(
        version = version, notes = "bug fixes",
        publishedAt = now, assets = listOf(UpdateAsset("lanmsg-android-arm64.apk", 42uL, "https://example.com/a.apk")),
    )

    private fun active() = nm.activeNotifications.toList()

    @Test fun notifies_when_update_available() = runTest {
        val c = checker()
        fake.handler = { req ->
            when (req) {
                is Request.GetSettings -> Response.Settings(initialSettings(autoCheckUpdate = true))
                is Request.CheckUpdate -> Response.UpdateCheck(UpdateCheckView(update = updateInfo("0.2.0"), current = "0.1.0", channel = InstallChannel.MANUAL, releaseUrl = "https://example.com/releases"))
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected ${req.javaClass.simpleName}")
            }
        }
        val r = c.runCheck()
        runCurrent()
        assertEquals(true, r)
        val a = active()
        assertEquals(1, a.size)
        assertEquals(NotificationChannels.UPDATES, a[0].notification.channelId)
        // 时间戳已记录：节流窗口内的下一次检查被跳过
        assertEquals(now, prefs.lastAutoUpdateCheck.value)
        assertNull(c.runCheck())
    }

    @Test fun no_notification_when_up_to_date() = runTest {
        val c = checker()
        val r = c.runCheck()
        runCurrent()
        assertEquals(false, r)
        assertEquals(0, active().size)
    }

    @Test fun skipped_when_throttled() = runTest {
        prefs.setLastAutoUpdateCheck(now - 3600_000L) // 1 小时前查过
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire(CoreHolder.UI)
        runCurrent()
        val c = UpdateChecker(ctx, holder, prefs, backgroundScope, { now })
        assertNull(c.runCheck())
        // 节流跳过时连 GetSettings 都不发
        assertTrue(fake.requests.none { it is Request.CheckUpdate })
    }

    @Test fun skipped_when_disabled_in_settings() = runTest {
        val c = checker()
        fake.handler = { req ->
            when (req) {
                is Request.GetSettings -> Response.Settings(initialSettings(autoCheckUpdate = false))
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected ${req.javaClass.simpleName}")
            }
        }
        assertNull(c.runCheck())
        assertTrue(fake.requests.none { it is Request.CheckUpdate })
    }

    @Test fun silent_when_settings_unavailable() = runTest {
        val c = checker()
        fake.handler = { throw ApiException(ErrorCode.NOT_INITIALIZED, "not initialized") }
        assertNull(c.runCheck())
        assertEquals(0, active().size)
    }

    /** core 反复启停时每轮 false→true 都尝试检查；节流窗口内跳过、窗口外再查（“每天最多一次”兜底）。 */
    @Test fun checks_again_on_each_core_restart() = runTest {
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        val c = UpdateChecker(ctx, holder, prefs, backgroundScope, { now })
        c.start()
        runCurrent()
        fun checkRequests() = fake.requests.count { it is Request.CheckUpdate }
        holder.acquire(CoreHolder.UI)
        advanceUntilIdle()
        assertEquals(1, checkRequests())
        holder.release(CoreHolder.UI)
        advanceUntilIdle()
        // 节流窗口内重启：不检查
        holder.acquire(CoreHolder.UI)
        advanceUntilIdle()
        assertEquals(1, checkRequests())
        holder.release(CoreHolder.UI)
        advanceUntilIdle()
        // 节流窗口外重启：再检查
        now += 25 * 60 * 60 * 1000L
        holder.acquire(CoreHolder.UI)
        advanceUntilIdle()
        assertEquals(2, checkRequests())
    }

    /** 并发 runCheck（快速关-开 core 或与手动路径同时发生）只执行一个，不双发请求。 */
    @Test fun concurrent_run_checks_are_deduplicated() = runTest {
        val c = checker()
        val gate = CompletableDeferred<Unit>()
        val base = fake.handler
        fake.handler = { req ->
            if (req is Request.GetSettings) {
                gate.await()
                base(req)
            } else base(req)
        }
        val a = launch { c.runCheck() }
        runCurrent() // a 已持锁并挂在 gate.await
        val b = launch { c.runCheck() }
        advanceUntilIdle() // b 被去重直接返回
        gate.complete(Unit)
        advanceUntilIdle()
        assertEquals(1, fake.requests.count { it is Request.GetSettings })
        assertEquals(1, fake.requests.count { it is Request.CheckUpdate })
    }
}
