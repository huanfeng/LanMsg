@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.core

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.withContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import java.util.concurrent.CountDownLatch
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
        localsendEnabled = false, unicastTargets = emptyList(), backgroundMode = mode, localsendListening = null, localsendPort = 53317u, autoCheckUpdate = true,
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

    @Test fun proxy_forwards_requests_to_core_started_by_ui_owner() = runTest {
        val fake = FakeCoreApi().apply { handler = { Response.Ok } }
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire("ui")
        assertEquals(Response.Ok, holder.api.request(Request.GetSelf))
        assertEquals(listOf<Request>(Request.GetSelf), fake.requests)
    }

    @Test fun proxy_after_ui_release_neither_creates_core_nor_leaves_owner() = runTest {
        var created = 0
        val holder = CoreHolder({ created++; FakeCoreApi() }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire("ui")
        holder.release("ui")
        runCurrent()
        val e = runCatching { holder.api.request(Request.GetSelf) }.exceptionOrNull()
        assertTrue(e is ApiException)
        assertEquals("core not running", e?.message)
        assertEquals(emptySet<String>(), holder.owners.value)
        assertFalse(holder.running.value)
        assertEquals(1, created)
    }

    @Test fun proxy_wraps_non_api_exceptions_from_a_shut_down_core() = runTest {
        val fake = FakeCoreApi().apply { handler = { throw IllegalStateException("closed") } }
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire("ui")
        val e = runCatching { holder.api.request(Request.GetSelf) }.exceptionOrNull()
        assertTrue(e is ApiException)
    }

    /** shutdown 阻塞直到 [gate] 放行的实例；用真实线程，因为阻塞调用在单线程测试调度器上会卡死。 */
    private class BlockingShutdownCore(private val gate: CountDownLatch) : CoreApi by FakeCoreApi() {
        override fun shutdown() {
            gate.await()
        }
    }

    @Test fun acquire_cancelled_while_waiting_reclaims_owner_and_allows_shutdown() = runBlocking {
        val gate = CountDownLatch(1)
        val scope = CoroutineScope(SupervisorJob())
        var n = 0
        val holder = CoreHolder(
            { if (++n == 1) BlockingShutdownCore(gate) else FakeCoreApi() },
            scope, Dispatchers.IO, shutdownTimeoutMillis = 5_000,
        )
        holder.acquire("ui")
        holder.release("ui") // 旧实例的 shutdown 阻塞着，占住 lifecycle 锁
        val waiting = launch(Dispatchers.Default) { holder.acquire("worker") }
        withTimeout(2_000) { holder.owners.first { "worker" in it } }
        waiting.cancelAndJoin()
        assertEquals(emptySet<String>(), holder.owners.value)
        gate.countDown()
        holder.acquire("ui") // 锁放开后可以正常创建，随后能 shutdown
        holder.release("ui")
        withTimeout(2_000) { while (holder.running.value) delay(10) }
        scope.cancel()
    }

    @Test fun acquire_cancelled_while_core_is_being_created_still_shuts_it_down() = runBlocking {
        val gate = CountDownLatch(1)
        val entered = CountDownLatch(1)
        val fake = FakeCoreApi()
        val scope = CoroutineScope(SupervisorJob())
        val holder = CoreHolder({ entered.countDown(); gate.await(); fake }, scope, Dispatchers.IO)
        val job = launch(Dispatchers.Default) { holder.acquire("ui") }
        withTimeout(2_000) { while (entered.count > 0L) delay(5) }
        job.cancel() // 创建不可中断，此时只标记取消
        gate.countDown()
        job.join()
        withTimeout(2_000) { while (!fake.shutdownCalled) delay(10) }
        assertEquals(emptySet<String>(), holder.owners.value)
        assertFalse(holder.running.value)
        scope.cancel()
    }

    @Test fun acquire_already_cancelled_with_running_instance_reclaims_owner() = runBlocking {
        val scope = CoroutineScope(SupervisorJob())
        val holder = CoreHolder({ FakeCoreApi() }, scope, Dispatchers.IO)
        holder.acquire("service") // 实例已由 service 持有，acquire 本可走快路径
        // 在协程内部取消后再 acquire：ensureActive 必须在快路径之前生效
        val result = runCatching { withContext(Job()) { currentCoroutineContext().cancel(); holder.acquire("ui") } }
        assertTrue(result.exceptionOrNull() is CancellationException)
        assertEquals(setOf("service"), holder.owners.value)
        assertTrue(holder.running.value)
        scope.cancel()
    }

    @Test fun shutdown_that_hangs_times_out_and_lets_next_acquire_proceed() = runBlocking {
        val gate = CountDownLatch(1)
        val scope = CoroutineScope(SupervisorJob())
        var n = 0
        val holder = CoreHolder(
            { if (++n == 1) BlockingShutdownCore(gate) else FakeCoreApi() },
            scope, Dispatchers.IO, shutdownTimeoutMillis = 200,
        )
        holder.acquire("ui")
        holder.release("ui")
        withTimeout(3_000) { holder.acquire("ui") }
        assertEquals(2, n)
        gate.countDown()
        scope.cancel()
    }

    @Test fun next_create_waits_for_timed_out_shutdown_to_finish() = runBlocking {
        val gate = CountDownLatch(1)
        val scope = CoroutineScope(SupervisorJob())
        var n = 0
        var gateOpenWhenSecondCreated = false
        val holder = CoreHolder(
            {
                if (++n == 1) {
                    BlockingShutdownCore(gate)
                } else {
                    gateOpenWhenSecondCreated = gate.count == 0L
                    FakeCoreApi()
                }
            },
            scope, Dispatchers.IO, shutdownTimeoutMillis = 100, pendingShutdownWaitMillis = 5_000,
        )
        holder.acquire("ui")
        holder.release("ui")
        // 旧 shutdown 超时（100ms）后才放开；新实例必须等到它结束才创建
        val opener = launch(Dispatchers.Default) {
            delay(500)
            gate.countDown()
        }
        withTimeout(10_000) { holder.acquire("ui") }
        opener.join()
        assertEquals(2, n)
        assertTrue(gateOpenWhenSecondCreated)
        scope.cancel()
    }

    @Test fun pending_shutdown_wait_is_bounded() = runBlocking {
        val gate = CountDownLatch(1)
        val scope = CoroutineScope(SupervisorJob())
        var n = 0
        val holder = CoreHolder(
            { if (++n == 1) BlockingShutdownCore(gate) else FakeCoreApi() },
            scope, Dispatchers.IO, shutdownTimeoutMillis = 100, pendingShutdownWaitMillis = 100,
        )
        holder.acquire("ui")
        holder.release("ui")
        withTimeout(5_000) { holder.acquire("ui") } // gate 始终没放开，仍应放行
        assertEquals(2, n)
        gate.countDown()
        scope.cancel()
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
