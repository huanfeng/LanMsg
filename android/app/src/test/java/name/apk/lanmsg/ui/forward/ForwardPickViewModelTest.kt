@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.forward

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
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class ForwardPickViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun fake() = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.ListConversations -> Response.Conversations(
                    listOf(Fixtures.conv("c1", last = null), Fixtures.conv("c2", last = 300), Fixtures.conv("c3", last = 100)),
                )
                is Request.ForwardFiles -> Response.MsgId("m9")
                else -> Response.Ok
            }
        }
    }

    @Test fun items_sorted_by_last_message_desc_nulls_last() = runTest {
        val vm = ForwardPickViewModel(fake(), listOf("h1"))
        advanceUntilIdle()
        assertEquals(listOf("c2", "c3", "c1"), vm.state.value.items.map { it.conv })
    }

    @Test fun pick_sends_forward_files_and_sets_done() = runTest {
        val fake = fake()
        val vm = ForwardPickViewModel(fake, listOf("h1", "h2"))
        advanceUntilIdle()
        vm.pick("c2")
        advanceUntilIdle()
        assertEquals(Request.ForwardFiles("c2", listOf("h1", "h2")), fake.requests.last())
        assertEquals("c2", vm.state.value.done)
    }

    @Test fun failure_sets_error_and_keeps_done_null() = runTest {
        val fake = fake().apply {
            val inner = handler
            handler = { req -> if (req is Request.ForwardFiles) throw ApiException(ErrorCode.PEER_OFFLINE, "x") else inner(req) }
        }
        val vm = ForwardPickViewModel(fake, listOf("h1"))
        advanceUntilIdle()
        vm.pick("c2")
        advanceUntilIdle()
        assertNotNull(vm.state.value.error)
        assertNull(vm.state.value.done)
    }

    @Test fun not_implemented_sets_core_outdated() = runTest {
        val fake = fake().apply {
            val inner = handler
            handler = { req -> if (req is Request.ForwardFiles) throw ApiException(ErrorCode.INTERNAL, "未实现：ForwardFiles") else inner(req) }
        }
        val vm = ForwardPickViewModel(fake, listOf("h1"))
        advanceUntilIdle()
        vm.pick("c2")
        advanceUntilIdle()
        assertTrue(vm.state.value.coreOutdated)
        assertNull(vm.state.value.error)
        assertNull(vm.state.value.done)
    }
}
