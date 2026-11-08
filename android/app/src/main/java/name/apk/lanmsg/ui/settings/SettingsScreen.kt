package name.apk.lanmsg.ui.settings

import android.Manifest
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings
import android.util.Log
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import kotlinx.coroutines.launch
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.service.BatteryGuidance
import name.apk.lanmsg.ui.common.ConfirmDialog
import name.apk.lanmsg.ui.common.Format
import name.apk.lanmsg.ui.common.OptionRow
import name.apk.lanmsg.ui.common.SectionHeader
import name.apk.lanmsg.ui.common.SwitchRow
import name.apk.lanmsg.ui.common.TextInputDialog
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.BackgroundMode

@Composable
fun SettingsScreen(onBack: () -> Unit, onOpenNetwork: () -> Unit) {
    val vm: SettingsViewModel = viewModel(
        factory = viewModelFactory {
            initializer { SettingsViewModel(ServiceLocator.coreHolder.api, ServiceLocator.localPrefs, ServiceLocator.importer) }
        },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    // 从网络设置页返回时，LocalSend 等设置可能已被改写，重新读取避免整体回写时覆盖
    LifecycleResumeEffect(Unit) {
        vm.reload()
        onPauseOrDispose {}
    }
    SettingsContent(state, vm, onBack, onOpenNetwork)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsContent(state: SettingsViewModel.UiState, vm: SettingsViewModel, onBack: () -> Unit, onOpenNetwork: () -> Unit) {
    val c = LanMsgColors.current
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(it) }
    LaunchedEffect(errorText, state.errorSeq) { errorText?.let { snackbar.showSnackbar(it) } }
    val clearedText = state.clearedBytes?.let { stringResource(R.string.settings_clear_outbox_done, Format.size(it)) }
    LaunchedEffect(clearedText) {
        if (clearedText != null) {
            snackbar.showSnackbar(clearedText)
            vm.clearedShown()
        }
    }

    val needLocation = stringResource(R.string.settings_trusted_wifi_need_location)
    val notConnected = stringResource(R.string.settings_trusted_wifi_not_connected)
    fun addCurrentWifi() {
        when (val r = WifiSsid.current(ctx)) {
            is WifiSsid.Reading.Connected -> vm.addTrustedWifi(r.ssid)
            WifiSsid.Reading.NotConnected -> scope.launch { snackbar.showSnackbar(notConnected) }
            WifiSsid.Reading.Unavailable -> scope.launch { snackbar.showSnackbar(needLocation) }
        }
    }
    val locationPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) addCurrentWifi() else scope.launch { snackbar.showSnackbar(needLocation) }
    }

    val batteryExempt by rememberBatteryExempt()
    var editingName by remember { mutableStateOf(false) }
    var confirmClear by remember { mutableStateOf(false) }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.me_settings)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.back))
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = c.surface,
                    titleContentColor = c.ink,
                    navigationIconContentColor = c.ink,
                ),
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { inner ->
        val settings = state.settings
        if (settings == null) {
            Box(Modifier.padding(inner).fillMaxSize(), contentAlignment = Alignment.Center) {
                if (state.error == null) CircularProgressIndicator(color = LanMsgControls.progress)
            }
            return@Scaffold
        }
        Column(Modifier.padding(inner).fillMaxSize().verticalScroll(rememberScrollState())) {
            Group {
                ValueRow(stringResource(R.string.settings_device_name), settings.deviceName) { editingName = true }
                Divider()
                OptionRow(
                    title = stringResource(R.string.settings_auto_image),
                    current = autoLabel(settings.autoImageMax),
                    options = SettingsViewModel.AUTO_LIMITS,
                    label = { autoLabel(it) },
                    onSelect = { v -> vm.update { it.copy(autoImageMax = v) } },
                )
                Divider()
                OptionRow(
                    title = stringResource(R.string.settings_auto_file),
                    current = autoLabel(settings.autoFileMax),
                    options = SettingsViewModel.AUTO_LIMITS,
                    label = { autoLabel(it) },
                    onSelect = { v -> vm.update { it.copy(autoFileMax = v) } },
                )
                Divider()
                SwitchRow(stringResource(R.string.settings_wifi_only), settings.autoFileWifiOnly, { v -> vm.update { it.copy(autoFileWifiOnly = v) } })
                Divider()
                ValueRow(stringResource(R.string.settings_network), "", onOpenNetwork)
            }

            SectionHeader(stringResource(R.string.settings_bg_mode))
            Group {
                BackgroundOption(R.string.bg_realtime, BackgroundMode.REALTIME, settings.backgroundMode) { m -> vm.update { it.copy(backgroundMode = m) } }
                Divider()
                BackgroundOption(R.string.bg_power_saving, BackgroundMode.POWER_SAVING, settings.backgroundMode) { m -> vm.update { it.copy(backgroundMode = m) } }
                Divider()
                BackgroundOption(R.string.bg_off, BackgroundMode.OFF, settings.backgroundMode) { m -> vm.update { it.copy(backgroundMode = m) } }
                if (BatteryGuidance.showLegacyEntry(settings.backgroundMode, batteryExempt)) {
                    Divider()
                    ActionRow(stringResource(R.string.settings_battery), stringResource(R.string.settings_battery_desc)) { openBatterySettings(ctx) }
                }
            }

            BatteryGuidanceCard(settings.backgroundMode, batteryExempt)

            SectionHeader(stringResource(R.string.settings_trusted_wifi))
            Group {
                for (ssid in state.trustedWifi.sorted()) {
                    Row(
                        Modifier.fillMaxWidth().padding(start = 16.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(ssid, color = c.ink, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                        IconButton(onClick = { vm.removeTrustedWifi(ssid) }) {
                            Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.settings_trusted_wifi_remove, ssid), tint = c.inkMuted)
                        }
                    }
                    Divider()
                }
                TextButton(
                    onClick = {
                        if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED) {
                            addCurrentWifi()
                        } else {
                            locationPermission.launch(Manifest.permission.ACCESS_FINE_LOCATION)
                        }
                    },
                    colors = LanMsgButtons.text(),
                    modifier = Modifier.padding(horizontal = 8.dp),
                ) { Text(stringResource(R.string.settings_trusted_wifi_add)) }
            }

            Box(Modifier.padding(top = 20.dp))
            Group {
                SwitchRow(stringResource(R.string.settings_notifications), state.notifications, vm::setNotifications)
                Divider()
                SwitchRow(stringResource(R.string.settings_save_gallery), state.saveToGallery, vm::setSaveToGallery)
                Divider()
                ActionRow(
                    stringResource(R.string.settings_clear_outbox),
                    stringResource(R.string.settings_clear_outbox_desc, Format.size(state.outboxBytes)),
                ) { confirmClear = true }
            }
            Box(Modifier.padding(bottom = 24.dp))
        }

        if (editingName) {
            TextInputDialog(
                title = stringResource(R.string.settings_device_name),
                initial = settings.deviceName,
                onConfirm = { name -> editingName = false; vm.update { it.copy(deviceName = name) } },
                onDismiss = { editingName = false },
            )
        }
        if (confirmClear) {
            ConfirmDialog(
                title = stringResource(R.string.settings_clear_outbox),
                text = stringResource(R.string.settings_clear_outbox_confirm),
                confirmLabel = stringResource(R.string.settings_clear_outbox_action),
                onConfirm = { confirmClear = false; vm.clearOutbox() },
                onDismiss = { confirmClear = false },
            )
        }
    }
}

