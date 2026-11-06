package name.apk.lanmsg

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import org.junit.Assert.assertEquals
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
}
