@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.share

import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.files.Importer
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
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class SharePickViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun fake() = FakeCoreApi().apply {
        handler = { r ->
            when (r) {
                is Request.ListConversations -> Response.Conversations(listOf(Fixtures.conv("c1", "")))
                is Request.SendFiles -> Response.MsgId("m1")
                is Request.SendText -> Response.MsgId("m2")
                else -> Response.Ok
            }
        }
    }

    private class FakeImporter(val out: List<String>) : Importer {
        override suspend fun import(uris: List<Uri>): List<String> = out
    }

    @Test fun pick_with_uris_imports_then_sends_files() = runTest {
        val core = fake()
        val vm = SharePickViewModel(core, FakeImporter(listOf("/x/a.jpg")))
        vm.setPending(listOf(Uri.parse("content://x/a")), null)
        advanceUntilIdle()
        assertEquals(1, vm.state.value.items.size)
        vm.pick("c1"); advanceUntilIdle()
        assertEquals(Request.SendFiles("c1", listOf("/x/a.jpg")), core.requests.last())
        assertEquals("c1", vm.state.value.sentTo)
    }

    @Test fun pick_with_text_sends_text() = runTest {
        val core = fake()
        val vm = SharePickViewModel(core, FakeImporter(emptyList()))
        vm.setPending(emptyList(), "hello")
        advanceUntilIdle()
        vm.pick("c1"); advanceUntilIdle()
        assertEquals(Request.SendText("c1", "hello"), core.requests.last())
        assertEquals("c1", vm.state.value.sentTo)
    }

    @Test fun all_imports_failed_sets_error_and_sends_nothing() = runTest {
        val core = fake()
        val vm = SharePickViewModel(core, FakeImporter(emptyList()))
        vm.setPending(listOf(Uri.parse("content://x/a")), null)
        advanceUntilIdle()
        vm.pick("c1"); advanceUntilIdle()
        assertNull(vm.state.value.sentTo)
        assertNotNull(vm.state.value.error)
        assertEquals(false, core.requests.any { it is Request.SendFiles })
    }

    @Test fun send_failure_sets_error() = runTest {
        val core = fake().apply {
            val base = handler
            handler = { r -> if (r is Request.SendText) throw ApiException(ErrorCode.PEER_OFFLINE, "x") else base(r) }
        }
        val vm = SharePickViewModel(core, FakeImporter(emptyList()))
        vm.setPending(emptyList(), "hi"); advanceUntilIdle()
        vm.pick("c1"); advanceUntilIdle()
        assertNull(vm.state.value.sentTo)
        assertNotNull(vm.state.value.error)
        assertEquals(false, vm.state.value.busy)
    }

    @Test fun reset_clears_pending_and_result() = runTest {
        val vm = SharePickViewModel(fake(), FakeImporter(listOf("/a")))
        vm.setPending(emptyList(), "hi"); advanceUntilIdle()
        vm.pick("c1"); advanceUntilIdle()
        vm.reset()
        assertEquals(null, vm.state.value.sentTo)
        assertEquals(null, vm.state.value.pendingText)
    }
}
