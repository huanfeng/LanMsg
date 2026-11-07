package name.apk.lanmsg.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.BatteryManager
import android.os.PowerManager
import androidx.core.content.ContextCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import name.apk.lanmsg.core.LocalPrefs
import name.apk.lanmsg.ui.settings.WifiSsid

/**
 * 观察屏幕、充电、可信 Wi-Fi 与 App 前后台，汇总成 [conditions]。随进程存活，不反注册。
 * 须在主线程构造（ProcessLifecycleOwner）。
 */
class DeviceConditions(ctx: Context, prefs: LocalPrefs, scope: CoroutineScope) {
    private val app = ctx.applicationContext
    private val _conditions = MutableStateFlow(
        Conditions(
            appVisible = ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED),
            screenOn = (app.getSystemService(Context.POWER_SERVICE) as PowerManager).isInteractive,
            charging = (app.getSystemService(Context.BATTERY_SERVICE) as BatteryManager).isCharging,
            onTrustedWifi = false,
        ),
    )
    val conditions: StateFlow<Conditions> = _conditions.asStateFlow()

    @Volatile private var trusted: Set<String> = emptySet()

    init {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                when (intent.action) {
                    Intent.ACTION_SCREEN_ON -> _conditions.update { it.copy(screenOn = true) }
                    Intent.ACTION_SCREEN_OFF -> _conditions.update { it.copy(screenOn = false) }
                    Intent.ACTION_POWER_CONNECTED -> _conditions.update { it.copy(charging = true) }
                    Intent.ACTION_POWER_DISCONNECTED -> _conditions.update { it.copy(charging = false) }
                }
            }
        }
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_SCREEN_OFF)
            addAction(Intent.ACTION_POWER_CONNECTED)
            addAction(Intent.ACTION_POWER_DISCONNECTED)
        }
        ContextCompat.registerReceiver(app, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)

        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                _conditions.update { it.copy(appVisible = true) }
                recomputeWifi() // 用户可能刚授予了定位权限，SSID 才读得到
            }

            override fun onStop(owner: LifecycleOwner) {
                _conditions.update { it.copy(appVisible = false) }
            }
        })

        val cm = app.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        cm.registerNetworkCallback(
            NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI).build(),
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) = recomputeWifi()

                override fun onLost(network: Network) = recomputeWifi()

                override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) = recomputeWifi()
            },
        )

        scope.launch {
            prefs.trustedWifi.collect {
                trusted = it
                recomputeWifi()
            }
        }
    }

    private fun recomputeWifi() {
        val reading = WifiSsid.current(app)
        val on = reading is WifiSsid.Reading.Connected && reading.ssid in trusted
        _conditions.update { it.copy(onTrustedWifi = on) }
    }
}
