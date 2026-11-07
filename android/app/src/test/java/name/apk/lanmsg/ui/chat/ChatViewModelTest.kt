@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.chat

import uniffi.lanmsg_api.TransferFailReason
import name.apk.lanmsg.R
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.notify.VisibleConversation
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import androidx.test.ext.junit.runners.AndroidJUnit4
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.MessageState
import uniffi.lanmsg_api.MessageView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class ChatViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    @Before fun resetVisible() {
        VisibleConversation.current.value = null
    }

    private fun m(id: String, conv: String = "c1", mine: Boolean = false, millis: Long = 1000) =
        Fixtures.textMsg(id, conv, "t-$id", mine, millis)

    private fun fakeWith(latest: () -> List<MessageView>) = FakeCoreApi().apply {
        handler = { r ->
            when (r) {
                is Request.ListConversations -> Response.Conversations(listOf(Fixtures.conv("c1", "对端")))
                is Request.ListMessages -> Response.Messages(latest())
                is Request.DownloadBlob -> Response.TransferId("t-dl")
                else -> Response.Ok
            }
        }
    }

    private fun ChatViewModel.ids() = state.value.messages.map { it.id }

    @Test fun initial_load_requests_latest_50() = runTest {
        val fake = fakeWith { listOf(m("m1")) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        assertTrue(fake.requests.contains(Request.ListMessages("c1", null, 50u)))
        assertEquals(listOf("m1"), vm.ids())
        assertEquals("对端", vm.state.value.title)
    }

    @Test fun message_added_twice_is_shown_once() = runTest {
        val fake = fakeWith { emptyList() }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        fake.emit(Event.MessageAdded(m("m1")))
        fake.emit(Event.MessageAdded(m("m1")))
        advanceUntilIdle()
        assertEquals(1, vm.state.value.messages.size)
    }

    @Test fun message_added_for_other_conv_ignored() = runTest {
        val fake = fakeWith { emptyList() }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        fake.emit(Event.MessageAdded(m("x1", conv = "c2")))
        advanceUntilIdle()
        assertTrue(vm.state.value.messages.isEmpty())
    }

    @Test fun message_updated_replaces_by_id() = runTest {
        val fake = fakeWith { listOf(m("m1")) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        assertEquals(MessageState.SENT, vm.state.value.messages[0].state)
        fake.emit(Event.MessageUpdated(m("m1").copy(state = MessageState.DELIVERED)))
        advanceUntilIdle()
        assertEquals(listOf("m1"), vm.ids())
        assertEquals(MessageState.DELIVERED, vm.state.value.messages[0].state)
    }

    @Test fun reload_in_flight_does_not_overwrite_newer_message_update() = runTest {
        val gate = CompletableDeferred<Unit>()
        var calls = 0
        val fake = FakeCoreApi().apply {
            handler = { r ->
                when (r) {
                    is Request.ListConversations -> Response.Conversations(listOf(Fixtures.conv("c1", "对端")))
                    is Request.ListMessages -> {
                        if (calls++ > 0) gate.await()
                        Response.Messages(listOf(m("m1")))
                    }
                    else -> Response.Ok
                }
            }
        }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        fake.emitLagged()
        advanceUntilIdle()
        fake.emit(Event.MessageUpdated(m("m1").copy(state = MessageState.DELIVERED)))
        advanceUntilIdle()
        gate.complete(Unit)
        advanceUntilIdle()
        assertEquals(MessageState.DELIVERED, vm.state.value.messages.single().state)
    }

    @Test fun lagged_triggers_full_reload_without_duplicates() = runTest {
        var current = listOf(m("m1"), m("m2"))
        val fake = fakeWith { current }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        assertEquals(listOf("m1", "m2"), vm.ids())
        current = listOf(m("m1"), m("m2"), m("m3"))
        fake.emitLagged()
        advanceUntilIdle()
        assertEquals(listOf("m1", "m2", "m3"), vm.ids())
    }

    @Test fun load_more_uses_oldest_id_and_prepends() = runTest {
        val fake = FakeCoreApi().apply {
            var olderCalls = 0
            handler = { r ->
                when {
                    r is Request.ListConversations -> Response.Conversations(emptyList())
                    r is Request.ListMessages && r.before == null -> Response.Messages(listOf(m("m3"), m("m4")))
                    r is Request.ListMessages -> Response.Messages(if (olderCalls++ == 0) listOf(m("m1"), m("m2")) else emptyList())
                    else -> Response.Ok
                }
            }
        }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        vm.loadMore(); advanceUntilIdle()
        assertTrue(fake.requests.contains(Request.ListMessages("c1", "m3", 50u)))
        assertEquals(listOf("m1", "m2", "m3", "m4"), vm.ids())
        assertTrue(vm.state.value.hasMore)
        vm.loadMore(); advanceUntilIdle()
        assertFalse(vm.state.value.hasMore)
        assertEquals(listOf("m1", "m2", "m3", "m4"), vm.ids())
    }

    @Test fun send_text_clears_input_and_ignores_blank() = runTest {
        val fake = fakeWith { emptyList() }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        vm.setInput("  "); vm.sendText(); advanceUntilIdle()
        assertTrue(fake.requests.none { it is Request.SendText })
        vm.setInput("hi"); vm.sendText(); advanceUntilIdle()
        assertEquals(Request.SendText("c1", "hi"), fake.requests.last())
        assertEquals("", vm.state.value.input)
    }

    @Test fun send_text_failure_restores_input_and_flags_peer_offline() = runTest {
        val fake = fakeWith { emptyList() }
        val base = fake.handler
        fake.handler = { r -> if (r is Request.SendText) throw ApiException(ErrorCode.PEER_OFFLINE, "offline") else base(r) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        vm.setInput("hi"); vm.sendText(); advanceUntilIdle()
        assertEquals("hi", vm.state.value.input)
        assertEquals(R.string.err_peer_offline, vm.state.value.error)
    }

    @Test fun mark_read_sent_once_per_last_id_when_visible() = runTest {
        val fake = fakeWith { listOf(m("m1"), m("m2")) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        vm.onVisible(); advanceUntilIdle()
        assertEquals(listOf(Request.MarkRead("c1", "m2")), fake.requests.filterIsInstance<Request.MarkRead>())
        assertEquals("c1", VisibleConversation.current.value)
        fake.emit(Event.MessageUpdated(m("m2"))); advanceUntilIdle()
        assertEquals(1, fake.requests.count { it is Request.MarkRead })
        fake.emit(Event.MessageAdded(m("m3"))); advanceUntilIdle()
        assertEquals(Request.MarkRead("c1", "m3"), fake.requests.last { it is Request.MarkRead })
        vm.onHidden(); advanceUntilIdle()
        assertNull(VisibleConversation.current.value)
        fake.emit(Event.MessageAdded(m("m4"))); advanceUntilIdle()
        assertEquals(2, fake.requests.count { it is Request.MarkRead })
    }

    @Test fun mark_read_failure_is_silent_and_retried_on_next_message() = runTest {
        val fake = fakeWith { listOf(m("m1")) }
        val base = fake.handler
        var failing = true
        fake.handler = { r ->
            if (r is Request.MarkRead && failing) throw ApiException(ErrorCode.NOT_FOUND, "message not found") else base(r)
        }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        vm.onVisible(); advanceUntilIdle()
        assertNull(vm.state.value.error)
        failing = false
        fake.emit(Event.MessageAdded(m("m2"))); advanceUntilIdle()
        assertNull(vm.state.value.error)
        assertEquals(Request.MarkRead("c1", "m2"), fake.requests.last { it is Request.MarkRead })
        assertEquals(2, fake.requests.count { it is Request.MarkRead })
    }

    @Test fun import_and_send_flags_error_when_all_imports_fail() = runTest {
        val vm = ChatViewModel(fakeWith { emptyList() }, "c1", VisibleConversation)
        advanceUntilIdle()
        val importer = object : name.apk.lanmsg.files.Importer {
            override suspend fun import(uris: List<android.net.Uri>) = emptyList<String>()
        }
        vm.importAndSend(importer, listOf(android.net.Uri.EMPTY)); advanceUntilIdle()
        assertEquals(R.string.share_import_failed, vm.state.value.error)
    }

    @Test fun can_recall_only_own_within_two_minutes() = runTest {
        val vm = ChatViewModel(fakeWith { emptyList() }, "c1", VisibleConversation)
        val mine = m("m1", mine = true, millis = 1000)
        assertTrue(vm.canRecall(mine, 1000 + 119_000))
        assertTrue(vm.canRecall(mine, 1000 + 120_000))
        assertFalse(vm.canRecall(mine, 1000 + 120_001))
        assertFalse(vm.canRecall(m("m2", mine = false, millis = 1000), 1000 + 1_000))
        assertFalse(vm.canRecall(mine.copy(state = MessageState.RECALLED), 1000 + 1_000))
    }

    @Test fun transfer_progress_then_finished_reloads() = runTest {
        val fake = fakeWith { listOf(m("m1")) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        val before = fake.requests.count { it is Request.ListMessages }
        fake.emit(Event.TransferProgress("t1", "h", 5uL, 10uL, 0uL)); advanceUntilIdle()
        assertEquals(5uL to 10uL, vm.state.value.progress["h"])
        fake.emit(Event.TransferFinished("t1", "h", true, null, null)); advanceUntilIdle()
        assertTrue(vm.state.value.progress.isEmpty())
        assertEquals(before + 1, fake.requests.count { it is Request.ListMessages })
    }

    @Test fun failed_transfer_sets_error_and_clears_progress() = runTest {
        val fake = fakeWith { listOf(m("m1")) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        fake.emit(Event.TransferProgress("t1", "h", 5uL, 10uL, 0uL)); advanceUntilIdle()
        fake.emit(Event.TransferFinished("t1", "h", false, "connection reset", TransferFailReason.SOURCE_OFFLINE)); advanceUntilIdle()
        assertEquals(R.string.download_source_offline, vm.state.value.error)
        assertTrue(vm.state.value.progress.isEmpty())
    }

    @Test fun user_cancelled_transfer_does_not_set_error() = runTest {
        val fake = fakeWith { listOf(m("m1")) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        fake.emit(Event.TransferProgress("t1", "h", 5uL, 10uL, 0uL)); advanceUntilIdle()
        vm.cancel("h"); advanceUntilIdle()
        fake.emit(Event.TransferFinished("t1", "h", false, "cancelled", TransferFailReason.CANCELLED)); advanceUntilIdle()
        assertNull(vm.state.value.error)
    }

    @Test fun download_request_failure_sets_error() = runTest {
        val fake = fakeWith { listOf(m("m1")) }
        val ok = fake.handler
        fake.handler = { r -> if (r is Request.DownloadBlob) throw ApiException(ErrorCode.PEER_OFFLINE, "offline") else ok(r) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        vm.download("h"); advanceUntilIdle()
        assertEquals(R.string.err_peer_offline, vm.state.value.error)
    }

    @Test fun failed_cancel_request_does_not_swallow_later_real_failure() = runTest {
        val fake = fakeWith { listOf(m("m1")) }
        val ok = fake.handler
        fake.handler = { r -> if (r is Request.CancelTransfer) throw ApiException(ErrorCode.IO, "no cancel") else ok(r) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        fake.emit(Event.TransferProgress("t1", "h", 5uL, 10uL, 0uL)); advanceUntilIdle()
        vm.cancel("h"); advanceUntilIdle()
        fake.emit(Event.TransferFinished("t1", "h", false, "reset", TransferFailReason.IO)); advanceUntilIdle()
        assertEquals(R.string.download_io_failed, vm.state.value.error)
    }

    @Test fun new_download_clears_stale_cancelled_mark() = runTest {
        val fake = fakeWith { listOf(m("m1")) }
        val ok = fake.handler
        fake.handler = { r -> if (r is Request.DownloadBlob) Response.TransferId("t1") else ok(r) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        fake.emit(Event.TransferProgress("t1", "h", 5uL, 10uL, 0uL)); advanceUntilIdle()
        vm.cancel("h"); advanceUntilIdle()
        vm.download("h"); advanceUntilIdle()
        fake.emit(Event.TransferFinished("t1", "h", false, "reset", TransferFailReason.IO)); advanceUntilIdle()
        assertEquals(R.string.download_io_failed, vm.state.value.error)
    }

    @Test fun cancel_uses_transfer_id_from_progress() = runTest {
        val fake = fakeWith { emptyList() }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        fake.emit(Event.TransferProgress("t9", "h", 1uL, 10uL, 0uL)); advanceUntilIdle()
        vm.cancel("h"); advanceUntilIdle()
        assertEquals(Request.CancelTransfer("t9"), fake.requests.last())
    }

    @Test fun delete_local_removes_message() = runTest {
        val fake = fakeWith { listOf(m("m1"), m("m2")) }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        vm.deleteLocal("m1"); advanceUntilIdle()
        assertEquals(Request.DeleteLocal(listOf("m1"), false), fake.requests.last())
        assertEquals(listOf("m2"), vm.ids())
    }

    @Test fun delete_local_during_reload_is_not_resurrected_by_stale_response() = runTest {
        val gate = CompletableDeferred<Unit>()
        var calls = 0
        val fake = FakeCoreApi().apply {
            handler = { r ->
                when (r) {
                    is Request.ListConversations -> Response.Conversations(listOf(Fixtures.conv("c1", "对端")))
                    is Request.ListMessages -> {
                        val n = calls++
                        if (n > 0) gate.await()
                        Response.Messages(listOf(m("m1"), m("m2")))
                    }
                    else -> Response.Ok
                }
            }
        }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        assertEquals(listOf("m1", "m2"), vm.ids())
        fake.emitLagged() // 触发重拉并挂起在 gate
        advanceUntilIdle()
        vm.deleteLocal("m1"); advanceUntilIdle()
        assertEquals(listOf("m2"), vm.ids())
        gate.complete(Unit) // 放行含 m1 的旧响应
        advanceUntilIdle()
        assertEquals(listOf("m2"), vm.ids())
    }

    @Test fun import_and_send_sends_imported_paths_and_skips_when_empty() = runTest {
        val fake = fakeWith { emptyList() }
        val vm = ChatViewModel(fake, "c1", VisibleConversation)
        advanceUntilIdle()
        val importer = object : name.apk.lanmsg.files.Importer {
            override suspend fun import(uris: List<android.net.Uri>) = if (uris.size == 1) listOf("/o/a.jpg") else emptyList()
        }
        vm.importAndSend(importer, listOf(android.net.Uri.EMPTY)); advanceUntilIdle()
        assertEquals(Request.SendFiles("c1", listOf("/o/a.jpg")), fake.requests.last())
        val n = fake.requests.size
        vm.importAndSend(importer, listOf(android.net.Uri.EMPTY, android.net.Uri.EMPTY)); advanceUntilIdle()
        assertEquals(n, fake.requests.size)
    }
}
