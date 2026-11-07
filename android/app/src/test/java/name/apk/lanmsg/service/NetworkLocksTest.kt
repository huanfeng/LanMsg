package name.apk.lanmsg.service

import android.content.Context
import android.net.wifi.WifiManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowWifiManager

@RunWith(AndroidJUnit4::class)
@Config(sdk = [34])
class NetworkLocksTest {
    private lateinit var wifi: WifiManager
    private lateinit var shadow: ShadowWifiManager

    @Before fun setUp() {
        val ctx = ApplicationProvider.getApplicationContext<Context>()
        wifi = ctx.getSystemService(Context.WIFI_SERVICE) as WifiManager
        shadow = shadowOf(wifi)
    }

    @Test fun start_holds_multicast_but_not_wifi_lock() {
        val locks = NetworkLocks(wifi)
        locks.start()
        assertEquals(1, shadow.activeLockCount) // 仅 MulticastLock
    }

    @Test fun wifi_lock_follows_wanted_flag() {
        val locks = NetworkLocks(wifi)
        locks.start()
        locks.setWifiLockWanted(true)
        assertEquals(2, shadow.activeLockCount)
        locks.setWifiLockWanted(true) // 重复请求不重复获取
        assertEquals(2, shadow.activeLockCount)
        locks.setWifiLockWanted(false)
        assertEquals(1, shadow.activeLockCount)
    }

    @Test fun stop_releases_everything() {
        val locks = NetworkLocks(wifi)
        locks.start()
        locks.setWifiLockWanted(true)
        locks.stop()
        assertEquals(0, shadow.activeLockCount)
    }

    @Test fun lock_mode_depends_on_api_level() {
        @Suppress("DEPRECATION")
        assertEquals(WifiManager.WIFI_MODE_FULL_HIGH_PERF, NetworkLocks.wifiLockMode(33))
        assertEquals(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, NetworkLocks.wifiLockMode(34))
    }
}
