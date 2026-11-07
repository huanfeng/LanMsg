package name.apk.lanmsg

import android.app.Application

/** Robolectric 专用 Application：只设置 ServiceLocator.app，不启动后台控制器（那会尝试创建真实 core）。 */
class TestApp : Application() {
    override fun onCreate() {
        super.onCreate()
        ServiceLocator.app = this
    }
}
