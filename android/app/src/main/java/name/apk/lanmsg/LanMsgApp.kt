package name.apk.lanmsg

import android.app.Application
import name.apk.lanmsg.notify.NotificationChannels

class LanMsgApp : Application() {
    override fun onCreate() {
        super.onCreate()
        ServiceLocator.app = this
        NotificationChannels.ensure(this)
        // 档位与条件决定是否常驻服务；core 未初始化时控制器什么都不做
        ServiceLocator.backgroundController.start()
        ServiceLocator.notifier.start()
        // 启动自动检查更新：core 被拉起后查一次，24h 节流，开关关闭时跳过
        ServiceLocator.updateChecker.start()
    }
}
