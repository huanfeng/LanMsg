package name.apk.lanmsg

import android.app.Application
import android.content.Context
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.FfiCoreApi
import name.apk.lanmsg.core.LocalPrefs

object ServiceLocator {
    lateinit var app: Application

    /** 测试中替换为 FakeCoreApi 工厂，Robolectric 下不得加载本地库；替换时丢弃已缓存的实例。 */
    var coreFactory: (Context) -> CoreApi = { FfiCoreApi.create(it) }
        set(value) {
            synchronized(this) {
                field = value
                cachedCore = null
            }
        }

    private var cachedCore: CoreApi? = null

    /**
     * 过渡用的 core 单例，Task 11 由 coreHolder.api 取代。
     * 首次访问会创建真实 core（读写磁盘），必须先在 IO 线程访问一次（MainActivity 启动时已做）。
     */
    val core: CoreApi
        get() = synchronized(this) { cachedCore ?: coreFactory(app).also { cachedCore = it } }

    val localPrefs: LocalPrefs by lazy { LocalPrefs(app) }
}