@Composable
private fun Group(content: @Composable () -> Unit) {
    val c = LanMsgColors.current
    Column(Modifier.fillMaxWidth().background(c.surface)) {
        HorizontalDivider(color = c.line)
        content()
        HorizontalDivider(color = c.line)
    }
}

@Composable
private fun Divider() = HorizontalDivider(color = LanMsgColors.current.line)

@Composable
private fun ValueRow(title: String, value: String, onClick: () -> Unit) {
    val c = LanMsgColors.current
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(title, color = c.ink, style = MaterialTheme.typography.bodyLarge)
        Text(value, color = c.inkMuted, modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun ActionRow(title: String, description: String, onClick: () -> Unit) {
    val c = LanMsgColors.current
    Column(Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(title, color = c.signal, style = MaterialTheme.typography.bodyLarge)
        Text(description, color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
    }
}

@Composable
private fun BackgroundOption(label: Int, value: BackgroundMode, current: BackgroundMode, onSelect: (BackgroundMode) -> Unit) {
    val c = LanMsgColors.current
    Row(
        Modifier.fillMaxWidth().clickable { onSelect(value) }.padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RadioButton(
            selected = current == value,
            onClick = null,
            colors = RadioButtonDefaults.colors(selectedColor = c.signal, unselectedColor = c.inkMuted),
        )
        Text(stringResource(label), color = c.ink, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(start = 16.dp))
    }
}

/** 0 = 关闭，ULong.MAX_VALUE = 不限，其余按大小格式显示。 */
@Composable
private fun autoLabel(v: ULong): String = when (v) {
    0uL -> stringResource(R.string.auto_off)
    ULong.MAX_VALUE -> stringResource(R.string.size_unlimited)
    else -> Format.size(v.toLong())
}

private fun openBatterySettings(ctx: Context) {
    try {
        ctx.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    } catch (e: ActivityNotFoundException) {
        Log.w("SettingsScreen", "Battery optimization settings not available", e)
    }
}
