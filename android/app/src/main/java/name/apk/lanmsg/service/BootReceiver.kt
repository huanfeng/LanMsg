package name.apk.lanmsg.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import name.apk.lanmsg.ServiceLocator

/** 开机或升级后拉起进程并启动控制器；core 未初始化或档位为“关闭”时控制器自己什么都不做。 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            ServiceLocator.backgroundController.start()
        }
    }
}
