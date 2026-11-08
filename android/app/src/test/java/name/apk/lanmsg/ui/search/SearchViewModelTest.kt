@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.search

import name.apk.lanmsg.ui.common.errorTextRes
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
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
class SearchViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun fake() = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.ListConversations -> Response.Conversations(listOf(Fixtures.conv("c1", title = "张三")))
                is Request.SearchMessages -> Response.Messages(listOf(Fixtures.textMsg("m1", conv = "c1", text = req.query)))
                else -> Response.Ok
            }
        }
    }

    private fun FakeCoreApi.searches() = requests.filterIsInstance<Request.SearchMessages>()

    @Test fun query_is_debounced_and_sends_search() = runTest {
        val fake = fake()
        val vm = SearchViewModel(fake)
        vm.setQuery("a")
        advanceTimeBy(100)
        vm.setQuery("ab")
        advanceTimeBy(350)
        runCurrent()
        assertEquals(listOf(Request.SearchMessages("ab", null, 50u)), fake.searches())
        assertEquals(1, vm.state.value.rows.size)
    }

    @Test fun blank_query_clears_without_request() = runTest {
        val fake = fake()
        val vm = SearchViewModel(fake)
        vm.setQuery("a")
        advanceUntilIdle()
        assertEquals(1, vm.state.value.rows.size)
        vm.setQuery("  ")
        advanceUntilIdle()
        assertTrue(vm.state.value.rows.isEmpty())
        assertEquals(1, fake.searches().size)
    }

    @Test fun rows_carry_conversation_title() = runTest {
        val vm = SearchViewModel(fake())
        vm.setQuery("hi")
        advanceUntilIdle()
        assertEquals("张三", vm.state.value.rows.single().convTitle)
    }

    @Test fun conversation_updated_refreshes_cached_title() = runTest {
        val fake = fake()
        val vm = SearchViewModel(fake)
        vm.setQuery("a")
        advanceUntilIdle()
        fake.emit(Event.ConversationUpdated(Fixtures.conv("c1", title = "张三丰")))
        vm.setQuery("ab")
        advanceUntilIdle()
        assertEquals("张三丰", vm.state.value.rows.single().convTitle)
        assertEquals(1, fake.requests.count { it == Request.ListConversations })
    }

    @Test fun unknown_conversation_triggers_refetch() = runTest {
        val fake = fake().apply {
            val inner = handler
            var calls = 0
            handler = { req ->
                if (req == Request.ListConversations) {
                    calls++
                    Response.Conversations(if (calls == 1) emptyList() else listOf(Fixtures.conv("c1", title = "张三")))
                } else inner(req)
            }
        }
        val vm = SearchViewModel(fake)
        vm.setQuery("a")
        advanceUntilIdle()
        assertEquals("张三", vm.state.value.rows.single().convTitle)
    }

    @Test fun self_devices_conversation_is_flagged() = runTest {
        val vm = SearchViewModel(fake())
        vm.setQuery("a")
        advanceUntilIdle()
        assertEquals(ConvKind.SELF_DEVICES, vm.state.value.rows.single().kind)
    }

    @Test fun not_implemented_sets_core_outdated() = runTest {
        val fake = fake().apply {
            val inner = handler
            handler = { req -> if (req is Request.SearchMessages) throw ApiException(ErrorCode.UNIMPLEMENTED, "unimplemented: SearchMessages") else inner(req) }
        }
        val vm = SearchViewModel(fake)
        vm.setQuery("a")
        advanceUntilIdle()
        assertTrue(vm.state.value.coreOutdated)
        assertNull(vm.state.value.error)
    }

    @Test fun other_failure_sets_error() = runTest {
        val fake = fake().apply {
            val inner = handler
            handler = { req -> if (req is Request.SearchMessages) throw ApiException(ErrorCode.IO, "boom") else inner(req) }
        }
        val vm = SearchViewModel(fake)
        vm.setQuery("a")
        advanceUntilIdle()
        assertEquals(errorTextRes(ErrorCode.IO), vm.state.value.error)
    }

    @Test fun permanently_missing_conversation_is_refetched_only_once() = runTest {
        val fake = fake().apply {
            val inner = handler
            handler = { req ->
                if (req is Request.SearchMessages) {
                    Response.Messages(listOf(Fixtures.textMsg("m1", conv = "gone", text = req.query)))
                } else {
                    inner(req)
                }
            }
        }
        val vm = SearchViewModel(fake)
        vm.setQuery("a"); advanceUntilIdle()
        vm.setQuery("ab"); advanceUntilIdle()
        vm.setQuery("abc"); advanceUntilIdle()
        // 首次拉取 1 次 + 发现缺失后重拉 1 次；之后同一缺失会话不再重拉
        assertEquals(2, fake.requests.count { it == Request.ListConversations })
        assertEquals("", vm.state.value.rows.single().convTitle)
    }
}
