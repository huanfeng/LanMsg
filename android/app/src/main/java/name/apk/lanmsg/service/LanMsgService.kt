package name.apk.lanmsg.service

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.wifi.WifiManager
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import name.apk.lanmsg.MainActivity
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.core.CoreHolder
import name.apk.lanmsg.notify.NotificationChannels
import uniffi.lanmsg_api.BackgroundMode
import uniffi.lanmsg_api.Request

/** 常驻前台服务（specialUse）：持有 core 与网络锁，保证后台也能收到局域网消息。 */
class LanMsgService : LifecycleService() {
    private var locks: NetworkLocks? = null
    private var networkNotifier: NetworkChangeNotifier? = null

    // 常量在编译期内联；API 34 以下系统忽略类型参数
    @SuppressLint("InlinedApi")
    override fun onCreate() {
        super.onCreate()
        try {
            ServiceCompat.startForeground(
                this, NOTIF_ID_SERVICE, buildNotification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
            )
        } catch (e: RuntimeException) {
            // 系统不允许此刻进入前台（如后台启动限制）：放弃本次启动，下次条件变化时控制器会重试
            Log.w(TAG, "Cannot enter foreground", e)
            stopSelf()
            return
        }
        val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        locks = NetworkLocks(wifi).also { it.start() }
        // WifiLock 只在实时档持有；档位变化时随之获取 / 释放
        lifecycleScope.launch {
            ServiceLocator.coreHolder.backgroundMode.collect { mode ->
                locks?.setWifiLockWanted(mode == BackgroundMode.REALTIME)
            }
        }
        // 网络变化时让 core 清零重连退避、立即重拨已配对设备
        val cm = applicationContext.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        networkNotifier = NetworkChangeNotifier(cm) {
            lifecycleScope.launch(Dispatchers.IO) {
                try {
                    ServiceLocator.coreHolder.api.request(Request.NetworkChanged)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to notify core of network change", e)
                }
            }
        }.also { it.start() }
        // UNDISPATCHED：owner 在主线程上同步登记，onDestroy 的 release 一定排在它之后
        lifecycleScope.launch(Dispatchers.IO, start = CoroutineStart.UNDISPATCHED) {
            try {
                ServiceLocator.coreHolder.acquire(CoreHolder.SERVICE)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e(TAG, "Failed to start core", e)
                stopSelf()
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        return START_STICKY
    }

    override fun onDestroy() {
        super.onDestroy()
        networkNotifier?.stop()
        networkNotifier = null
        locks?.stop()
        locks = null
        ServiceLocator.coreHolder.release(CoreHolder.SERVICE)
    }

    private fun buildNotification(): android.app.Notification {
        // 服务可能先于 Application 之外的入口启动，自己保证渠道存在
        NotificationChannels.ensure(this)
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, NotificationChannels.SERVICE)
            .setSmallIcon(R.drawable.ic_stat_lanmsg)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(getString(R.string.notif_service_text))
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .build()
    }

    companion object {
        private const val TAG = "LanMsgService"
        const val NOTIF_ID_SERVICE = 1
    }
}
