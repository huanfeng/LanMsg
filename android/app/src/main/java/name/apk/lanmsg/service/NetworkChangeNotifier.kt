package name.apk.lanmsg.service

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log

/** 链路的地址与路由快照；用于判断 LinkProperties 回调是否真的带来了变化。 */
data class LinkSnapshot(val addresses: Set<String>, val routes: Set<String>) {
    companion object {
        fun of(lp: LinkProperties) = LinkSnapshot(
            addresses = lp.linkAddresses.map { it.toString() }.toSet(),
            routes = lp.routes.map { it.toString() }.toSet(),
        )

        /** 有基线且与之不同才算变化；第一次见到该网络只记基线（紧随 onAvailable，已通知过）。 */
        fun changed(previous: LinkSnapshot?, current: LinkSnapshot): Boolean = previous != null && previous != current
    }
}

/**
 * 监听 Wi-Fi 可用，以及链路地址 / 路由真的变化，通知 core 清零重连退避并立即重拨
 * （core 侧还会再做去抖）。
 * [start] 与 [stop] 须在同一线程（主线程）调用；[onChange] 在系统回调线程上触发，需自行切线程。
 */
class NetworkChangeNotifier(
    private val cm: ConnectivityManager,
    private val onChange: () -> Unit,
) {
    private var callback: ConnectivityManager.NetworkCallback? = null
    private val snapshots = HashMap<Network, LinkSnapshot>()

    fun start() {
        if (callback != null) return
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                synchronized(snapshots) { snapshots.remove(network) }
                onChange()
            }

            override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) {
                val current = LinkSnapshot.of(linkProperties)
                val previous = synchronized(snapshots) { snapshots.put(network, current) }
                if (LinkSnapshot.changed(previous, current)) onChange()
            }

            override fun onLost(network: Network) {
                synchronized(snapshots) { snapshots.remove(network) }
            }

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
        synchronized(snapshots) { snapshots.clear() }
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
