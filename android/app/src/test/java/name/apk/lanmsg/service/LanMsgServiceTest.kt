package name.apk.lanmsg.service

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

@RunWith(AndroidJUnit4::class)
@Config(sdk = [34])
class LanMsgServiceTest {
    private val cores = mutableListOf<FakeCoreApi>()

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
}
