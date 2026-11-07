package name.apk.lanmsg

import android.app.Application

class LanMsgApp : Application() {
    override fun onCreate() {
        super.onCreate()
        ServiceLocator.app = this
        // 档位与条件决定是否常驻服务；core 未初始化时控制器什么都不做
        ServiceLocator.backgroundController.start()
    }
}
