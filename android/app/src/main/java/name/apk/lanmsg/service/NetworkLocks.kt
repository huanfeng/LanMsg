package name.apk.lanmsg.service

import android.net.wifi.WifiManager
import android.os.Build

/**
 * 前台服务期间持有的网络锁：MulticastLock 全程持有；WifiLock 只在实时档持有。
 * 所有方法须在同一线程（主线程）调用。
 */
class NetworkLocks(private val wifi: WifiManager, sdkInt: Int = Build.VERSION.SDK_INT) {
    private var multicastLock: WifiManager.MulticastLock? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private val wifiMode = wifiLockMode(sdkInt)

    /** 服务启动时调用。 */
    fun start() {
        if (multicastLock != null) return
        multicastLock = wifi.createMulticastLock(TAG).apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    /** 实时档为 true：持有 WifiLock，API 34 以下可防止熄屏后 Wi-Fi 进入省电（34 及以上仅亮屏前台有效，见 wifiLockMode）；否则释放。 */
    fun setWifiLockWanted(wanted: Boolean) {
        if (wanted) {
            if (wifiLock?.isHeld == true) return
            wifiLock = wifi.createWifiLock(wifiMode, TAG).apply {
                setReferenceCounted(false)
                acquire()
            }
        } else {
            releaseWifi()
        }
    }

    /** 服务停止时调用，释放全部锁。 */
    fun stop() {
        releaseWifi()
        multicastLock?.takeIf { it.isHeld }?.release()
        multicastLock = null
    }

    private fun releaseWifi() {
        wifiLock?.takeIf { it.isHeld }?.release()
        wifiLock = null
    }

    companion object {
        private const val TAG = "lanmsg"

        /**
         * 注意：这把锁对后台保活的作用有限。API 34 起系统把 FULL_HIGH_PERF 直接换成
         * FULL_LOW_LATENCY，而 LOW_LATENCY 只在已连 AP、亮屏且应用在前台时才生效；
         * 所以 API 34 及以上熄屏后它几乎没有效果，后台保活主要靠豁免电池优化、
         * 厂商省电策略和自启动。34 以下 HIGH_PERF 在后台也有效，故仍用它。
         */
        @Suppress("DEPRECATION")
        fun wifiLockMode(sdkInt: Int): Int =
            if (sdkInt >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                WifiManager.WIFI_MODE_FULL_LOW_LATENCY
            } else {
                WifiManager.WIFI_MODE_FULL_HIGH_PERF
            }
    }
}
