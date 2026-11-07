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
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** 控制器对系统做的两件事，抽成接口便于测试。 */
interface ServiceActions {
    fun setServiceRunning(run: Boolean)

    fun setWorkerScheduled(scheduled: Boolean)
}

class SystemServiceActions(ctx: Context) : ServiceActions {
    private val app = ctx.applicationContext

    override fun setServiceRunning(run: Boolean) {
        val intent = Intent(app, LanMsgService::class.java)
        if (run) {
            try {
                ContextCompat.startForegroundService(app, intent)
            } catch (e: IllegalStateException) {
                // Android 12+ 后台中不允许启动前台服务；回到前台或下一次条件变化时再试
                Log.w(TAG, "Cannot start foreground service now", e)
            } catch (e: SecurityException) {
                Log.w(TAG, "Foreground service start denied", e)
            }
        } else {
            app.stopService(intent)
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
) {
    private val started = AtomicBoolean(false)
    private var lastRun: Boolean? = null
    private var lastWorker: Boolean? = null

    fun start() {
        if (!started.compareAndSet(false, true)) return
        scope.launch {
            combine(holder.backgroundMode.filterNotNull(), conditions) { m, c -> m to c }.collect { (m, c) ->
                val run = BackgroundPolicy.shouldRunService(m, c)
                if (run != lastRun) {
                    lastRun = run
                    actions.setServiceRunning(run)
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
