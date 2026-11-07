@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.core

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.BackgroundMode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SettingsDto

@RunWith(AndroidJUnit4::class)
class CoreHolderTest {
    private fun settings(dir: String = "/d", mode: BackgroundMode = BackgroundMode.REALTIME) = SettingsDto(
        deviceName = "p", downloadDir = dir, autoImageMax = 0uL, autoFileMax = 0uL, autoFileWifiOnly = true,
        localsendEnabled = false, unicastTargets = emptyList(), backgroundMode = mode, localsendListening = null,
    )

    @Test fun core_holder_starts_core_once_for_concurrent_acquires() = runTest {
        var created = 0
        val holder = CoreHolder({ created++; FakeCoreApi() }, backgroundScope, StandardTestDispatcher(testScheduler))
        val a = async { holder.acquire("ui") }
        val b = async { holder.acquire("service") }
        assertSame(a.await(), b.await())
        assertEquals(1, created)
    }

    @Test fun release_to_zero_shuts_down_core() = runTest {
        val fake = FakeCoreApi()
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire("ui")
        holder.acquire("service")
        assertTrue(holder.running.value)
        holder.release("ui")
        runCurrent()
        assertEquals(0, fake.shutdownCount)
        holder.release("service")
        runCurrent()
        assertEquals(1, fake.shutdownCount)
        assertFalse(holder.running.value)
    }

    @Test fun same_owner_acquired_twice_counts_once() = runTest {
        val fake = FakeCoreApi()
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire("ui")
        holder.acquire("ui")
        holder.release("ui")
        runCurrent()
        assertEquals(1, fake.shutdownCount)
    }

    @Test fun reacquire_after_shutdown_creates_new_core() = runTest {
        var created = 0
        val holder = CoreHolder({ created++; FakeCoreApi() }, backgroundScope, StandardTestDispatcher(testScheduler))
        val first = holder.acquire("ui")
        holder.release("ui")
        runCurrent()
        val second = holder.acquire("ui")
        assertEquals(2, created)
        assertNotSame(first, second)
    }

    @Test fun release_immediately_followed_by_acquire_never_overlaps_two_cores() = runTest {
        val log = mutableListOf<String>()
        var n = 0
        val holder = CoreHolder(
            {
                val id = ++n
                log += "create$id"
                object : CoreApi by FakeCoreApi() {
                    override fun shutdown() {
                        log += "shutdown$id"
                    }
                }
            },
            backgroundScope, StandardTestDispatcher(testScheduler),
        )
        holder.acquire("ui")
        holder.release("ui")
        val again = async { holder.acquire("ui") }
        again.await()
        // 旧实例必须先完全关闭，新实例才能创建（否则抢同一个数据库与端口）
        assertEquals(listOf("create1", "shutdown1", "create2"), log)
        assertTrue(holder.running.value)
    }

    @Test fun proxy_forwards_requests_to_current_core_and_acquires_ui_if_idle() = runTest {
        val fake = FakeCoreApi().apply { handler = { Response.Ok } }
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        assertEquals(Response.Ok, holder.api.request(Request.GetSelf))
        assertTrue(holder.running.value)
        assertTrue("ui" in holder.owners.value)
        assertEquals(listOf<Request>(Request.GetSelf), fake.requests)
    }

    @Test fun proxy_does_not_add_ui_owner_when_core_already_running() = runTest {
        val fake = FakeCoreApi().apply { handler = { Response.Ok } }
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire("service")
        holder.api.request(Request.GetSelf)
        assertEquals(setOf("service"), holder.owners.value)
    }

    @Test fun proxy_events_follow_the_current_instance_and_new_instance_signals_lagged() = runTest {
        val cores = mutableListOf<FakeCoreApi>()
        val holder = CoreHolder({ FakeCoreApi().also { cores += it } }, backgroundScope, StandardTestDispatcher(testScheduler))
        val seen = mutableListOf<Event>()
        var lagged = 0
        backgroundScope.launch { holder.api.events.collect { seen += it } }
        backgroundScope.launch { holder.api.lagged.collect { lagged++ } }
        runCurrent()
        holder.acquire("ui")
        runCurrent()
        assertEquals(0, lagged) // 首个实例不触发全量重拉
        cores[0].emit(Event.PeerOffline("d1"))
        runCurrent()
        holder.release("ui")
        runCurrent()
        holder.acquire("ui")
        runCurrent()
        cores[1].emit(Event.PeerOffline("d2"))
        cores[0].emit(Event.PeerOffline("old"))
        runCurrent()
        assertEquals(listOf<Event>(Event.PeerOffline("d1"), Event.PeerOffline("d2")), seen)
        assertEquals(1, lagged) // 实例更换后让订阅方重拉
    }

    @Test fun ensure_download_dir_sets_only_when_different_and_tolerates_failure() = runTest {
        val fake = FakeCoreApi().apply {
            handler = { req -> if (req == Request.GetSettings) Response.Settings(settings("/old")) else Response.Ok }
        }
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.ensureDownloadDir("/new") // 没有实例：不崩溃
        holder.acquire("ui")
        holder.ensureDownloadDir("/new")
        val set = fake.requests.filterIsInstance<Request.SetSettings>().single()
        assertEquals("/new", set.settings.downloadDir)
        fake.requests.clear()
        fake.handler = { req -> if (req == Request.GetSettings) Response.Settings(settings("/new")) else Response.Ok }
        holder.ensureDownloadDir("/new")
        assertTrue(fake.requests.none { it is Request.SetSettings })
        fake.handler = { throw ApiException(uniffi.lanmsg_api.ErrorCode.NOT_INITIALIZED, "not initialized") }
        holder.ensureDownloadDir("/other") // 未初始化：容忍
    }

    @Test fun new_core_gets_download_dir_applied_before_it_is_published() = runTest {
        val fake = FakeCoreApi().apply {
            handler = { req -> if (req == Request.GetSettings) Response.Settings(settings("/old")) else Response.Ok }
        }
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler), downloadDir = { "/ext/Download/LanMsg" })
        holder.acquire("ui")
        assertEquals("/ext/Download/LanMsg", fake.requests.filterIsInstance<Request.SetSettings>().single().settings.downloadDir)
    }

    @Test fun background_mode_is_unknown_until_initialized() = runTest {
        val fake = FakeCoreApi().apply {
            handler = { req ->
                when (req) {
                    Request.GetSelf -> Response.SelfInfo(Fixtures.selfInfo(initialized = false))
                    else -> Response.Settings(settings(mode = BackgroundMode.POWER_SAVING))
                }
            }
        }
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire("ui")
        holder.refreshBackgroundMode()
        assertNull(holder.backgroundMode.value)
        fake.handler = { req ->
            when (req) {
                Request.GetSelf -> Response.SelfInfo(Fixtures.selfInfo(initialized = true))
                else -> Response.Settings(settings(mode = BackgroundMode.POWER_SAVING))
            }
        }
        holder.refreshBackgroundMode()
        assertEquals(BackgroundMode.POWER_SAVING, holder.backgroundMode.value)
    }
}
