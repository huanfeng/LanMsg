package name.apk.lanmsg.service

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [34])
class NetworkChangeNotifierTest {
    private lateinit var cm: ConnectivityManager

    @Before fun setUp() {
        cm = ApplicationProvider.getApplicationContext<Context>().getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    }

    @Test fun network_callbacks_invoke_the_core_notification() {
        var calls = 0
        val notifier = NetworkChangeNotifier(cm) { calls++ }
        notifier.start()
        val callbacks = shadowOf(cm).networkCallbacks
        assertEquals(1, callbacks.size)
        val cb = callbacks.single()
        val network = requireNotNull(cm.activeNetwork)
        cb.onAvailable(network)
        cb.onLinkPropertiesChanged(network, LinkProperties())
        assertEquals(2, calls)
    }

    @Test fun stop_unregisters_the_callback() {
        val notifier = NetworkChangeNotifier(cm) {}
        notifier.start()
        notifier.start() // 重复 start 不重复注册
        assertEquals(1, shadowOf(cm).networkCallbacks.size)
        notifier.stop()
        assertEquals(0, shadowOf(cm).networkCallbacks.size)
    }
}
