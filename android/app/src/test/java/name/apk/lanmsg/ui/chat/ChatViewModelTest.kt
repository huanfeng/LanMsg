@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.chat

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
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.MessageState
import uniffi.lanmsg_api.MessageView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

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
        assertEquals(ChatViewModel.ERROR_PEER_OFFLINE, vm.state.value.error)
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
        fake.emit(Event.TransferFinished("t1", "h", true, null)); advanceUntilIdle()
        assertTrue(vm.state.value.progress.isEmpty())
        assertEquals(before + 1, fake.requests.count { it is Request.ListMessages })
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
}
