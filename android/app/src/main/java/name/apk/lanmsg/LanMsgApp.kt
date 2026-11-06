package name.apk.lanmsg

import android.app.Application

class LanMsgApp : Application() {
    override fun onCreate() {
        super.onCreate()
        ServiceLocator.app = this
    }
}
