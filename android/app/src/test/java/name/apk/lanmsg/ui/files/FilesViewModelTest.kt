@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.files

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
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.FileKindFilter
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class FilesViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun fake() = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                is Request.QueryFiles -> Response.Files(emptyList())
                Request.ListMyDevices -> Response.Devices(emptyList())
                Request.ListPeers -> Response.Peers(emptyList())
                else -> Response.Ok
            }
        }
    }

    private fun FakeCoreApi.queries() = requests.filterIsInstance<Request.QueryFiles>()

    @Test fun default_filter_queries_all_with_limit_200() = runTest {
        val fake = fake()
        val vm = FilesViewModel(fake, null)
        advanceUntilIdle()
        val f = fake.queries().single().filter
        assertEquals(FileKindFilter.ALL, f.kind)
        assertNull(f.conv)
        assertNull(f.sourceDevice)
        assertNull(f.state)
        assertNull(f.nameQuery)
        assertEquals(200u, f.limit)
        assertFalse(vm.state.value.loading)
    }

    @Test fun initial_conv_is_passed_to_filter() = runTest {
        val fake = fake()
        FilesViewModel(fake, "c9")
        advanceUntilIdle()
        assertEquals("c9", fake.queries().single().filter.conv)
    }

    @Test fun set_kind_requeries_with_new_kind() = runTest {
        val fake = fake()
        val vm = FilesViewModel(fake, null)
        advanceUntilIdle()
        vm.setKind(FileKindFilter.IMAGE)
        advanceUntilIdle()
        assertEquals(FileKindFilter.IMAGE, fake.queries().last().filter.kind)
    }

    @Test fun set_state_and_source_device_requery() = runTest {
        val fake = fake()
        val vm = FilesViewModel(fake, null)
        advanceUntilIdle()
        vm.setState(BlobState.LOCAL)
        vm.setSourceDevice("d2")
        advanceUntilIdle()
        val f = fake.queries().last().filter
        assertEquals(BlobState.LOCAL, f.state)
        assertEquals("d2", f.sourceDevice)
    }

    @Test fun query_is_debounced() = runTest {
        val fake = fake()
        val vm = FilesViewModel(fake, null)
        advanceUntilIdle()
        val before = fake.queries().size
        vm.setQuery("a")
        advanceTimeBy(100)
        vm.setQuery("ab")
        advanceTimeBy(350)
        runCurrent()
        val added = fake.queries().drop(before)
        assertEquals(listOf("ab"), added.map { it.filter.nameQuery })
    }

    @Test fun blank_query_becomes_null() = runTest {
        val fake = fake()
        val vm = FilesViewModel(fake, null)
        advanceUntilIdle()
        vm.setQuery("x")
        advanceUntilIdle()
        vm.setQuery("   ")
        advanceUntilIdle()
        assertNull(fake.queries().last().filter.nameQuery)
    }

    @Test fun burst_of_events_requeries_once() = runTest {
        val fake = fake()
        FilesViewModel(fake, null)
        advanceUntilIdle()
        val before = fake.queries().size
        repeat(3) {
            fake.emit(Event.MessageAdded(Fixtures.fileMsg("m$it", "h$it")))
            advanceTimeBy(100)
        }
        advanceTimeBy(600)
        runCurrent()
        assertEquals(before + 1, fake.queries().size)
    }

    @Test fun message_updated_requeries_with_same_debounce() = runTest {
        val fake = fake()
        FilesViewModel(fake, null)
        advanceUntilIdle()
        val before = fake.queries().size
        fake.emit(Event.MessageUpdated(Fixtures.fileMsg("m1", "h1")))
        fake.emit(Event.MessageUpdated(Fixtures.fileMsg("m1", "h1")))
        advanceTimeBy(300)
        assertEquals(before, fake.queries().size)
        advanceTimeBy(300)
        runCurrent()
        assertEquals(before + 1, fake.queries().size)
    }

    @Test fun lagged_requeries_immediately() = runTest {
        val fake = fake()
        FilesViewModel(fake, null)
        advanceUntilIdle()
        val before = fake.queries().size
        fake.emitLagged()
        runCurrent()
        assertEquals(before + 1, fake.queries().size)
    }

    @Test fun not_implemented_sets_core_outdated() = runTest {
        val fake = fake().apply {
            handler = { throw ApiException(ErrorCode.INTERNAL, "未实现：QueryFiles") }
        }
        val vm = FilesViewModel(fake, null)
        advanceUntilIdle()
        assertTrue(vm.state.value.coreOutdated)
        assertNull(vm.state.value.error)
        assertFalse(vm.state.value.loading)
    }

    @Test fun other_failure_sets_error() = runTest {
        val fake = fake().apply {
            handler = { req -> if (req is Request.QueryFiles) throw ApiException(ErrorCode.IO, "boom") else Response.Ok }
        }
        val vm = FilesViewModel(fake, null)
        advanceUntilIdle()
        assertEquals("boom", vm.state.value.error)
        assertFalse(vm.state.value.coreOutdated)
    }

    @Test fun download_sends_request_and_remembers_transfer_for_cancel() = runTest {
        val fake = fake().apply {
            val inner = handler
            handler = { req -> if (req is Request.DownloadBlob) Response.TransferId("t1") else inner(req) }
        }
        val vm = FilesViewModel(fake, null)
        advanceUntilIdle()
        vm.download("h1")
        advanceUntilIdle()
        assertTrue(fake.requests.contains(Request.DownloadBlob("h1")))
        vm.cancel("h1")
        advanceUntilIdle()
        assertTrue(fake.requests.contains(Request.CancelTransfer("t1")))
    }

    @Test fun source_devices_list_own_devices_first_then_other_peers() = runTest {
        val fake = fake().apply {
            val inner = handler
            handler = { req ->
                when (req) {
                    Request.ListMyDevices -> Response.Devices(
                        listOf(
                            uniffi.lanmsg_api.DeviceView("d2", "D2", "手机", "phone", false, null, false),
                            uniffi.lanmsg_api.DeviceView("d1", "D1", "本机名", "phone", true, null, true),
                        ),
                    )
                    Request.ListPeers -> Response.Peers(listOf(Fixtures.peer("d2"), Fixtures.peer("d3", name = "老王")))
                    else -> inner(req)
                }
            }
        }
        val vm = FilesViewModel(fake, null)
        advanceUntilIdle()
        val devices = vm.state.value.devices
        assertEquals(listOf("d1", "d2", "d3"), devices.map { it.deviceId })
        assertTrue(devices[0].self)
        assertNotNull(devices.firstOrNull { it.deviceId == "d3" && it.subLabel == "老王" })
    }
}
