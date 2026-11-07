package name.apk.lanmsg

import android.Manifest
import android.app.Application
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Environment
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.CoreHolder
import name.apk.lanmsg.core.FfiCoreApi
import name.apk.lanmsg.core.LocalPrefs
import name.apk.lanmsg.files.MediaStoreSaver
import name.apk.lanmsg.files.OutboxImporter
import name.apk.lanmsg.notify.Notifier
import name.apk.lanmsg.service.BackgroundController
import name.apk.lanmsg.service.DeviceConditions
import name.apk.lanmsg.service.SystemServiceActions
import java.io.File

object ServiceLocator {
    lateinit var app: Application

    /** 进程级协程作用域：CoreHolder 的关闭任务、后台控制器等。 */
    val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    private var holder: CoreHolder? = null

    /**
     * 测试中替换为 FakeCoreApi 工厂，Robolectric 下不得加载本地库；
     * 替换时丢弃已创建的 CoreHolder（下次访问用新工厂重建）。
     */
    var coreFactory: (Context) -> CoreApi = { FfiCoreApi.create(it) }
        set(value) {
            synchronized(this) {
                field = value
                holder = null
            }
        }

    /** core 的唯一持有者；ViewModel 只用 [CoreHolder.api]。首次创建 core 只发生在 IO 线程。 */
    val coreHolder: CoreHolder
        get() = synchronized(this) {
            holder ?: CoreHolder(
                factory = { coreFactory(app) },
                scope = appScope,
                io = Dispatchers.IO,
                downloadDir = ::externalDownloadDir,
            ).also { holder = it }
        }

    val localPrefs: LocalPrefs by lazy { LocalPrefs(app) }

    val importer: OutboxImporter by lazy { OutboxImporter(app) }

    val mediaSaver: MediaStoreSaver by lazy { MediaStoreSaver(app) }

    val deviceConditions: DeviceConditions by lazy { DeviceConditions(app, localPrefs, appScope) }

    val backgroundController: BackgroundController by lazy {
        BackgroundController(
            coreHolder, deviceConditions.conditions, appScope, SystemServiceActions(app),
            onModeSeen = localPrefs::setLastBackgroundMode,
        )
    }

    val notifier: Notifier by lazy {
        Notifier(
            app, coreHolder, localPrefs, mediaSaver,
            permissionGranted = {
                (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
                    ContextCompat.checkSelfPermission(app, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) &&
                    app.getSystemService(NotificationManager::class.java).areNotificationsEnabled()
            },
            scope = appScope,
        )
    }

    /** 应用专属外部下载目录下的 LanMsg 子目录（无需存储权限）；外部存储不可用时返回 null。在 IO 线程调用。 */
    private fun externalDownloadDir(): String? {
        val base = app.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: return null
        val dir = File(base, "LanMsg")
        return if (dir.isDirectory || dir.mkdirs()) dir.absolutePath else null
    }
}
