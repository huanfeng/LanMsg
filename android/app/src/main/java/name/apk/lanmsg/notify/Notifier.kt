package name.apk.lanmsg.notify

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import name.apk.lanmsg.MainActivity
import name.apk.lanmsg.R
import name.apk.lanmsg.core.CoreHolder
import name.apk.lanmsg.core.NotifierPrefs
import name.apk.lanmsg.core.call
import name.apk.lanmsg.files.MediaStoreSaver
import name.apk.lanmsg.ui.nav.Routes
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.FileItemView
import uniffi.lanmsg_api.MessageBodyView
import uniffi.lanmsg_api.MessageView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.RequestKind
import uniffi.lanmsg_api.RequestView
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.StatusDto

/**
 * 订阅 core 事件并发系统通知，同时处理“下载完成自动入相册”。规则见 docs/ui-architecture.md §7。
 * 内部对 core 的请求可能因 core 未运行等原因失败：一律记日志并降级，绝不让订阅协程终止。
 */
class Notifier(
    ctx: Context,
    private val holder: CoreHolder,
    private val prefs: NotifierPrefs,
    private val saver: MediaStoreSaver,
    private val permissionGranted: () -> Boolean,
    private val scope: CoroutineScope,
    private val clock: () -> Long = System::currentTimeMillis,
    /** App 是否在前台（有 Activity 处于 STARTED）；测试注入。 */
    private val appVisible: () -> Boolean = {
        ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
    },
    /** App 前后台变化流；回到前台时据此取消“后台期间被 pin 的会话”弹出的通知。测试注入。 */
    private val appForeground: Flow<Boolean> = flow {
        emitAll(ProcessLifecycleOwner.get().lifecycle.currentStateFlow.map { it.isAtLeast(Lifecycle.State.STARTED) })
    },
) {
    private val ctx = ctx.applicationContext
    private val nm = this.ctx.getSystemService(NotificationManager::class.java)

    /** 本人状态缓存：读取时刻 + 值；null = 还没读过。 */
    @Volatile private var statusReadAt: Long? = null
    @Volatile private var status = StatusDto.ONLINE

    /** 本机设备 id；与 [status] 同时刷新，读取失败时为 null。 */
    @Volatile private var selfDeviceId: String? = null

    /** 上次记日志时的 (通知开关, 权限)；只在变化时记一次，避免每条消息都刷日志。 */
    @Volatile private var lastGate: Pair<Boolean, Boolean> = true to true

    /** 本机设备 id 未知的告警是否已记过；拿到 id 后复位。 */
    @Volatile private var selfUnknownLogged = false

    private val lock = Any()

    /** 同会话最近几条消息，用于 MessagingStyle 累积。 */
    private val history = HashMap<String, ArrayDeque<Line>>()

    /** 按 hash 索引的最近出现过的文件条目（访问序 LRU），用于下载完成后自动入相册。 */
    private val fileCache = object : LinkedHashMap<String, CachedFile>(64, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, CachedFile>) = size > FILE_CACHE_SIZE
    }

    private class Line(val id: String, val text: String, val millis: Long, val sender: String)

    private class CachedFile(val conv: String, val item: FileItemView)

    fun start() {
        NotificationChannels.ensure(ctx)
        scope.launch { refreshStatus() }
        scope.launch {
            holder.api.events.collect { e ->
                try {
                    handle(e)
                } catch (ex: CancellationException) {
                    throw ex
                } catch (ex: Exception) {
                    Log.w(TAG, "Failed to handle event ${e.javaClass.simpleName}", ex)
                }
            }
        }
        // 打开某会话（或图片查看页盖在其上）时取消它的通知；App 在后台时不算可见，回到前台再取消
        scope.launch {
            combine(VisibleConversation.visible, appForeground) { convs, fg -> if (fg) convs else emptySet() }
                .collect { convs -> convs.forEach { cancelConversation(it) } }
        }
    }

    internal suspend fun handle(e: Event) {
        when (e) {
            is Event.MessageAdded -> {
                cacheFiles(e.msg)
                onMessage(e.msg)
            }
            is Event.MessageUpdated -> cacheFiles(e.msg)
            is Event.IncomingRequest -> onRequest(e.request)
            is Event.TransferFinished -> if (e.ok) onTransferFinished(e.hash)
            else -> Unit
        }
    }

    private suspend fun onMessage(msg: MessageView) {
        // 先取状态（顺带刷新本机设备 id），再判断是否本机发出
        if (currentStatus() == StatusDto.BUSY) return
        if (isFromThisDevice(msg)) return
        if (!canNotify()) return
        if (msg.conv in prefs.mutedConversations.first()) return
        if (appVisible() && VisibleConversation.isVisible(msg.conv)) return
        val body = truncate(bodyText(msg.body) ?: return, MAX_BODY_CODE_POINTS)
        val conv = conversationOf(msg.conv)
        // “我的设备”会话与桌面一致：标题用发送设备名
        val title = truncate(
            if (conv == null || conv.kind == ConvKind.SELF_DEVICES) msg.authorDeviceName else conv.title,
            MAX_TITLE_CODE_POINTS,
        )
        val id = msg.conv.hashCode()
        // 用户划掉通知不会通知我们：通知已不在栏里时，累积的旧消息要丢弃
        // activeNotifications 是 binder 调用，放在锁外读取
        val active = nm.activeNotifications.any { it.id == id }
        val lines = synchronized(lock) {
            if (!active) history.remove(msg.conv)
            val q = history.getOrPut(msg.conv) { ArrayDeque() }
            if (q.none { it.id == msg.id }) q.addLast(Line(msg.id, body, msg.hlcMillis, msg.authorDeviceName))
            while (q.size > HISTORY_SIZE) q.removeFirst()
            q.toList()
        }
        val style = NotificationCompat.MessagingStyle(Person.Builder().setName(ctx.getString(R.string.app_name)).build())
            .setConversationTitle(title)
        lines.forEach { style.addMessage(it.text, it.millis, Person.Builder().setName(it.sender).build()) }
        val n = NotificationCompat.Builder(ctx, NotificationChannels.MESSAGES)
            .setSmallIcon(R.drawable.ic_stat_lanmsg)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(style)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setContentIntent(openRoute(id, Routes.chat(msg.conv)))
            .build()
        nm.notify(id, n)
    }

    private suspend fun onRequest(r: RequestView) {
        if (!canNotify()) return
        val kind = ctx.getString(
            when (r.kind) {
                RequestKind.CONTACT -> R.string.req_kind_contact
                RequestKind.MESSAGE -> R.string.req_kind_message
                RequestKind.FILE -> R.string.req_kind_file
            },
        )
        val id = r.request.hashCode()
        val n = NotificationCompat.Builder(ctx, NotificationChannels.REQUESTS)
            .setSmallIcon(R.drawable.ic_stat_lanmsg)
            .setContentTitle(ctx.getString(R.string.app_name))
            .setContentText(ctx.getString(R.string.notif_new_request, r.fromName, kind))
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setContentIntent(openRoute(id, Routes.REQUESTS))
            .build()
        nm.notify(id, n)
    }

    private suspend fun onTransferFinished(hash: String) {
        val cached = synchronized(lock) { fileCache.remove(hash) } ?: return
        val item = cached.item
        if (!(item.mime.startsWith("image/") || item.mime.startsWith("video/"))) return
        if (!prefs.saveMediaToGallery.first()) return
        try {
            val msgs = holder.api.call<Response.Messages>(Request.ListMessages(cached.conv, null, LOOKUP_LIMIT)).value
            // 同一文件可能出现在多条消息里，取已有本地路径的那个
            val local = msgs.firstNotNullOfOrNull { findItem(it.body, hash)?.localPath?.takeIf { p -> p.isNotEmpty() } } ?: return
            if (saver.saveToGallery(local, item.mime, item.name) == null) {
                Log.w(TAG, "Auto save to gallery returned no result for ${item.name}")
            }
        } catch (ex: CancellationException) {
            throw ex
        } catch (ex: Exception) {
            Log.w(TAG, "Auto save to gallery failed for ${item.name}", ex)
        }
    }

    private fun cacheFiles(msg: MessageView) {
        if (isFromThisDevice(msg)) return
        val items = filesOf(msg.body)
        if (items.isEmpty()) return
        synchronized(lock) { items.forEach { fileCache[it.hash] = CachedFile(msg.conv, it) } }
    }

    private fun filesOf(body: MessageBodyView): List<FileItemView> = when (body) {
        is MessageBodyView.Files -> body.items
        is MessageBodyView.Folder -> body.items
        else -> emptyList()
    }

    private fun findItem(body: MessageBodyView, hash: String): FileItemView? = filesOf(body).firstOrNull { it.hash == hash }

    private fun bodyText(body: MessageBodyView): String? = when (body) {
        is MessageBodyView.Text -> body.text
        // 系统消息只在会话内显示，不弹通知
        is MessageBodyView.System -> null
        is MessageBodyView.Files -> ctx.getString(R.string.notif_new_files, body.items.firstOrNull()?.name.orEmpty())
        is MessageBodyView.Folder -> ctx.getString(R.string.notif_new_files, body.name)
        is MessageBodyView.Unsupported -> null
    }

    private suspend fun canNotify(): Boolean {
        val enabled = prefs.notificationsEnabled.first()
        val granted = permissionGranted()
        val gate = enabled to granted
        if (gate != lastGate) {
            lastGate = gate
            if (!enabled || !granted) Log.w(TAG, "Notification suppressed: enabledInSettings=$enabled permissionGranted=$granted")
        }
        return enabled && granted
    }

    /**
     * 是否本机发出的消息。core 的 isMine 表示“作者用户是自己”，会把“我的设备”里其他设备的消息也算进去，
     * 所以优先比较作者设备与本机设备 id；本机设备 id 未知时退回 isMine。
     */
    private fun isFromThisDevice(msg: MessageView): Boolean {
        val self = selfDeviceId
        if (self.isNullOrEmpty()) {
            if (!selfUnknownLogged) {
                selfUnknownLogged = true
                Log.w(TAG, "Self device id unknown, falling back to isMine")
            }
            return msg.isMine
        }
        return msg.authorDevice == self
    }

    private suspend fun conversationOf(conv: String): ConversationSummary? = try {
        holder.api.call<Response.Conversations>(Request.ListConversations).value.firstOrNull { it.conv == conv }
    } catch (ex: CancellationException) {
        throw ex
    } catch (ex: Exception) {
        Log.w(TAG, "ListConversations failed, using author device name: ${ex.message}")
        null
    }

    /** 状态缓存 5 秒；读取失败按 ONLINE（宁可多通知，不漏通知）。 */
    private suspend fun currentStatus(): StatusDto {
        val at = statusReadAt
        if (at == null || clock() - at > STATUS_TTL_MS) refreshStatus()
        return status
    }

    private suspend fun refreshStatus() {
        status = try {
            val info = holder.api.call<Response.SelfInfo>(Request.GetSelf).value
            selfDeviceId = info.deviceId
            selfUnknownLogged = false
            info.status
        } catch (ex: CancellationException) {
            throw ex
        } catch (ex: Exception) {
            Log.w(TAG, "GetSelf failed, assuming ONLINE: ${ex.message}")
            StatusDto.ONLINE
        }
        statusReadAt = clock()
    }

    private fun cancelConversation(conv: String) {
        synchronized(lock) { history.remove(conv) }
        nm.cancel(conv.hashCode())
    }

    private fun openRoute(requestCode: Int, route: String): PendingIntent {
        val intent = Intent(ctx, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(MainActivity.EXTRA_ROUTE, route)
        return PendingIntent.getActivity(ctx, requestCode, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    internal companion object {
        /** 按码点截断（不拆代理对），超长时末尾加省略号。 */
        fun truncate(s: String, maxCodePoints: Int): String =
            if (s.codePointCount(0, s.length) <= maxCodePoints) s else s.substring(0, s.offsetByCodePoints(0, maxCodePoints)) + "…"

        const val TAG = "Notifier"
        const val STATUS_TTL_MS = 5_000L
        const val FILE_CACHE_SIZE = 500
        const val HISTORY_SIZE = 5
        const val MAX_BODY_CODE_POINTS = 500
        const val MAX_TITLE_CODE_POINTS = 100
        const val LOOKUP_LIMIT = 50u
    }
}
