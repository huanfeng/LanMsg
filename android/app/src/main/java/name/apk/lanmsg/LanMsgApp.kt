package name.apk.lanmsg

import android.Manifest
import android.app.Application
import android.app.NotificationManager
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat
import name.apk.lanmsg.notify.NotificationChannels
import name.apk.lanmsg.notify.Notifier

class LanMsgApp : Application() {
    override fun onCreate() {
        super.onCreate()
        ServiceLocator.app = this
        NotificationChannels.ensure(this)
        // 档位与条件决定是否常驻服务；core 未初始化时控制器什么都不做
        ServiceLocator.backgroundController.start()
        ServiceLocator.notifier.start()
    }
}
