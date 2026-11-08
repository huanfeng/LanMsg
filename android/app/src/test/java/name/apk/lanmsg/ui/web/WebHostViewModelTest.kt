@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.web

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class WebHostViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private var hosting = false
    private var startError: ErrorCode? = null
    private var convs = listOf(Fixtures.conv("g1", kind = ConvKind.GUEST), Fixtures.conv("d1", kind = ConvKind.DM))

    private fun fake() = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.WebStart -> startError?.let { throw ApiException(it, "x") } ?: Response.WebSession(Fixtures.webSessionInfo())
                Request.ListConversations -> Response.Conversations(convs)
                else -> Response.Ok
            }
        }
    }

    private fun vm(fake: FakeCoreApi) = WebHostViewModel(fake, { hosting = it }, { 5000L }, CoroutineScope(main.dispatcher))

    @Test fun start_sets_hosting_and_info() = runTest {
        val vm = vm(fake())
        vm.start()
        assertTrue(hosting)
        advanceUntilIdle()
        assertNotNull(vm.state.value.info)
        assertEquals(listOf("g1"), vm.state.value.guests.map { it.conv })
    }

    @Test fun start_failure_clears_hosting_and_reports_code() = runTest {
        startError = ErrorCode.IO
        val vm = vm(fake())
        vm.start()
        advanceUntilIdle()
        assertFalse(hosting)
        assertEquals(ErrorCode.IO, vm.state.value.error)
        assertTrue(vm.state.value.ended)
    }

    @Test fun web_request_then_allow() = runTest {
        val f = fake()
        val vm = vm(f)
        advanceUntilIdle()
        f.emit(Event.WebRequest("r1", "UA", "192.168.1.5"))
        // advanceUntilIdle 会把 60 秒自动拒绝的延时也跑完，这里只跑当前时刻的任务
        runCurrent()
        assertEquals("r1", vm.state.value.pending?.request)
        vm.respond(true)
        runCurrent()
        assertEquals(Request.WebRespond("r1", true), f.requests.last())
        assertNull(vm.state.value.pending)
    }

    @Test fun pending_request_denied_after_60_seconds() = runTest {
        val f = fake()
        val vm = vm(f)
        advanceUntilIdle()
        f.emit(Event.WebRequest("r1", "UA", "192.168.1.5"))
        runCurrent()
        advanceTimeBy(59_000)
        assertNotNull(vm.state.value.pending)
        advanceTimeBy(2_000)
        advanceUntilIdle()
        assertNull(vm.state.value.pending)
        assertEquals(Request.WebRespond("r1", false), f.requests.last())
    }

    @Test fun stop_clears_hosting() = runTest {
        val f = fake()
        val vm = vm(f)
        vm.start()
        advanceUntilIdle()
        vm.stop()
        advanceUntilIdle()
        assertFalse(hosting)
        assertTrue(f.requests.contains(Request.WebStop))
        assertNull(vm.state.value.info)
        assertTrue(vm.state.value.ended)
    }

    @Test fun session_changed_to_none_ends_page_and_hosting() = runTest {
        val f = fake()
        val vm = vm(f)
        vm.start()
        advanceUntilIdle()
        f.emit(Event.WebSessionChanged(null))
        advanceUntilIdle()
        assertFalse(hosting)
        assertTrue(vm.state.value.ended)
    }

    @Test fun guest_conversation_update_upserts_guests() = runTest {
        val f = fake()
        val vm = vm(f)
        vm.start()
        advanceUntilIdle()
        f.emit(Event.ConversationUpdated(Fixtures.conv("g2", kind = ConvKind.GUEST, last = 2000)))
        f.emit(Event.ConversationUpdated(Fixtures.conv("d2", kind = ConvKind.DM)))
        advanceUntilIdle()
        assertEquals(setOf("g1", "g2"), vm.state.value.guests.map { it.conv }.toSet())
    }

    @Test fun select_url_ignores_out_of_range() = runTest {
        val vm = vm(fake())
        vm.start()
        advanceUntilIdle()
        vm.selectUrl(1)
        assertEquals(1, vm.state.value.urlIndex)
        vm.selectUrl(5)
        assertEquals(1, vm.state.value.urlIndex)
    }

    @Test fun leaving_while_start_in_flight_clears_hosting_and_stops_core() = runTest {
        val gate = CompletableDeferred<Response>()
        val f = fake().apply {
            val base = handler
            handler = { req -> if (req == Request.WebStart) gate.await() else base(req) }
        }
        val store = ViewModelStore()
        val vm = ViewModelProvider(store, viewModelFactory { initializer { vm(f) } })[WebHostViewModel::class.java]
        vm.start()
        runCurrent()
        assertTrue(hosting)
        store.clear()
        runCurrent()
        assertFalse(hosting)
        assertTrue(f.requests.contains(Request.WebStop))
    }

    @Test fun leaving_with_active_session_stops_core() = runTest {
        val f = fake()
        val store = ViewModelStore()
        val vm = ViewModelProvider(store, viewModelFactory { initializer { vm(f) } })[WebHostViewModel::class.java]
        vm.start()
        advanceUntilIdle()
        store.clear()
        advanceUntilIdle()
        assertFalse(hosting)
        assertTrue(f.requests.contains(Request.WebStop))
    }

    @Test fun exit_without_guests_stops_directly() = runTest {
        val f = fake()
        val vm = vm(f)
        vm.start()
        advanceUntilIdle()
        vm.requestExit()
        advanceUntilIdle()
        assertFalse(vm.state.value.confirmExit)
        assertTrue(vm.state.value.ended)
        assertTrue(f.requests.contains(Request.WebStop))
    }

    @Test fun exit_with_active_guest_asks_confirmation_first() = runTest {
        val f = fake()
        val vm = vm(f)
        vm.start()
        advanceUntilIdle()
        f.emit(Event.ConversationUpdated(Fixtures.conv("g1", kind = ConvKind.GUEST)))
        advanceUntilIdle()
        vm.requestExit()
        advanceUntilIdle()
        assertTrue(vm.state.value.confirmExit)
        assertFalse(vm.state.value.ended)
        assertTrue(f.requests.none { it == Request.WebStop })
        vm.dismissExit()
        assertFalse(vm.state.value.confirmExit)
        vm.requestExit()
        vm.stop()
        advanceUntilIdle()
        assertTrue(vm.state.value.ended)
        assertTrue(f.requests.contains(Request.WebStop))
    }

    @Test fun requests_are_queued_in_arrival_order() = runTest {
        val f = fake()
        val vm = vm(f)
        advanceUntilIdle()
        f.emit(Event.WebRequest("r1", "UA", "192.168.1.5"))
        f.emit(Event.WebRequest("r2", "UA", "192.168.1.6"))
        f.emit(Event.WebRequest("r1", "UA", "192.168.1.5"))
        runCurrent()
        assertEquals("r1", vm.state.value.pending?.request)
        vm.respond(false)
        runCurrent()
        assertEquals("r2", vm.state.value.pending?.request)
        assertEquals(Request.WebRespond("r1", false), f.requests.last())
        vm.respond(true)
        runCurrent()
        assertNull(vm.state.value.pending)
        assertEquals(Request.WebRespond("r2", true), f.requests.last())
    }
}
