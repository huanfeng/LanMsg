package name.apk.lanmsg.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.SectionHeader
import name.apk.lanmsg.ui.common.SwitchRow
import name.apk.lanmsg.ui.common.errorTextRes
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.InterfaceView

@Composable
fun NetworkSettingsScreen(onBack: () -> Unit) {
    val vm: NetworkSettingsViewModel = viewModel(
        factory = viewModelFactory { initializer { NetworkSettingsViewModel(ServiceLocator.coreHolder.api) } },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    NetworkSettingsContent(state, vm, onBack)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NetworkSettingsContent(state: NetworkSettingsViewModel.UiState, vm: NetworkSettingsViewModel, onBack: () -> Unit) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(errorTextRes(it)) }
    LaunchedEffect(errorText) {
        if (errorText != null) {
            snackbar.showSnackbar(errorText)
            vm.clearError()
        }
    }
    var manualAddr by remember { mutableStateOf("") }
    var scanCidr by remember { mutableStateOf("") }
    val settings = state.settings

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.net_title)) },
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
        Column(Modifier.padding(inner).fillMaxSize().verticalScroll(rememberScrollState())) {
            if (settings != null) {
                Group {
                    SwitchRow(
                        stringResource(R.string.settings_localsend),
                        settings.localsendEnabled,
                        vm::setLocalSendEnabled,
                    )
                    if (settings.localsendEnabled && settings.localsendListening == false) {
                        Text(
                            stringResource(R.string.localsend_port_busy, LOCALSEND_DEFAULT_PORT),
                            color = c.danger,
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 12.dp),
                        )
                    }
                }
            }

            SectionHeader(stringResource(R.string.net_manual_add))
            Group {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedTextField(
                            value = manualAddr,
                            onValueChange = { manualAddr = it },
                            singleLine = true,
                            placeholder = { Text(stringResource(R.string.net_manual_hint), color = c.inkSubtle) },
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                            colors = LanMsgControls.textField(),
                            modifier = Modifier.weight(1f).testTag("manual_addr"),
                        )
                        Button(
                            onClick = { vm.addManualPeer(manualAddr) },
                            enabled = manualAddr.isNotBlank() && !state.manualBusy,
                        ) { Text(stringResource(R.string.net_connect)) }
                    }
                    Text(stringResource(R.string.net_manual_help), color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
                    when {
                        state.manualBusy -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp, color = LanMsgControls.progress)
                            Text(stringResource(R.string.net_connecting), color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
                        }
                        state.manualError != null ->
                            Text(stringResource(errorTextRes(state.manualError)), color = c.danger, style = MaterialTheme.typography.bodySmall)
                        state.manualOk ->
                            Text(stringResource(R.string.net_connected), color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
                    }
                }
                for (p in state.manualPeers) {
                    HorizontalDivider(color = c.line)
                    Row(Modifier.fillMaxWidth().padding(start = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(p.addr, color = c.ink, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                        IconButton(onClick = { vm.removeManualPeer(p.addr) }) {
                            Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.net_manual_remove, p.addr), tint = c.inkMuted)
                        }
                    }
                }
            }

            SectionHeader(stringResource(R.string.net_unicast_targets))
            Group {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(
                        value = state.unicastText,
                        onValueChange = vm::editUnicast,
                        minLines = 2,
                        maxLines = 6,
                        isError = state.unicastInvalid,
                        colors = LanMsgControls.textField(),
                        supportingText = {
                            Text(
                                stringResource(if (state.unicastInvalid) R.string.net_unicast_invalid else R.string.net_unicast_hint),
                                color = if (state.unicastInvalid) c.danger else c.inkMuted,
                            )
                        },
                        modifier = Modifier.fillMaxWidth().testTag("unicast_targets"),
                    )
                    TextButton(
                        onClick = vm::saveUnicast,
                        colors = LanMsgButtons.text(),
                        enabled = settings != null,
                    ) { Text(stringResource(R.string.net_unicast_save)) }
                }
            }

            SectionHeader(stringResource(R.string.net_scan))
            Group {
                val shown = visibleInterfaces(state.interfaces)
                for (iface in shown.ipv4) {
                    InterfaceRow(iface, scanEnabled = state.scanning == null) { vm.scan(scanCidrFor(iface)) }
                    HorizontalDivider(color = c.line)
                }
                if (shown.otherIpv6 > 0) {
                    Text(
                        stringResource(R.string.net_other_ipv6, shown.otherIpv6),
                        color = c.inkMuted,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(16.dp),
                    )
                    HorizontalDivider(color = c.line)
                }
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedTextField(
                            value = scanCidr,
                            onValueChange = { scanCidr = it },
                            singleLine = true,
                            placeholder = { Text(stringResource(R.string.net_scan_hint), color = c.inkSubtle) },
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                            colors = LanMsgControls.textField(),
                            modifier = Modifier.weight(1f).testTag("scan_cidr"),
                        )
                        Button(
                            onClick = { vm.scan(scanCidr) },
                            enabled = scanCidr.isNotBlank() && state.scanning == null,
                        ) { Text(stringResource(R.string.net_scan_action)) }
                    }
                    val scanning = state.scanning
                    val result = state.scanResult
                    if (scanning != null) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp, color = LanMsgControls.progress)
                            Text(stringResource(R.string.net_scanning, scanning), color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
                        }
                    } else if (result != null) {
                        Text(
                            stringResource(R.string.net_scan_result, result.probed.toInt(), result.found.toInt()),
                            color = c.inkMuted,
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun InterfaceRow(iface: InterfaceView, scanEnabled: Boolean, onScan: () -> Unit) {
    val c = LanMsgColors.current
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(iface.name, color = c.ink, style = MaterialTheme.typography.bodyLarge)
            Text("${iface.addr}/${iface.prefix}", color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
            tunnelLabelRes(iface.tunnelKind)?.let {
                Text(stringResource(it), color = c.signal, style = MaterialTheme.typography.labelSmall)
            }
        }
        // 只扫描 IPv4 网卡
        if (isIpv4Address(iface.addr)) {
            TextButton(onClick = onScan, enabled = scanEnabled, colors = LanMsgButtons.text()) {
                Text(stringResource(R.string.net_scan_this))
            }
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

/** 隧道网卡角标文案；非隧道返回 null。 */
private fun tunnelLabelRes(kind: String?): Int? = when (kind) {
    null -> null
    "tailscale" -> R.string.net_tunnel_tailscale
    "zerotier" -> R.string.net_tunnel_zerotier
    "wireguard" -> R.string.net_tunnel_wireguard
    else -> R.string.net_tunnel_generic
}

internal fun isIpv4Address(addr: String): Boolean = !addr.contains(':') && addr.count { it == '.' } == 3

/** 扫描网段：前缀不小于 22 用网卡自身网段，否则取本机地址所在 /24（core 只接受前缀不小于 22）。 */
internal fun scanCidrFor(iface: InterfaceView): String {
    val octets = iface.addr.split('.').map { it.toIntOrNull() ?: 0 }
    if (octets.size != 4) return "${iface.addr}/${iface.prefix}"
    val prefix = if (iface.prefix.toInt() >= 22) iface.prefix.toInt() else 24
    val addr = octets.fold(0L) { acc, o -> (acc shl 8) or (o.toLong() and 0xFF) }
    val mask = (0xFFFFFFFFL shl (32 - prefix)) and 0xFFFFFFFFL
    val net = addr and mask
    return "${(net shr 24) and 0xFF}.${(net shr 16) and 0xFF}.${(net shr 8) and 0xFF}.${net and 0xFF}/$prefix"
}

/** 网卡列表的展示结果：可扫描的 IPv4 网卡，以及被折叠的其他 IPv6 地址个数。 */
internal data class VisibleInterfaces(val ipv4: List<InterfaceView>, val otherIpv6: Int)

private const val LOCALSEND_DEFAULT_PORT = 53317

/** IPv4 在前并按（名称，地址）去重；IPv6 中隐藏链路本地（fe80::/10）与已废弃的站点本地（fec0::/10），其余只计数。 */
internal fun visibleInterfaces(all: List<InterfaceView>): VisibleInterfaces {
    val v4 = all.filter { isIpv4Address(it.addr) }.distinctBy { it.name to it.addr }
    val v6 = all.filter { !isIpv4Address(it.addr) }.distinctBy { it.name to it.addr }.count { !isHiddenIpv6(it.addr) }
    return VisibleInterfaces(v4, v6)
}

private fun isHiddenIpv6(addr: String): Boolean {
    val first = addr.substringBefore(':').toIntOrNull(16) ?: return false
    // fe80::/10 与 fec0::/10 的首个 16 位组分别落在 0xfe80..0xfebf 与 0xfec0..0xfeff
    return first in 0xfe80..0xfeff
}
