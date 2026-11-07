@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.contacts

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.R
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class ContactsViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun fake(peers: () -> List<uniffi.lanmsg_api.PeerView> = { emptyList() }, requests: () -> Int = { 0 }) =
        FakeCoreApi().apply {
            handler = { req ->
                when (req) {
                    Request.ListContacts -> Response.Contacts(listOf(Fixtures.contact("u9")))
                    Request.ListPeers -> Response.Peers(peers())
                    Request.ListRequests -> Response.Requests(List(requests()) { Fixtures.request("r$it") })
                    is Request.RequestContact -> Response.Ok
                    else -> throw ApiException(ErrorCode.INTERNAL, "unexpected $req")
                }
            }
        }

    @Test fun nearby_excludes_contacts_and_mine() = runTest {
        val fake = fake(peers = {
            listOf(
                Fixtures.peer("d1", "u1", isContact = true),
                Fixtures.peer("d2", "u2", isMine = true),
                Fixtures.peer("d3", "u3"),
            )
        })
        val vm = ContactsViewModel(fake)
        advanceUntilIdle()
        assertEquals(listOf("u3"), vm.state.value.nearby.map { it.userId })
        assertEquals(listOf("u9"), vm.state.value.contacts.map { it.userId })
    }

    @Test fun nearby_shows_one_row_per_user() = runTest {
        val fake = fake(peers = { listOf(Fixtures.peer("d3", "u3"), Fixtures.peer("d4", "u3")) })
        val vm = ContactsViewModel(fake)
        advanceUntilIdle()
        assertEquals(listOf("d3"), vm.state.value.nearby.map { it.deviceId })
    }

    @Test fun add_sends_request_contact_and_toasts() = runTest {
        val fake = fake()
        val vm = ContactsViewModel(fake)
        advanceUntilIdle()
        vm.add("u2")
        advanceUntilIdle()
        assertEquals(Request.RequestContact("u2"), fake.requests.last())
        assertEquals(R.string.toast_request_sent, vm.state.value.toast)
        vm.toastShown()
        assertNull(vm.state.value.toast)
    }

    @Test fun add_failure_sets_error_without_toast() = runTest {
        val fake = fake()
        val vm = ContactsViewModel(fake)
        advanceUntilIdle()
        fake.handler = { throw ApiException(ErrorCode.PEER_OFFLINE, "offline") }
        vm.add("u2")
        advanceUntilIdle()
        assertEquals(R.string.err_peer_offline, vm.state.value.error)
        assertNull(vm.state.value.toast)
    }

    @Test fun incoming_request_refreshes_count() = runTest {
        var count = 0
        val fake = fake(requests = { count })
        val vm = ContactsViewModel(fake)
        advanceUntilIdle()
        assertEquals(0, vm.state.value.requestCount)
        count = 2
        fake.emit(Event.IncomingRequest(Fixtures.request("r9")))
        advanceUntilIdle()
        assertEquals(2, vm.state.value.requestCount)
    }

    @Test fun lagged_reloads_everything() = runTest {
        var count = 0
        val fake = fake(requests = { count })
        val vm = ContactsViewModel(fake)
        advanceUntilIdle()
        count = 3
        fake.emitLagged()
        advanceUntilIdle()
        assertEquals(3, vm.state.value.requestCount)
    }
}
