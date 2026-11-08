@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.station

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
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.StationPutItem
import uniffi.lanmsg_api.StoreMode

@RunWith(AndroidJUnit4::class)
class StationViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private var stations = listOf(Fixtures.stationView("s1", hostOnline = false))
    private var items = listOf(Fixtures.stationTextItem("i1", "hi"), Fixtures.stationFileItem("i2"))
    private var putError: ErrorCode? = null

    private val importer = object : Importer {
        override suspend fun import(uris: List<Uri>): List<String> = uris.map { "/outbox/${it.lastPathSegment}" }
    }

    private fun fake() = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.StationList -> Response.Stations(stations)
                is Request.StationItems -> Response.StationItems(items)
                is Request.StationPut -> putError?.let { throw ApiException(it, "x") } ?: Response.Ok
                is Request.DownloadBlob -> Response.TransferId("t1")
                else -> Response.Ok
            }
        }
    }

    private fun vm(fake: FakeCoreApi, clock: () -> Long = { 1000L }) = StationViewModel(fake, "s1", importer, clock)

    @Test fun loads_items_and_host_state() = runTest {
        val vm = vm(fake())
        advanceUntilIdle()
        val s = vm.state.value
        assertEquals(2, s.items.size)
        assertFalse(s.hostOnline)
        assertEquals("桌面中转站", s.name)
    }

    @Test fun station_updated_event_triggers_refresh() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        assertEquals(1, fake.requests.count { it is Request.StationItems })
        fake.emit(Event.StationUpdated("s2"))
        advanceUntilIdle()
        assertEquals(1, fake.requests.count { it is Request.StationItems })
        fake.emit(Event.StationUpdated("s1"))
        advanceUntilIdle()
        assertEquals(2, fake.requests.count { it is Request.StationItems })
        assertTrue(vm.state.value.items.isNotEmpty())
    }

    @Test fun lagged_triggers_refresh() = runTest {
        val fake = fake()
        vm(fake)
        advanceUntilIdle()
        fake.emitLagged()
        advanceUntilIdle()
        assertEquals(2, fake.requests.count { it is Request.StationItems })
    }

    @Test fun put_text_uses_copy_mode() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        vm.putText("hi")
        advanceUntilIdle()
        assertEquals(
            Request.StationPut("s1", StationPutItem.Text("hi"), StoreMode.COPY, null),
            fake.requests.filterIsInstance<Request.StationPut>().last(),
        )
    }

    @Test fun blank_text_is_not_sent() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        vm.putText("   ")
        advanceUntilIdle()
        assertTrue(fake.requests.none { it is Request.StationPut })
    }

    @Test fun put_uris_imports_then_puts_paths_with_copy_mode() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        vm.putUris(listOf(Uri.parse("content://x/a.txt")))
        advanceUntilIdle()
        assertEquals(
            Request.StationPut("s1", StationPutItem.Paths(listOf("/outbox/a.txt")), StoreMode.COPY, null),
            fake.requests.filterIsInstance<Request.StationPut>().last(),
        )
        assertFalse(vm.state.value.busy)
    }

    @Test fun expiry_applies_to_put() = runTest {
        val fake = fake()
        val vm = vm(fake) { 1000L }
        advanceUntilIdle()
        vm.setExpiry(StationViewModel.Expiry.DAY1)
        vm.putText("x")
        advanceUntilIdle()
        val put = fake.requests.filterIsInstance<Request.StationPut>().last()
        assertEquals(1000L + 86_400_000L, put.expiresAt)
        assertEquals(StationViewModel.Expiry.DAY1, vm.state.value.expiry)
    }

    @Test fun set_pinned_sends_request() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        vm.setPinned("i1", true)
        advanceUntilIdle()
        assertEquals(Request.StationSetPinned("s1", "i1", true), fake.requests.filterIsInstance<Request.StationSetPinned>().last())
    }

    @Test fun remove_and_download_send_requests() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        vm.remove("i1")
        vm.download("h-i2")
        advanceUntilIdle()
        assertEquals(Request.StationRemove("s1", "i1"), fake.requests.filterIsInstance<Request.StationRemove>().last())
        assertEquals(Request.DownloadBlob("h-i2"), fake.requests.filterIsInstance<Request.DownloadBlob>().last())
    }

    @Test fun failure_is_stored_as_error_code_and_cleared() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        putError = ErrorCode.PEER_OFFLINE
        vm.putText("x")
        advanceUntilIdle()
        assertEquals(ErrorCode.PEER_OFFLINE, vm.state.value.error)
        assertFalse(vm.state.value.busy)
        vm.clearError()
        assertNull(vm.state.value.error)
    }

    @Test fun busy_stays_true_while_request_pending_without_error() = runTest {
        val fake = fake()
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        val ok = fake.handler
        fake.handler = { req -> if (req is Request.StationPut) { gate.await(); Response.Ok } else ok(req) }
        val vm = vm(fake)
        advanceUntilIdle()
        vm.putText("x")
        advanceUntilIdle()
        assertTrue(vm.state.value.busy)
        assertNull(vm.state.value.error)
        gate.complete(Unit)
        advanceUntilIdle()
        assertFalse(vm.state.value.busy)
    }

    @Test fun refresh_failure_keeps_old_items_and_records_error() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        val ok = fake.handler
        fake.handler = { req -> if (req is Request.StationItems) throw ApiException(ErrorCode.IO, "x") else ok(req) }
        vm.refresh()
        advanceUntilIdle()
        assertEquals(2, vm.state.value.items.size)
        assertEquals(ErrorCode.IO, vm.state.value.error)
    }
}
