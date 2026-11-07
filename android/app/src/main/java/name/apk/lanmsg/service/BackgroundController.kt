package name.apk.lanmsg.service

import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import name.apk.lanmsg.core.CoreHolder
import uniffi.lanmsg_api.BackgroundMode
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** 控制器对系统做的两件事，抽成接口便于测试。 */
interface ServiceActions {
    /** 返回 false 表示系统拒绝了启动（如后台限制），调用方应在合适时机重试。 */
    fun setServiceRunning(run: Boolean): Boolean

    fun setWorkerScheduled(scheduled: Boolean)
}

class SystemServiceActions(ctx: Context) : ServiceActions {
    private val app = ctx.applicationContext

    override fun setServiceRunning(run: Boolean): Boolean {
        val intent = Intent(app, LanMsgService::class.java)
        if (!run) {
            app.stopService(intent)
            return true
        }
        return try {
            ContextCompat.startForegroundService(app, intent)
            true
        } catch (e: IllegalStateException) {
            // Android 12+ 后台中不允许启动前台服务；回到前台时由控制器重试
            Log.w(TAG, "Cannot start foreground service now", e)
            false
        } catch (e: SecurityException) {
            Log.w(TAG, "Foreground service start denied", e)
            false
        }
    }

    override fun setWorkerScheduled(scheduled: Boolean) {
        val wm = WorkManager.getInstance(app)
        if (scheduled) {
            wm.enqueueUniquePeriodicWork(
                SyncWorker.UNIQUE_NAME,
                ExistingPeriodicWorkPolicy.KEEP,
                PeriodicWorkRequestBuilder<SyncWorker>(15, TimeUnit.MINUTES).build(),
            )
        } else {
            wm.cancelUniqueWork(SyncWorker.UNIQUE_NAME)
        }
    }

    private companion object {
        const val TAG = "BackgroundController"
    }
}

/**
 * 订阅后台档位与设备条件，按 [BackgroundPolicy] 启停 [LanMsgService]、排/撤 [SyncWorker]。
 * 档位未知（core 未初始化）时什么都不做。[start] 可重复调用。
 */
class BackgroundController(
    private val holder: CoreHolder,
    private val conditions: StateFlow<Conditions>,
    private val scope: CoroutineScope,
    private val actions: ServiceActions,
    /** 启动时读完档位后，最多等多久让服务接手 core，避免 core 先关再开。 */
    private val serviceWaitMillis: Long = 10_000,
    /** 读到或收到新档位时记下来，供开机时先于 core 就绪拉起服务。 */
    private val onModeSeen: suspend (BackgroundMode) -> Unit = {},
) {
    private val started = AtomicBoolean(false)
    private var lastRun: Boolean? = null
    private var lastWorker: Boolean? = null

    /** 上次启动服务被系统拒绝；回到前台前不再重试。 */
    private var startFailed = false

    fun start() {
        if (!started.compareAndSet(false, true)) return
        scope.launch {
            combine(holder.backgroundMode.filterNotNull(), conditions) { m, c -> m to c }.collect { (m, c) ->
                onModeSeen(m)
                val run = BackgroundPolicy.shouldRunService(m, c)
                val blocked = startFailed && run && !c.appVisible
                if (!blocked && run != lastRun) {
                    startFailed = false
                    if (actions.setServiceRunning(run)) {
                        lastRun = run
                    } else {
                        lastRun = null
                        startFailed = true
                    }
                }
                val worker = BackgroundPolicy.shouldScheduleWorker(m)
                if (worker != lastWorker) {
                    lastWorker = worker
                    actions.setWorkerScheduled(worker)
                }
            }
        }
        scope.launch { readInitialMode() }
    }

    /** 进程刚起来（开机、被系统重启）时界面可能不在：自己短暂持有 core 读一次档位。 */
    private suspend fun readInitialMode() {
        try {
            holder.acquire(CoreHolder.CONTROLLER)
            try {
                holder.refreshBackgroundMode()
                val mode = holder.backgroundMode.value
                if (mode != null && BackgroundPolicy.shouldRunService(mode, conditions.value)) {
                    withTimeoutOrNull(serviceWaitMillis) { holder.owners.first { CoreHolder.SERVICE in it } }
                }
            } finally {
                holder.release(CoreHolder.CONTROLLER)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Log.w("BackgroundController", "Failed to read background mode", e)
        }
    }
}
