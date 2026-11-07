@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.search

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
import uniffi.lanmsg_api.ErrorCode
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

    @Test fun not_implemented_sets_core_outdated() = runTest {
        val fake = fake().apply {
            val inner = handler
            handler = { req -> if (req is Request.SearchMessages) throw ApiException(ErrorCode.INTERNAL, "未实现：SearchMessages") else inner(req) }
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
        assertEquals("boom", vm.state.value.error)
    }
}
