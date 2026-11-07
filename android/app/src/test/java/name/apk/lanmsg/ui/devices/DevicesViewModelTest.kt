@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.devices

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CompletableDeferred
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
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.DeviceView
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.PairStateDto
import uniffi.lanmsg_api.PathClassDto
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class DevicesViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val self = DeviceView("d1", "D1", "本机", "phone", true, null, true)
    private val other = DeviceView("d2", "D2", "电脑", "desktop", false, PathClassDto.LAN, false)

    private fun fake(list: () -> List<DeviceView>) = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.ListMyDevices -> Response.Devices(list())
                is Request.RemoveDevice -> Response.Ok
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected $req")
            }
        }
    }

    @Test fun loads_devices() = runTest {
        val vm = DevicesViewModel(fake { listOf(self, other) })
        advanceUntilIdle()
        assertEquals(listOf("d1", "d2"), vm.state.value.devices.map { it.deviceId })
    }

    @Test fun confirm_remove_sends_remove_device_and_reloads() = runTest {
        var devices = listOf(self, other)
        val f = fake { devices }
        val vm = DevicesViewModel(f)
        advanceUntilIdle()
        vm.askRemove(other)
        assertEquals(other, vm.state.value.confirmRemove)
        devices = listOf(self)
        vm.confirmRemove()
        advanceUntilIdle()
        assertEquals(1, f.requests.count { it == Request.RemoveDevice("d2") })
        assertEquals(listOf("d1"), vm.state.value.devices.map { it.deviceId })
        assertNull(vm.state.value.confirmRemove)
    }

    @Test fun dismiss_clears_confirmation_without_request() = runTest {
        val f = fake { listOf(self, other) }
        val vm = DevicesViewModel(f)
        advanceUntilIdle()
        vm.askRemove(other)
        vm.dismiss()
        advanceUntilIdle()
        assertNull(vm.state.value.confirmRemove)
        assertEquals(0, f.requests.count { it is Request.RemoveDevice })
    }

    @Test fun cannot_ask_to_remove_self() = runTest {
        val vm = DevicesViewModel(fake { listOf(self) })
        advanceUntilIdle()
        vm.askRemove(self)
        assertNull(vm.state.value.confirmRemove)
    }

    @Test fun devices_changed_reloads() = runTest {
        var devices = listOf(self)
        val f = fake { devices }
        val vm = DevicesViewModel(f)
        advanceUntilIdle()
        devices = listOf(self, other)
        f.emit(Event.DevicesChanged)
        advanceUntilIdle()
        assertEquals(2, vm.state.value.devices.size)
    }

    @Test fun pair_done_and_peer_online_reload() = runTest {
        var devices = listOf(self)
        val f = fake { devices }
        val vm = DevicesViewModel(f)
        advanceUntilIdle()
        devices = listOf(self, other)
        f.emit(Event.PairStatus("s1", PairStateDto.Verifying))
        advanceUntilIdle()
        assertEquals(1, vm.state.value.devices.size) // 配对未完成不重拉
        f.emit(Event.PairStatus("s1", PairStateDto.Done))
        advanceUntilIdle()
        assertEquals(2, vm.state.value.devices.size)
        devices = listOf(self, other, other.copy(deviceId = "d3"))
        f.emit(Event.PeerOnline(Fixtures.peer("d3")))
        advanceUntilIdle()
        assertEquals(3, vm.state.value.devices.size)
    }

    @Test fun remove_failure_sets_error() = runTest {
        val f = fake { listOf(self, other) }
        val vm = DevicesViewModel(f)
        advanceUntilIdle()
        val ok = f.handler
        f.handler = { req -> if (req is Request.RemoveDevice) throw ApiException(ErrorCode.IO, "disk") else ok(req) }
        vm.askRemove(other)
        vm.confirmRemove()
        advanceUntilIdle()
        assertEquals("disk", vm.state.value.error)
        assertNull(vm.state.value.confirmRemove)
    }

    @Test fun self_is_shown_online_even_though_core_reports_offline() {
        assertEquals(true, isDeviceOnline(self.copy(online = false)))
        assertEquals(false, isDeviceOnline(other))
        assertEquals(true, isDeviceOnline(other.copy(online = true)))
    }

    @Test fun burst_of_events_is_debounced_into_one_load() = runTest {
        val f = fake { listOf(self) }
        val vm = DevicesViewModel(f)
        advanceUntilIdle()
        val before = f.requests.count { it == Request.ListMyDevices }
        f.emit(Event.DevicesChanged); runCurrent()
        advanceTimeBy(100)
        f.emit(Event.PeerOnline(Fixtures.peer("d3"))); runCurrent()
        advanceTimeBy(100)
        f.emit(Event.PeerOffline("d3")); runCurrent()
        assertEquals(before, f.requests.count { it == Request.ListMyDevices })
        advanceUntilIdle()
        assertEquals(before + 1, f.requests.count { it == Request.ListMyDevices })
        assertEquals(1, vm.state.value.devices.size)
    }

    @Test fun older_in_flight_load_is_cancelled_by_newer_one() = runTest {
        val gate = CompletableDeferred<Unit>()
        var calls = 0
        val f = FakeCoreApi().apply {
            handler = { req ->
                if (req == Request.ListMyDevices) {
                    val n = ++calls
                    if (n == 1) gate.await()
                    Response.Devices(if (n == 1) listOf(self) else listOf(self, other))
                } else {
                    Response.Ok
                }
            }
        }
        val vm = DevicesViewModel(f)
        advanceTimeBy(300) // 第一次加载已发出并挂起
        f.emit(Event.DevicesChanged)
        advanceUntilIdle()
        gate.complete(Unit)
        advanceUntilIdle()
        assertEquals(2, vm.state.value.devices.size)
    }
}
