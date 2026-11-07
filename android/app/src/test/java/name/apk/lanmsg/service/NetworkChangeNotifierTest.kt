package name.apk.lanmsg.service

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
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
        assertEquals(1, calls)
        // 紧随 onAvailable 的首次 LinkProperties 只记基线，内容没变的重复回调也不转发
        cb.onLinkPropertiesChanged(network, LinkProperties())
        cb.onLinkPropertiesChanged(network, LinkProperties())
        assertEquals(1, calls)
    }

    @Test fun link_snapshot_change_requires_a_baseline_and_a_difference() {
        val a = LinkSnapshot(setOf("192.168.1.2/24"), setOf("0.0.0.0/0"))
        val b = LinkSnapshot(setOf("10.0.0.5/24"), setOf("0.0.0.0/0"))
        assertFalse(LinkSnapshot.changed(null, a))
        assertFalse(LinkSnapshot.changed(a, a.copy()))
        assertTrue(LinkSnapshot.changed(a, b))
        assertTrue(LinkSnapshot.changed(a, a.copy(routes = emptySet())))
    }

    @Test fun stop_unregisters_the_callback() {
        val notifier = NetworkChangeNotifier(cm) {}
        notifier.start()
        notifier.start() // 重复 start 不重复注册
        assertEquals(1, shadowOf(cm).networkCallbacks.size)
        notifier.stop()
        assertEquals(0, shadowOf(cm).networkCallbacks.size)
    }

    @Test fun ipv6_privacy_address_rotation_is_not_a_change() {
        val before = LinkSnapshot.fromStrings(
            listOf("192.168.1.2/24", "2409:8a00::1111/64"),
            listOf("192.168.1.0/24 -> 0.0.0.0 wlan0", "::/0 -> fe80::1 wlan0"),
        )
        val after = LinkSnapshot.fromStrings(
            listOf("192.168.1.2/24", "2409:8a00::2222/64"),
            listOf("192.168.1.0/24 -> 0.0.0.0 wlan0", "::/0 -> fe80::1 wlan0"),
        )
        assertFalse(LinkSnapshot.changed(before, after))
        val moved = LinkSnapshot.fromStrings(listOf("10.0.0.5/24", "2409:8a00::2222/64"), emptyList())
        assertTrue(LinkSnapshot.changed(before, moved))
    }
}
