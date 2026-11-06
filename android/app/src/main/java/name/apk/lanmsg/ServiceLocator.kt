package name.apk.lanmsg

import android.app.Application
import android.content.Context

object ServiceLocator {
    lateinit var app: Application

    // 骨架阶段的占位：返回类型暂为 Any，Task 3 引入 CoreApi 后改为 (Context) -> CoreApi，
    // 默认值改为 FfiCoreApi.create；测试中替换为 Fake。
    var coreFactory: (Context) -> Any = { throw IllegalStateException("core 未就绪") }
}
