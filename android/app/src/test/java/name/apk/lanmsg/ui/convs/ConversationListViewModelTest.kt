@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.convs

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.MutedStore
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

    private fun fakeWith(list: () -> List<ConversationSummary>) =
        FakeCoreApi().apply { handler = { Response.Conversations(list()) } }

    @Test fun sorts_by_last_message_desc_nulls_last() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("a", last = 100), Fixtures.conv("b", last = null), Fixtures.conv("c", last = 300)) }
        val vm = ConversationListViewModel(fake, MemoryMutedStore())
        advanceUntilIdle()
        assertEquals(listOf("c", "a", "b"), vm.state.value.rows.map { it.summary.conv })
        assertEquals(false, vm.state.value.loading)
    }

    @Test fun conversation_updated_upserts_and_resorts() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("a", last = 100), Fixtures.conv("b", last = 200), Fixtures.conv("c", last = 50)) }
        val vm = ConversationListViewModel(fake, MemoryMutedStore())
        advanceUntilIdle()
        fake.emit(Event.ConversationUpdated(Fixtures.conv("c", last = 999)))
        advanceUntilIdle()
        assertEquals(listOf("c", "b", "a"), vm.state.value.rows.map { it.summary.conv })
    }

    @Test fun conversation_updated_adds_unknown_conversation() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("a", last = 100)) }
        val vm = ConversationListViewModel(fake, MemoryMutedStore())
        advanceUntilIdle()
        fake.emit(Event.ConversationUpdated(Fixtures.conv("n", last = 500)))
        advanceUntilIdle()
        assertEquals(listOf("n", "a"), vm.state.value.rows.map { it.summary.conv })
    }

    @Test fun lagged_reloads_list() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("a")) }
        ConversationListViewModel(fake, MemoryMutedStore())
        advanceUntilIdle()
        fake.emitLagged()
        advanceUntilIdle()
        assertEquals(2, fake.requests.count { it == Request.ListConversations })
    }

    @Test fun toggle_mute_marks_row_muted() = runTest {
        val fake = fakeWith { listOf(Fixtures.conv("m1")) }
        val vm = ConversationListViewModel(fake, MemoryMutedStore())
        advanceUntilIdle()
        vm.toggleMute("m1")
        advanceUntilIdle()
        assertTrue(vm.state.value.rows.single().muted)
        vm.toggleMute("m1")
        advanceUntilIdle()
        assertEquals(false, vm.state.value.rows.single().muted)
    }

    @Test fun refresh_in_flight_does_not_overwrite_newer_conversation_update() = runTest {
        val gate = CompletableDeferred<Unit>()
        var calls = 0
        val fake = FakeCoreApi().apply {
            handler = {
                if (calls++ == 0) Response.Conversations(listOf(Fixtures.conv("a", last = 100)))
                else {
                    gate.await()
                    Response.Conversations(listOf(Fixtures.conv("a", last = 100)))
                }
            }
        }
        val vm = ConversationListViewModel(fake, MemoryMutedStore())
        advanceUntilIdle()
        fake.emitLagged()
        advanceUntilIdle()
        fake.emit(Event.ConversationUpdated(Fixtures.conv("a", last = 999)))
        advanceUntilIdle()
        gate.complete(Unit)
        advanceUntilIdle()
        assertEquals(999L, vm.state.value.rows.single().summary.lastHlcMillis)
    }
}

private class MemoryMutedStore : MutedStore {
    private val set = MutableStateFlow<Set<String>>(emptySet())
    override val mutedConversations: Flow<Set<String>> = set

    override suspend fun setMuted(conv: String, muted: Boolean) {
        set.value = if (muted) set.value + conv else set.value - conv
    }
}
