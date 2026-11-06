package name.apk.lanmsg

import android.app.Application
import android.content.Context
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.FfiCoreApi
import name.apk.lanmsg.core.LocalPrefs

object ServiceLocator {
    lateinit var app: Application

    /** 测试中替换为 FakeCoreApi 工厂，Robolectric 下不得加载本地库。 */
    var coreFactory: (Context) -> CoreApi = { FfiCoreApi.create(it) }

    val localPrefs: LocalPrefs by lazy { LocalPrefs(app) }
}
