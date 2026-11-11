package name.apk.lanmsg.update

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import name.apk.lanmsg.MainActivity
import name.apk.lanmsg.R
import name.apk.lanmsg.core.CoreHolder
import name.apk.lanmsg.core.UpdatePrefs
import name.apk.lanmsg.core.call
import name.apk.lanmsg.notify.NotificationChannels
import name.apk.lanmsg.notify.Notifier
import name.apk.lanmsg.ui.nav.Routes
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.UpdateInfo
import java.util.concurrent.atomic.AtomicBoolean

/**
 * 启动自动检查更新：core 每次被任意 owner（服务/界面/worker）拉起都尝试检查，24 小时内不重复
 * （附录 T23：启动自动检查，每天最多一次；节流兜底跨翻转生效）。请求失败（core 未初始化、无网络等）
 * 只记日志，绝不影响启动。有新版本时在“应用更新”渠道发通知，点击进设置页。
 */
class UpdateChecker(
    ctx: Context,
    private val holder: CoreHolder,
    private val prefs: UpdatePrefs,
    private val scope: CoroutineScope,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    private val ctx = ctx.applicationContext
    private val nm = this.ctx.getSystemService(NotificationManager::class.java)

    /** 通知 id 固定：重复检查同一更新时只刷新这一条。 */
    internal val NOTIF_ID = 40

    /** 正在检查：防止快速关-开 core（或与手动路径）并发 runCheck 双写时间戳/双发请求。 */
    private val checking = AtomicBoolean(false)

    /** 观察循环已启动：重复调用 start() 不再挂新的循环。 */
    private val loopStarted = AtomicBoolean(false)

    /**
     * 每次 core 运行（false→true 翻转）都尝试检查：runCheck 完成后等 core 关闭，再观察下一次
     * 拉起，循环往复；“每天最多一次”由 24h 节流兜底。重复调用 start() 只挂一个观察循环。
     */
    fun start() {
        if (!loopStarted.compareAndSet(false, true)) return
        scope.launch {
            while (true) {
                holder.running.filter { it }.first()
                runCheck()
                // 等本轮 core 关闭再观察下一次拉起；进程存活期间 core 反复启停时每轮都会尝试
                holder.running.filter { !it }.first()
            }
        }
    }

    /**
     * 一次自动检查：24h 节流（无论结果）→ 读开关 → CheckUpdate → 有更新发通知。
     * 返回 null = 被节流/开关关闭/失败/已在检查中；true = 发了通知；false = 已是最新。
     * 入口原子去重：并发调用只执行一个，其余直接返回 null。
     */
    internal suspend fun runCheck(): Boolean? {
        if (!checking.compareAndSet(false, true)) {
            Log.i(TAG, "Auto update check already in progress; skipped")
            return null
        }
        try {
            val now = clock()
            val last = prefs.lastAutoUpdateCheck.first()
            if (last != null && now - last < AUTO_CHECK_INTERVAL_MS) {
                Log.i(TAG, "Auto update check throttled (last check ${now - last}ms ago)")
                return null
            }
            prefs.setLastAutoUpdateCheck(now)
            val settings = try {
                holder.api.call<Response.Settings>(Request.GetSettings).value
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.i(TAG, "Auto update check skipped: settings unavailable: ${e.message}")
                return null
            }
            if (!settings.autoCheckUpdate) return null
            val view = try {
                holder.api.call<Response.UpdateCheck>(Request.CheckUpdate).value
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "Auto update check failed: ${e.message}")
                return null
            }
            val info: UpdateInfo = view.update ?: return false
            notify(info)
            return true
        } finally {
            checking.set(false)
        }
    }

    /** “应用更新”渠道通知：标题带新版本号，点击进设置页看更新说明。 */
    private fun notify(info: UpdateInfo) {
        NotificationChannels.ensure(ctx)
        val notes = Notifier.truncate(info.notes.trim(), MAX_NOTES_CODE_POINTS)
        val n = NotificationCompat.Builder(ctx, NotificationChannels.UPDATES)
            .setSmallIcon(R.drawable.ic_stat_lanmsg)
            .setContentTitle(ctx.getString(R.string.notif_update_title, info.version))
            .setContentText(ctx.getString(R.string.notif_update_click_hint))
            .setAutoCancel(true)
            .setStyle(NotificationCompat.BigTextStyle().bigText(notes))
            .setCategory(NotificationCompat.CATEGORY_SYSTEM)
            .setLocalOnly(true)
            .setContentIntent(openSettings())
            .build()
        nm.notify(NOTIF_ID, n)
    }

    /** 通知点击打开设置页（MainActivity 的路由白名单包含 settings）。 */
    private fun openSettings(): PendingIntent {
        val intent = Intent(ctx, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(MainActivity.EXTRA_ROUTE, Routes.SETTINGS)
        return PendingIntent.getActivity(ctx, NOTIF_ID, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    private companion object {
        /** 与 core 侧自动检查同款节流间隔：每天最多一次。 */
        const val AUTO_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000L
        const val MAX_NOTES_CODE_POINTS = 300
        const val TAG = "UpdateChecker"
    }
}
