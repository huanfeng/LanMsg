@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.notify

import uniffi.lanmsg_api.TransferFailReason
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreHolder
import name.apk.lanmsg.core.NotifierPrefs
import name.apk.lanmsg.files.MediaStoreSaver
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.MessageBodyView
import uniffi.lanmsg_api.MessageView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.RequestKind
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.StatusDto

private class FakePrefs : NotifierPrefs {
    override val mutedConversations = MutableStateFlow<Set<String>>(emptySet())
    override val notificationsEnabled = MutableStateFlow(true)
    override val saveMediaToGallery = MutableStateFlow(true)
    override val trustedWifi: Flow<Set<String>> = MutableStateFlow(emptySet())

    override suspend fun setMuted(conv: String, muted: Boolean) {
        mutedConversations.value = if (muted) mutedConversations.value + conv else mutedConversations.value - conv
    }

    override suspend fun setNotificationsEnabled(v: Boolean) {
        notificationsEnabled.value = v
    }

    override suspend fun setSaveMediaToGallery(v: Boolean) {
        saveMediaToGallery.value = v
    }

    override suspend fun setTrustedWifi(ssids: Set<String>) = Unit
}

private class RecordingSaver(ctx: Context) : MediaStoreSaver(ctx) {
    val saved = mutableListOf<Triple<String, String, String>>()

    override suspend fun saveToGallery(localPath: String, mime: String, displayName: String): Uri? {
        saved += Triple(localPath, mime, displayName)
        return Uri.parse("content://media/1")
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [34])
class NotifierTest {
    private val ctx: Context = ApplicationProvider.getApplicationContext()
    private val nm = ctx.getSystemService(NotificationManager::class.java)
    private val prefs = FakePrefs()
    private val fake = FakeCoreApi()
    private val saver = RecordingSaver(ctx)
    private var permission = true
    private var foreground = true
    private val foregroundFlow = MutableStateFlow(true)
    private var now = 1_000_000L
    private var selfStatus = StatusDto.ONLINE

    @Before fun setUp() {
        nm.cancelAll()
        foreground = true
        foregroundFlow.value = true
        VisibleConversation.current.value = null
        fake.handler = { req ->
            when (req) {
                is Request.GetSelf -> Response.SelfInfo(Fixtures.selfInfo().apply { status = selfStatus })
                is Request.ListConversations -> Response.Conversations(listOf(Fixtures.conv("c1", "对端", kind = ConvKind.DM), Fixtures.conv("c2", "另一个", kind = ConvKind.DM), Fixtures.conv("self", "")))
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected ${req.javaClass.simpleName}")
            }
        }
    }

    @After fun tearDown() {
        VisibleConversation.current.value = null
    }

    private suspend fun TestScope.notifier(): Notifier {
        val holder = CoreHolder({ fake }, backgroundScope, StandardTestDispatcher(testScheduler))
        holder.acquire(CoreHolder.UI)
        return Notifier(ctx, holder, prefs, saver, { permission }, backgroundScope, { now }, { foreground }, foregroundFlow)
    }

    private fun other(id: String, conv: String = "c1", text: String = "hi"): Event = Event.MessageAdded(Fixtures.textMsg(id, conv, text))

    /** 会话标题：MessagingStyle 的 conversationTitle（系统据此渲染标题）。 */
    private fun titleOf(n: Notification): CharSequence? =
        NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(n)?.conversationTitle

    private fun active() = nm.activeNotifications.toList()

    @Test fun notifies_for_incoming_message() = runTest {
        val n = notifier()
        n.handle(other("m1"))
        val a = active()
        assertEquals(1, a.size)
        assertEquals(NotificationChannels.MESSAGES, a[0].notification.channelId)
        assertEquals("对端", titleOf(a[0].notification))
        assertEquals("c1".hashCode(), a[0].id)
    }

    @Test fun same_conversation_updates_single_notification() = runTest {
        val n = notifier()
        n.handle(other("m1", text = "a"))
        n.handle(other("m2", text = "b"))
        assertEquals(1, active().size)
    }

    @Test fun messaging_style_keeps_last_five_lines() = runTest {
        val n = notifier()
        for (i in 1..7) n.handle(other("m$i", text = "t$i"))
        val msgs = active().single().notification.extras.getParcelableArray(Notification.EXTRA_MESSAGES)
        assertEquals(5, msgs?.size)
    }

    @Test fun no_notification_for_own_message() = runTest {
        notifier().handle(Event.MessageAdded(Fixtures.textMsg("m1", mine = true)))
        assertEquals(0, active().size)
    }

    @Test fun notifies_for_message_from_my_other_device_with_device_name_title() = runTest {
        // core 的 isMine 表示“作者用户是自己”：我的另一台设备发来的消息 isMine=true，但不是本机发的
        val m = Fixtures.textMsg("m1", "self", mine = false).apply { isMine = true }
        notifier().handle(Event.MessageAdded(m))
        val a = active().single()
        assertEquals("self".hashCode(), a.id)
        assertEquals("设备一", titleOf(a.notification))
    }

