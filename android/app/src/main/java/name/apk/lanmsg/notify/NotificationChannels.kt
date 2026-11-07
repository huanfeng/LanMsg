package name.apk.lanmsg.notify

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import name.apk.lanmsg.R

/** 四个通知渠道的统一入口（docs/ui-architecture.md §7）。 */
object NotificationChannels {
    const val MESSAGES = "messages"
    const val REQUESTS = "requests"
    const val TRANSFERS = "transfers"
    const val SERVICE = "service"

    /** 幂等创建 4 个渠道；重复调用不会覆盖用户对渠道的设置。 */
    fun ensure(ctx: Context) {
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        nm.createNotificationChannels(
            listOf(
                NotificationChannel(MESSAGES, ctx.getString(R.string.notif_channel_messages), NotificationManager.IMPORTANCE_HIGH),
                NotificationChannel(REQUESTS, ctx.getString(R.string.notif_channel_requests), NotificationManager.IMPORTANCE_HIGH),
                NotificationChannel(TRANSFERS, ctx.getString(R.string.notif_channel_transfers), NotificationManager.IMPORTANCE_LOW),
                NotificationChannel(SERVICE, ctx.getString(R.string.notif_channel_service), NotificationManager.IMPORTANCE_MIN),
            ),
        )
    }
}
