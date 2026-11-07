package name.apk.lanmsg.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import name.apk.lanmsg.ServiceLocator
import uniffi.lanmsg_api.BackgroundMode

/**
 * 开机或升级后拉起进程并启动控制器。
 * 先按上次已知档位立即请求启动服务（赶在 BOOT_COMPLETED 的前台服务豁免窗口内），再交给控制器常规评估
 * （条件不满足或档位已变时由控制器停掉）。core 未初始化或档位为“关闭”时什么都不拉起。
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED && intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        val pending = goAsync()
        val actions = SystemServiceActions(context)
        ServiceLocator.appScope.launch {
            try {
                startServiceFromLastMode({ ServiceLocator.localPrefs.lastBackgroundMode.first() }, actions)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "Failed to start service from last known mode", e)
            } finally {
                pending.finish()
            }
            ServiceLocator.backgroundController.start()
        }
    }

    companion object {
        private const val TAG = "BootReceiver"

        /** 上次已知档位非“关闭”则立即请求启动服务；返回是否发起了启动请求。 */
        suspend fun startServiceFromLastMode(lastMode: suspend () -> BackgroundMode?, actions: ServiceActions): Boolean {
            val mode = lastMode()
            if (mode == null || mode == BackgroundMode.OFF) return false
            actions.setServiceRunning(true)
            return true
        }
    }
}
