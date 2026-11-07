package name.apk.lanmsg.service

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log

/**
 * 监听 Wi-Fi 可用 / 链路变化，通知 core 清零重连退避并立即重拨。
 * [start] 与 [stop] 须在同一线程（主线程）调用；[onChange] 在系统回调线程上触发，需自行切线程。
 */
class NetworkChangeNotifier(
    private val cm: ConnectivityManager,
    private val onChange: () -> Unit,
) {
    private var callback: ConnectivityManager.NetworkCallback? = null

    fun start() {
        if (callback != null) return
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = onChange()

            override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) = onChange()

            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) = Unit
        }
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        try {
            cm.registerNetworkCallback(request, cb)
            callback = cb
        } catch (e: RuntimeException) {
            // 个别系统在回调数量超限时抛异常；放弃监听，重连仍有退避兜底
            Log.w(TAG, "Cannot register network callback", e)
        }
    }

    fun stop() {
        val cb = callback ?: return
        callback = null
        try {
            cm.unregisterNetworkCallback(cb)
        } catch (e: IllegalArgumentException) {
            Log.w(TAG, "Network callback was not registered", e)
        }
    }

    private companion object {
        const val TAG = "NetworkChangeNotifier"
    }
}
