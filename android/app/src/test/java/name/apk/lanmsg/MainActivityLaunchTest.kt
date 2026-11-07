package name.apk.lanmsg

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import name.apk.lanmsg.core.CoreHolder
import android.os.Looper
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import kotlinx.coroutines.launch
import org.robolectric.Shadows.shadowOf
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class MainActivityLaunchTest {
    // 启动时 MainActivity 会请求 GetSelf，Robolectric 中不能加载本地库
    @Before fun fakeCore() {
        ServiceLocator.coreFactory = {
            FakeCoreApi().apply { handler = { Response.SelfInfo(value = Fixtures.selfInfo(initialized = false)) } }
        }
    }

    @Test
    fun activity_launches_and_shows_app_name() {
        ActivityScenario.launch(MainActivity::class.java).use { s ->
            s.onActivity { a -> assertEquals("LanMsg", a.getString(R.string.app_name)) }
        }
    }

    @Test
    fun fetch_self_retries_core_creation_after_first_failure() = runBlocking {
        var n = 0
        val scope = CoroutineScope(SupervisorJob())
        val holder = CoreHolder(
            {
                if (++n == 1) throw IllegalStateException("boom")
                FakeCoreApi().apply { handler = { Response.SelfInfo(value = Fixtures.selfInfo(initialized = true)) } }
            },
            scope, Dispatchers.IO,
        )
        try {
            fetchSelf(holder)
            fail("first attempt should fail")
        } catch (e: IllegalStateException) {
            assertEquals("boom", e.message)
        }
        // 对应界面点“重试”：同一个 holder 上再试一次应当成功
        assertTrue(fetchSelf(holder).initialized)
        assertEquals(2, n)
        scope.cancel()
    }

    private class Owner : LifecycleOwner {
        val registry = LifecycleRegistry.createUnsafe(this)
        override val lifecycle: Lifecycle get() = registry
    }

    /** 在主线程跑协程，反复处理主线程消息直到 [cond] 成立。 */
    private fun idleUntil(cond: () -> Boolean) {
        val end = System.currentTimeMillis() + 5_000
        while (!cond()) {
            shadowOf(Looper.getMainLooper()).idle()
            check(System.currentTimeMillis() < end) { "timeout" }
            Thread.sleep(5)
        }
    }

    @Test
    fun release_during_acquire_then_restart_still_gets_result() {
        val gate = CountDownLatch(1)
        val entered = CountDownLatch(1)
        val created = AtomicInteger()
        val scope = CoroutineScope(SupervisorJob())
        val holder = CoreHolder(
            {
                if (created.incrementAndGet() == 1) { entered.countDown(); gate.await() }
                FakeCoreApi().apply { handler = { Response.SelfInfo(value = Fixtures.selfInfo(initialized = true)) } }
            },
            scope, Dispatchers.IO,
        )
        val owner = Owner()
        owner.registry.currentState = Lifecycle.State.STARTED
        var result: Boolean? = null
        var failure: Int? = null
        val ui = CoroutineScope(Dispatchers.Main)
        ui.launch {
            fetchSelfWhileStarted(
                owner.lifecycle, holder, hasResult = { result != null }, onStart = {},
                onSelf = { result = it.initialized }, onFailure = { failure = it },
            )
        }
        idleUntil { entered.count == 0L }
        // 模拟 onStop：先 stop 生命周期（取消在途获取），再放手 ui
        owner.registry.currentState = Lifecycle.State.CREATED
        holder.release(CoreHolder.UI)
        gate.countDown()
        // 再次进入前台：应当重新获取并拿到结果，而不是空白
        owner.registry.currentState = Lifecycle.State.STARTED
        idleUntil { result != null }
        assertEquals(true, result)
        assertEquals(null, failure)
        assertEquals(2, created.get())
        ui.cancel()
        scope.cancel()
    }

    @Test
    fun no_acquire_while_not_started() {
        val created = AtomicInteger()
        val scope = CoroutineScope(SupervisorJob())
        val holder = CoreHolder({ created.incrementAndGet(); FakeCoreApi() }, scope, Dispatchers.IO)
        val owner = Owner()
        owner.registry.currentState = Lifecycle.State.CREATED
        val ui = CoroutineScope(Dispatchers.Main)
        ui.launch {
            fetchSelfWhileStarted(owner.lifecycle, holder, { false }, {}, {}, {})
        }
        repeat(20) { shadowOf(Looper.getMainLooper()).idle(); Thread.sleep(10) }
        assertEquals(0, created.get())
        assertEquals(emptySet<String>(), holder.owners.value)
        ui.cancel()
        scope.cancel()
    }
}
