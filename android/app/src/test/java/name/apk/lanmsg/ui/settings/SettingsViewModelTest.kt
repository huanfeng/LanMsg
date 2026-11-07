@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.settings

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.SettingsPrefs
import name.apk.lanmsg.files.OutboxStore
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.BackgroundMode
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SettingsDto

@RunWith(AndroidJUnit4::class)
class SettingsViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val initial = SettingsDto(
        deviceName = "phone", downloadDir = "/d", autoImageMax = 10uL, autoFileMax = 0uL,
        autoFileWifiOnly = true, localsendEnabled = false, unicastTargets = emptyList(),
        backgroundMode = BackgroundMode.REALTIME, localsendListening = null,
    )
    private var failSet = false
    private val prefs = MemoryPrefs()
    private val outbox = MemoryOutbox(2048)
    private val modes = mutableListOf<BackgroundMode>()

    /** core 当前持有的值：GetSettings 返回它，SetSettings 成功时更新它。 */
    private var coreValue = initial.copy()

    private fun fake() = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.GetSettings -> Response.Settings(coreValue.copy())
                is Request.SetSettings -> if (failSet) throw ApiException(ErrorCode.IO, "disk full") else { coreValue = req.settings.copy(); Response.Ok }
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected $req")
            }
        }
    }

    private var now = 10L * 24 * 3600 * 1000

    private fun vm(fake: FakeCoreApi) = SettingsViewModel(fake, prefs, outbox, { modes += it }, main.dispatcher) { now }

    @Test fun loads_settings_local_prefs_and_outbox_size() = runTest {
        prefs.notifications.value = false
        prefs.wifi.value = setOf("home")
        val vm = vm(fake())
        advanceUntilIdle()
        val s = vm.state.value
        assertEquals("phone", s.settings?.deviceName)
        assertEquals(false, s.notifications)
        assertEquals(setOf("home"), s.trustedWifi)
        assertEquals(2048L, s.outboxBytes)
    }

    @Test fun update_is_optimistic_and_sends_whole_settings() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        vm.update { it.copy(autoImageMax = 5uL) }
        assertEquals(5uL, vm.state.value.settings?.autoImageMax) // 未推进调度器：请求尚未发出
        advanceUntilIdle()
        val sent = fake.requests.filterIsInstance<Request.SetSettings>().single().settings
        assertEquals(5uL, sent.autoImageMax)
        assertEquals("phone", sent.deviceName)
        assertNull(vm.state.value.error)
    }

    @Test fun update_is_optimistic_and_rolls_back_on_failure() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        failSet = true
        vm.update { it.copy(localsendEnabled = true) }
        advanceUntilIdle()
        assertEquals(false, vm.state.value.settings?.localsendEnabled)
        assertNotNull(vm.state.value.error)
    }

    @Test fun background_mode_change_notifies_holder() = runTest {
        val vm = vm(fake())
        advanceUntilIdle()
        vm.update { it.copy(backgroundMode = BackgroundMode.POWER_SAVING) }
        advanceUntilIdle()
        assertEquals(listOf(BackgroundMode.POWER_SAVING), modes)
    }

    @Test fun background_mode_not_notified_when_unchanged_or_failed() = runTest {
        val vm = vm(fake())
        advanceUntilIdle()
        vm.update { it.copy(autoFileMax = 5uL) }
        advanceUntilIdle()
        failSet = true
        vm.update { it.copy(backgroundMode = BackgroundMode.OFF) }
        advanceUntilIdle()
        assertTrue(modes.isEmpty())
    }

    @Test fun local_switches_write_prefs() = runTest {
        val vm = vm(fake())
        advanceUntilIdle()
        vm.setNotifications(false)
        vm.setSaveToGallery(false)
        advanceUntilIdle()
        assertEquals(false, prefs.notifications.value)
        assertEquals(false, prefs.gallery.value)
        assertEquals(false, vm.state.value.notifications)
        assertEquals(false, vm.state.value.saveToGallery)
    }

    @Test fun trusted_wifi_add_remove_and_ignore_blank() = runTest {
        val vm = vm(fake())
        advanceUntilIdle()
        vm.addTrustedWifi("home")
        vm.addTrustedWifi("  ")
        vm.addTrustedWifi("office")
        advanceUntilIdle()
        assertEquals(setOf("home", "office"), vm.state.value.trustedWifi)
        vm.removeTrustedWifi("home")
        advanceUntilIdle()
        assertEquals(setOf("office"), vm.state.value.trustedWifi)
    }

    @Test fun clear_outbox_uses_seven_day_cutoff_and_refreshes_size() = runTest {
        val vm = vm(fake())
        advanceUntilIdle()
        vm.clearOutbox()
        advanceUntilIdle()
        assertEquals(now - 7L * 24 * 3600 * 1000, outbox.cutoff)
        assertEquals(0L, vm.state.value.outboxBytes)
    }

    @Test fun consecutive_failed_updates_end_with_core_value() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        failSet = true
        vm.update { it.copy(autoImageMax = 5uL) }
        vm.update { it.copy(autoFileMax = 7uL) }
        advanceUntilIdle()
        assertEquals(initial.autoImageMax, vm.state.value.settings?.autoImageMax)
        assertEquals(initial.autoFileMax, vm.state.value.settings?.autoFileMax)
        assertNotNull(vm.state.value.error)
    }

    @Test fun failed_update_shows_core_value_not_pre_write_snapshot() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        failSet = true
        vm.update { it.copy(autoImageMax = 5uL) }
        coreValue = coreValue.copy(deviceName = "changed-elsewhere") // 失败期间 core 已被别处改过
        advanceUntilIdle()
        assertEquals("changed-elsewhere", vm.state.value.settings?.deviceName)
    }

    @Test fun failed_update_and_failed_reload_fall_back_to_last_confirmed() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        val ok = fake.handler
        fake.handler = { req -> if (req is Request.SetSettings || req == Request.GetSettings) throw ApiException(ErrorCode.IO, "down") else ok(req) }
        vm.update { it.copy(autoImageMax = 5uL) }
        advanceUntilIdle()
        assertEquals(initial.autoImageMax, vm.state.value.settings?.autoImageMax)
    }

    @Test fun failed_first_update_is_not_carried_by_second() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        val ok = fake.handler
        var n = 0
        fake.handler = { req -> if (req is Request.SetSettings && n++ == 0) throw ApiException(ErrorCode.IO, "once") else ok(req) }
        vm.update { it.copy(autoImageMax = 5uL) }
        vm.update { it.copy(autoFileMax = 7uL) }
        advanceUntilIdle()
        assertEquals(initial.autoImageMax, coreValue.autoImageMax)
        assertEquals(7uL, coreValue.autoFileMax)
        assertEquals(initial.autoImageMax, vm.state.value.settings?.autoImageMax)
    }

    @Test fun lagged_during_write_does_not_override_optimistic_value() = runTest {
        val fake = fake()
        val vm = vm(fake)
        advanceUntilIdle()
        vm.update { it.copy(autoImageMax = 5uL) }
        fake.emitLagged() // 写入尚未发出：乐观值在途
        runCurrent()
        assertEquals(5uL, vm.state.value.settings?.autoImageMax)
        advanceUntilIdle()
        assertEquals(5uL, vm.state.value.settings?.autoImageMax)
        assertEquals(2, fake.requests.count { it == Request.GetSettings }) // 初次 + 写完后补拉一次
    }

    @Test fun parse_ssid_strips_quotes_and_unknown() {
        assertEquals("home", WifiSsid.parse("\"home\""))
        assertNull(WifiSsid.parse("<unknown ssid>"))
        assertNull(WifiSsid.parse(null))
        assertNull(WifiSsid.parse("\"\""))
    }
}

private class MemoryPrefs : SettingsPrefs {
    val notifications = MutableStateFlow(true)
    val gallery = MutableStateFlow(true)
    val wifi = MutableStateFlow<Set<String>>(emptySet())
    override val notificationsEnabled: Flow<Boolean> = notifications
    override val saveMediaToGallery: Flow<Boolean> = gallery
    override val trustedWifi: Flow<Set<String>> = wifi

    override suspend fun setNotificationsEnabled(v: Boolean) { notifications.value = v }
    override suspend fun setSaveMediaToGallery(v: Boolean) { gallery.value = v }
    override suspend fun setTrustedWifi(ssids: Set<String>) { wifi.value = ssids }
}

private class MemoryOutbox(private var bytes: Long) : OutboxStore {
    var cutoff: Long? = null
    override fun outboxSizeBytes(): Long = bytes
    override fun clearOlderThan(cutoffMillis: Long): Long { cutoff = cutoffMillis; val freed = bytes; bytes = 0; return freed }
}
