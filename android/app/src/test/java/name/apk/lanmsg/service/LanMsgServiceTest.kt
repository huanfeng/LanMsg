package name.apk.lanmsg.service

import android.content.Context
import android.net.ConnectivityManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.testutil.FakeCoreApi
import uniffi.lanmsg_api.Request

@RunWith(AndroidJUnit4::class)
@Config(sdk = [34])
class LanMsgServiceTest {
    // coreFactory 在服务 IO 线程上回调创建并追加，测试线程轮询遍历——
    // mutableListOf 跨线程读写无同步会抛 ConcurrentModificationException，
    // 用 CopyOnWriteArrayList（写少读多）消除竞争。
    private val cores = java.util.concurrent.CopyOnWriteArrayList<FakeCoreApi>()

    @Before fun fakeCore() {
        ServiceLocator.coreFactory = { FakeCoreApi().also { cores += it } }
    }

    /** acquire 的创建在 IO 线程上完成，轮询等待状态。 */
    private fun awaitRunning(expected: Boolean) {
        val deadline = System.currentTimeMillis() + 5000
        while (ServiceLocator.coreHolder.running.value != expected && System.currentTimeMillis() < deadline) Thread.sleep(20)
        assertTrue("running should be $expected", ServiceLocator.coreHolder.running.value == expected)
    }

    @Test fun service_goes_foreground_and_acquires_core() {
        val controller = Robolectric.buildService(LanMsgService::class.java).create()
        assertNotNull(shadowOf(controller.get()).lastForegroundNotification)
        awaitRunning(true)
        assertTrue("service" in ServiceLocator.coreHolder.owners.value)
        controller.destroy()
        awaitRunning(false)
    }

    @Test fun network_callback_notifies_core_and_is_unregistered_on_destroy() {
        val cm = ApplicationProvider.getApplicationContext<Context>().getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val controller = Robolectric.buildService(LanMsgService::class.java).create()
        awaitRunning(true)
        val cb = shadowOf(cm).networkCallbacks.single()
        cb.onAvailable(requireNotNull(cm.activeNetwork))
        val deadline = System.currentTimeMillis() + 5000
        while (cores.none { c -> c.requests.any { it is Request.NetworkChanged } } && System.currentTimeMillis() < deadline) Thread.sleep(20)
        assertTrue(cores.any { c -> c.requests.any { it is Request.NetworkChanged } })
        controller.destroy()
        assertTrue(shadowOf(cm).networkCallbacks.isEmpty())
        awaitRunning(false)
    }
}
