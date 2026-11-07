@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.contacts

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.RequestAction
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class RequestsViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun fake(fail: Boolean = false) = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.ListRequests -> Response.Requests(listOf(Fixtures.request("r1"), Fixtures.request("r2")))
                is Request.RespondRequest ->
                    if (fail) throw ApiException(ErrorCode.NOT_FOUND, "gone") else Response.Ok
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected $req")
            }
        }
    }

    @Test fun respond_removes_item_on_success() = runTest {
        val fake = fake()
        val vm = RequestsViewModel(fake)
        advanceUntilIdle()
        vm.respond("r1", RequestAction.ADD_CONTACT)
        advanceUntilIdle()
        assertEquals(Request.RespondRequest("r1", RequestAction.ADD_CONTACT), fake.requests.last())
        assertEquals(listOf("r2"), vm.state.value.items.map { it.request })
    }

    @Test fun respond_failure_keeps_item_and_sets_error() = runTest {
        val vm = RequestsViewModel(fake(fail = true))
        advanceUntilIdle()
        vm.respond("r1", RequestAction.REJECT)
        advanceUntilIdle()
        assertEquals(listOf("r1", "r2"), vm.state.value.items.map { it.request })
        assertNotNull(vm.state.value.error)
    }

    @Test fun incoming_request_reloads_list() = runTest {
        val fake = fake()
        val vm = RequestsViewModel(fake)
        advanceUntilIdle()
        vm.respond("r1", RequestAction.REJECT)
        advanceUntilIdle()
        fake.emit(Event.IncomingRequest(Fixtures.request("r3")))
        advanceUntilIdle()
        assertEquals(listOf("r1", "r2"), vm.state.value.items.map { it.request })
        assertNull(vm.state.value.error)
    }
}
