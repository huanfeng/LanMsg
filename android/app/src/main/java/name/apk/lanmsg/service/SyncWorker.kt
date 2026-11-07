package name.apk.lanmsg.service

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import kotlinx.coroutines.delay
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.core.CoreHolder

/** 省电模式下的周期同步：持有 core 约 30 秒，让对端有机会把离线期间的消息补过来。 */
class SyncWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        val holder = ServiceLocator.coreHolder
        try {
            holder.acquire(CoreHolder.WORKER)
            delay(HOLD_MILLIS)
        } finally {
            holder.release(CoreHolder.WORKER)
        }
        return Result.success()
    }

    companion object {
        const val UNIQUE_NAME = "lanmsg-sync"
        private const val HOLD_MILLIS = 30_000L
    }
}
