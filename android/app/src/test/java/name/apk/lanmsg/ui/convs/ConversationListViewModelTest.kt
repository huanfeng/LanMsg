@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.convs

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.LocalPrefs
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class ConversationListViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun prefs() = LocalPrefs(ApplicationProvider.getApplicationContext())

    private fun fakeWith(list: () -> List<ConversationSummary>) =
        FakeCoreApi().apply { handler = { Response.Conversations(list()) } }

    @Test fun sorts_by_last_message_desc_nulls_last() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("a", last = 100), Fixtures.conv("b", last = null), Fixtures.conv("c", last = 300)) }
        val vm = ConversationListViewModel(fake, prefs())
        advanceUntilIdle()
        assertEquals(listOf("c", "a", "b"), vm.state.value.rows.map { it.summary.conv })
        assertEquals(false, vm.state.value.loading)
    }

    @Test fun conversation_updated_upserts_and_resorts() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("a", last = 100), Fixtures.conv("b", last = 200), Fixtures.conv("c", last = 50)) }
        val vm = ConversationListViewModel(fake, prefs())
        advanceUntilIdle()
        fake.emit(Event.ConversationUpdated(Fixtures.conv("c", last = 999)))
        advanceUntilIdle()
        assertEquals(listOf("c", "b", "a"), vm.state.value.rows.map { it.summary.conv })
    }

    @Test fun conversation_updated_adds_unknown_conversation() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("a", last = 100)) }
        val vm = ConversationListViewModel(fake, prefs())
        advanceUntilIdle()
        fake.emit(Event.ConversationUpdated(Fixtures.conv("n", last = 500)))
        advanceUntilIdle()
        assertEquals(listOf("n", "a"), vm.state.value.rows.map { it.summary.conv })
    }

    @Test fun lagged_reloads_list() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("a")) }
        ConversationListViewModel(fake, prefs())
        advanceUntilIdle()
        fake.emitLagged()
        advanceUntilIdle()
        assertEquals(2, fake.requests.count { it == Request.ListConversations })
    }

    @Test fun toggle_mute_marks_row_muted() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("m1")) }
        val vm = ConversationListViewModel(fake, prefs())
        advanceUntilIdle()
        vm.toggleMute("m1")
        // DataStore 在自己的线程上写入，轮询等待结果回到测试调度器
        var muted = false
        repeat(100) {
            advanceUntilIdle()
            muted = vm.state.value.rows.firstOrNull()?.muted == true
            if (!muted) Thread.sleep(50)
        }
        assertTrue(muted)
    }
}