    @Test fun falls_back_to_is_mine_when_self_device_id_unknown() = runTest {
        fake.handler = { req ->
            when (req) {
                is Request.GetSelf -> throw ApiException(ErrorCode.INTERNAL, "core not running")
                is Request.ListConversations -> Response.Conversations(emptyList())
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected")
            }
        }
        val n = notifier()
        n.handle(Event.MessageAdded(Fixtures.textMsg("m1", mine = true)))
        assertEquals(0, active().size)
        n.handle(other("m2"))
        assertEquals(1, active().size)
    }

    @Test fun no_notification_for_visible_conversation() = runTest {
        val n = notifier()
        VisibleConversation.current.value = "c1"
        n.handle(other("m1", "c1"))
        assertEquals(0, active().size)
        n.handle(other("m2", "c2"))
        assertEquals(1, active().size)
    }

    @Test fun notification_when_visible_conversation_but_app_in_background() = runTest {
        val n = notifier()
        VisibleConversation.current.value = "c1"
        foreground = false
        n.handle(other("m1", "c1"))
        assertEquals(1, active().size)
    }

    @Test fun no_notification_for_pinned_conversation_while_image_viewer_open() = runTest {
        val n = notifier()
        VisibleConversation.pin("c1")
        try {
            n.handle(other("m1", "c1"))
            assertEquals(0, active().size)
        } finally {
            VisibleConversation.unpin("c1")
        }
        n.handle(other("m2", "c1"))
        assertEquals(1, active().size)
    }

    @Test fun no_notification_for_system_message() = runTest {
        val n = notifier()
        n.handle(Event.MessageAdded(Fixtures.systemMsg("m1", "c1")))
        assertEquals(0, active().size)
    }

    @Test fun no_notification_for_muted_conversation() = runTest {
        val n = notifier()
        prefs.setMuted("c1", true)
        n.handle(other("m1", "c1"))
        assertEquals(0, active().size)
    }

    @Test fun no_notification_when_notifications_disabled() = runTest {
        val n = notifier()
        prefs.setNotificationsEnabled(false)
        n.handle(other("m1"))
        assertEquals(0, active().size)
    }

    @Test fun no_notification_when_self_status_busy() = runTest {
        val n = notifier()
        selfStatus = StatusDto.BUSY
        n.handle(other("m1"))
        assertEquals(0, active().size)
        selfStatus = StatusDto.ONLINE
        now += 1_000 // 缓存未过期，仍按 BUSY
        n.handle(other("m2"))
        assertEquals(0, active().size)
        now += 6_000
        n.handle(other("m3"))
        assertEquals(1, active().size)
    }

    @Test fun notifier_skips_when_permission_denied() = runTest {
        val n = notifier()
        permission = false
        n.handle(other("m1"))
        n.handle(Event.IncomingRequest(Fixtures.request()))
        assertEquals(0, active().size)
    }

    @Test fun request_notification_opens_requests_route() = runTest {
        val n = notifier()
        n.handle(Event.IncomingRequest(Fixtures.request("r1", RequestKind.CONTACT, fromName = "小明")))
        val a = active().single()
        assertEquals(NotificationChannels.REQUESTS, a.notification.channelId)
        assertEquals("小明 想加你为联系人", a.notification.extras.getString(Notification.EXTRA_TEXT))
        assertEquals("requests", shadowOf(a.notification.contentIntent).savedIntent.getStringExtra("route"))
    }

    @Test fun message_notification_opens_chat_route() = runTest {
        val n = notifier()
        n.handle(other("m1", "c1"))
        val route = shadowOf(active().single().notification.contentIntent).savedIntent.getStringExtra("route")
        assertEquals("chat/c1", route)
    }

    @Test fun opening_conversation_cancels_its_notification() = runTest {
        val n = notifier()
        n.start()
        n.handle(other("m1", "c1"))
        n.handle(other("m2", "c2"))
        assertEquals(2, active().size)
        VisibleConversation.current.value = "c1"
        runCurrent()
        assertEquals(listOf("c2".hashCode()), active().map { it.id })
    }

    @Test fun two_conversations_have_independent_route_intents() = runTest {
        val n = notifier()
        n.handle(other("m1", "c1"))
        n.handle(other("m2", "c2"))
        val routes = active().associate { it.id to shadowOf(it.notification.contentIntent).savedIntent.getStringExtra("route") }
        assertEquals("chat/c1", routes["c1".hashCode()])
        assertEquals("chat/c2", routes["c2".hashCode()])
    }

    @Test fun duplicate_message_id_is_shown_once() = runTest {
        val n = notifier()
        n.handle(other("m1", text = "a"))
        n.handle(other("m1", text = "a"))
        n.handle(other("m2", text = "b"))
        assertEquals(2, active().single().notification.extras.getParcelableArray(Notification.EXTRA_MESSAGES)?.size)
    }

