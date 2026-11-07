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
}
