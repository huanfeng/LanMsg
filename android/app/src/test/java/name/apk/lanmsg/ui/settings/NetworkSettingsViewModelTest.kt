@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.settings

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.BackgroundMode
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.InterfaceView
import uniffi.lanmsg_api.ManualPeerView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SettingsDto

@RunWith(AndroidJUnit4::class)
class NetworkSettingsViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private var settings = SettingsDto("n", "/d", 0uL, 0uL, false, true, emptyList(), BackgroundMode.REALTIME, true)
    private var addError: ErrorCode? = null
    private var scanError: ErrorCode? = null
    private var peers = listOf(ManualPeerView("192.168.1.9:53320", null))

    private fun fake() = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.GetSettings -> Response.Settings(settings)
                Request.ListInterfaces -> Response.Interfaces(listOf(InterfaceView("wlan0", "192.168.1.5", 24u, null)))
                Request.ListManualPeers -> Response.ManualPeers(peers)
                is Request.AddManualPeer -> addError?.let { throw ApiException(it, "x") } ?: Response.Ok
                is Request.ScanSubnet -> scanError?.let { throw ApiException(it, "x") } ?: Response.Ok
                is Request.SetSettings -> { settings = req.settings; Response.Ok }
                else -> Response.Ok
            }
        }
    }

    @Test fun load_fills_interfaces_peers_and_settings() = runTest {
        val vm = NetworkSettingsViewModel(fake())
        advanceUntilIdle()
        val s = vm.state.value
        assertEquals(1, s.interfaces.size)
        assertEquals(1, s.manualPeers.size)
        assertNotNull(s.settings)
    }

    @Test fun scan_finished_sets_result() = runTest {
        val f = fake()
        val vm = NetworkSettingsViewModel(f)
        advanceUntilIdle()
        vm.scan("192.168.1.0/24")
        assertEquals("192.168.1.0/24", vm.state.value.scanning)
        advanceUntilIdle()
        f.emit(Event.ScanFinished("192.168.1.0/24", 254u, 2u))
        advanceUntilIdle()
        assertNull(vm.state.value.scanning)
        assertEquals(NetworkSettingsViewModel.ScanResult(254u, 2u), vm.state.value.scanResult)
    }

    @Test fun scan_ignored_while_scanning() = runTest {
        val f = fake()
        val vm = NetworkSettingsViewModel(f)
        advanceUntilIdle()
        vm.scan("192.168.1.0/24")
        vm.scan("10.0.0.0/24")
        advanceUntilIdle()
        assertEquals(1, f.requests.count { it is Request.ScanSubnet })
    }

    @Test fun scan_error_clears_scanning_and_reports_code() = runTest {
        scanError = ErrorCode.INVALID_ARGUMENT
        val vm = NetworkSettingsViewModel(fake())
        advanceUntilIdle()
        vm.scan("10.0.0.0/8")
        advanceUntilIdle()
        assertNull(vm.state.value.scanning)
        assertEquals(ErrorCode.INVALID_ARGUMENT, vm.state.value.error)
    }

    @Test fun manual_peer_error_code_shown() = runTest {
        addError = ErrorCode.INVALID_ARGUMENT
        val vm = NetworkSettingsViewModel(fake())
        advanceUntilIdle()
        vm.addManualPeer("bad")
        advanceUntilIdle()
        assertEquals(ErrorCode.INVALID_ARGUMENT, vm.state.value.manualError)
        assertFalse(vm.state.value.manualOk)
        assertFalse(vm.state.value.manualBusy)
    }

    @Test fun manual_peer_busy_while_connecting_then_ok() = runTest {
        val vm = NetworkSettingsViewModel(fake())
        advanceUntilIdle()
        vm.addManualPeer(" 192.168.1.9 ")
        assertTrue(vm.state.value.manualBusy)
        assertNull(vm.state.value.manualError)
        advanceUntilIdle()
        assertTrue(vm.state.value.manualOk)
        assertFalse(vm.state.value.manualBusy)
    }

    @Test fun unicast_targets_written_when_valid() = runTest {
        val f = fake()
        val vm = NetworkSettingsViewModel(f)
        advanceUntilIdle()
        vm.setUnicastTargets(listOf("192.168.2.10", "", " 10.0.0.0/24 "))
        advanceUntilIdle()
        assertEquals(listOf("192.168.2.10", "10.0.0.0/24"), settings.unicastTargets)
    }

    @Test fun unicast_targets_invalid_not_written() = runTest {
        val f = fake()
        val vm = NetworkSettingsViewModel(f)
        advanceUntilIdle()
        vm.setUnicastTargets(listOf("host.example"))
        advanceUntilIdle()
        assertTrue(vm.state.value.unicastInvalid)
        assertTrue(f.requests.none { it is Request.SetSettings })
    }

    @Test fun localsend_toggle_preserves_other_settings() = runTest {
        val vm = NetworkSettingsViewModel(fake())
        advanceUntilIdle()
        vm.setLocalSendEnabled(false)
        advanceUntilIdle()
        assertFalse(settings.localsendEnabled)
        assertEquals("n", settings.deviceName)
    }

    @Test fun unicast_target_rules_match_core() {
        val ok = listOf("192.168.1.2", "192.168.1.2:53317", "10.0.0.0/24", "10.0.0.1/32", "fe80::1", "[fe80::1]:53317", "::1")
        val bad = listOf("host", "host:53317", "192.168.1.2:0", "192.168.1.2:70000", "10.0.0.0/23", "10.0.0.0/33", "fd00::/64", "256.1.1.1", "1.2.3", "[::1", "[::1]x")
        ok.forEach { assertTrue(it, NetworkSettingsViewModel.isValidUnicastTarget(it)) }
        bad.forEach { assertFalse(it, NetworkSettingsViewModel.isValidUnicastTarget(it)) }
    }
}

class ScanCidrTest {
    private fun iface(addr: String, prefix: Int) = InterfaceView("x", addr, prefix.toUByte(), null)

    @Test fun small_prefix_uses_slash_24() {
        assertEquals("10.1.2.0/24", scanCidrFor(iface("10.1.2.3", 8)))
    }

    @Test fun prefix_24_and_22_use_interface_network() {
        assertEquals("192.168.1.0/24", scanCidrFor(iface("192.168.1.10", 24)))
        assertEquals("10.1.4.0/22", scanCidrFor(iface("10.1.5.5", 22)))
    }

    @Test fun ipv4_detection() {
        assertTrue(isIpv4Address("192.168.1.2"))
        assertFalse(isIpv4Address("fe80::1"))
    }
}

class VisibleInterfacesTest {
    private fun i(name: String, addr: String) = InterfaceView(name, addr, 24u, null)

    @Test fun ipv4_first_deduped_and_link_local_hidden() {
        val v = visibleInterfaces(
            listOf(
                i("wlan0", "fe80::1"), i("wlan0", "fec0::5"), i("wlan0", "2409::1"), i("wlan0", "fd00::2"),
                i("wlan0", "10.0.2.16"), i("wlan0", "10.0.2.16"), i("eth0", "192.168.1.2"),
            ),
        )
        assertEquals(listOf("10.0.2.16", "192.168.1.2"), v.ipv4.map { it.addr })
        assertEquals(2, v.otherIpv6)
    }
}