    @Test fun dismissed_notification_does_not_resurface_old_messages() = runTest {
        val n = notifier()
        n.handle(other("m1", text = "old"))
        nm.cancel("c1".hashCode()) // 用户划掉
        n.handle(other("m2", text = "new"))
        assertEquals(1, active().single().notification.extras.getParcelableArray(Notification.EXTRA_MESSAGES)?.size)
    }

    @Test fun long_body_and_title_are_truncated_by_code_point() = runTest {
        val n = notifier()
        n.handle(other("m1", text = "a".repeat(600)))
        val a = active().single().notification
        assertEquals("a".repeat(500) + "…", a.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
        fake.handler = { req ->
            if (req is Request.ListConversations) Response.Conversations(listOf(Fixtures.conv("c2", "t".repeat(150), kind = ConvKind.DM))) else Response.SelfInfo(Fixtures.selfInfo())
        }
        n.handle(other("m2", "c2"))
        val title = titleOf(active().first { it.id == "c2".hashCode() }.notification).toString()
        assertEquals("t".repeat(100) + "…", title)
    }

    @Test fun truncate_does_not_split_surrogate_pairs() {
        val s = "😀".repeat(5)
        assertEquals("😀😀😀…", Notifier.truncate(s, 3))
        assertEquals(s, Notifier.truncate(s, 5))
    }

    @Test fun returning_to_foreground_cancels_notification_of_pinned_conversation() = runTest {
        val n = notifier()
        n.start()
        runCurrent()
        foreground = false
        foregroundFlow.value = false
        runCurrent()
        VisibleConversation.pin("c1")
        try {
            n.handle(other("m1", "c1"))
            runCurrent()
            assertEquals(1, active().size) // 后台期间照常通知
            foreground = true
            foregroundFlow.value = true
            runCurrent()
            assertEquals(0, active().size)
        } finally {
            VisibleConversation.unpin("c1")
        }
    }

    @Test fun notifier_survives_failing_internal_requests() = runTest {
        val n = notifier()
        fake.handler = { throw ApiException(ErrorCode.INTERNAL, "core not running") }
        n.start()
        runCurrent()
        fake.emit(other("m1", "c1"))
        runCurrent()
        // 标题查不到时退回作者设备名，GetSelf 失败按 ONLINE
        val first = active().single()
        assertEquals("设备一", titleOf(first.notification))
        // 订阅协程仍活着：后续事件照常通知
        fake.emit(other("m2", "c2"))
        runCurrent()
        assertEquals(2, active().size)
    }

    private fun imageMsg(hash: String, mime: String, localPath: String? = null): MessageView {
        val m = Fixtures.fileMsg("f1", hash)
        val item = (m.body as MessageBodyView.Files).items[0]
        item.mime = mime
        item.localPath = localPath
        return m
    }

    private fun serveLocalPath(hash: String, mime: String, path: String?) {
        val base = fake.handler
        fake.handler = { req ->
            if (req is Request.ListMessages) Response.Messages(listOf(imageMsg(hash, mime, path))) else base(req)
        }
    }

    @Test fun transfer_finished_image_saved_to_gallery() = runTest {
        val n = notifier()
        n.handle(Event.MessageAdded(imageMsg("h1", "image/jpeg")))
        serveLocalPath("h1", "image/jpeg", "/data/a.jpg")
        n.handle(Event.TransferFinished("t1", "h1", true, null, null))
        assertEquals(listOf(Triple("/data/a.jpg", "image/jpeg", "a.bin")), saver.saved)
        // 同一次下载不会重复入库
        n.handle(Event.TransferFinished("t1", "h1", true, null, null))
        assertEquals(1, saver.saved.size)
    }

    @Test fun transfer_finished_non_media_is_not_saved() = runTest {
        val n = notifier()
        n.handle(Event.MessageAdded(imageMsg("h2", "application/pdf")))
        serveLocalPath("h2", "application/pdf", "/data/a.pdf")
        n.handle(Event.TransferFinished("t1", "h2", true, null, null))
        assertEquals(0, saver.saved.size)
    }

    @Test fun transfer_finished_skipped_when_failed_unknown_or_pref_off_or_no_local_path() = runTest {
        val n = notifier()
        n.handle(Event.MessageAdded(imageMsg("h3", "image/png")))
        serveLocalPath("h3", "image/png", "/data/b.png")
        n.handle(Event.TransferFinished("t", "h3", false, "boom", TransferFailReason.IO)) // 失败
        n.handle(Event.TransferFinished("t", "unknown", true, null, null)) // 缓存未命中
        assertEquals(0, saver.saved.size)

        prefs.setSaveMediaToGallery(false)
        n.handle(Event.MessageAdded(imageMsg("h4", "image/png")))
        n.handle(Event.TransferFinished("t", "h4", true, null, null))
        assertEquals(0, saver.saved.size)

        prefs.setSaveMediaToGallery(true)
        serveLocalPath("h5", "image/png", null)
        n.handle(Event.MessageAdded(imageMsg("h5", "image/png")))
        n.handle(Event.TransferFinished("t", "h5", true, null, null))
        assertEquals(0, saver.saved.size)
    }
}
