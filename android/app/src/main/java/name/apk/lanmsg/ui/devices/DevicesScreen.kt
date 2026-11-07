package name.apk.lanmsg.ui.devices

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.BorderStroke
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.ConfirmDialog
import name.apk.lanmsg.ui.common.PathBadge
import name.apk.lanmsg.ui.common.StatusDot
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.DeviceView

@Composable
fun DevicesScreen(onBack: () -> Unit, onAdd: () -> Unit, onJoin: () -> Unit) {
    val vm: DevicesViewModel = viewModel(factory = viewModelFactory { initializer { DevicesViewModel(ServiceLocator.core) } })
    val state by vm.state.collectAsStateWithLifecycle()
    DevicesContent(state, onBack, onAdd, onJoin, vm::askRemove, vm::confirmRemove, vm::dismiss)
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun DevicesContent(
    state: DevicesViewModel.UiState,
    onBack: () -> Unit,
    onAdd: () -> Unit,
    onJoin: () -> Unit,
    onAskRemove: (DeviceView) -> Unit,
    onConfirmRemove: () -> Unit,
    onDismiss: () -> Unit,
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(R.string.err_generic, it) }
    LaunchedEffect(errorText) { errorText?.let { snackbar.showSnackbar(it) } }

    state.confirmRemove?.let { d ->
        ConfirmDialog(
            title = stringResource(R.string.devices_remove),
            text = stringResource(R.string.devices_remove_confirm, d.name),
            confirmLabel = stringResource(R.string.devices_remove),
            onConfirm = onConfirmRemove,
            onDismiss = onDismiss,
        )
    }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.me_my_devices)) },
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
        Column(Modifier.padding(inner).fillMaxSize()) {
            LazyColumn(Modifier.weight(1f).fillMaxWidth()) {
                items(state.devices, key = { it.deviceId }) { d ->
                    Row(
                        Modifier.fillMaxWidth().background(c.surface)
                            .combinedClickable(onClick = {}, onLongClick = { onAskRemove(d) })
                            .padding(horizontal = 16.dp, vertical = 14.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(
                            if (d.deviceType == "desktop") Icons.Outlined.Computer else Icons.Outlined.PhoneAndroid,
                            contentDescription = null,
                            tint = c.inkMuted,
                        )
                        Column(Modifier.weight(1f).padding(start = 16.dp)) {
                            Text(d.name, color = c.ink, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(
                                if (d.isSelf) stringResource(R.string.devices_this) else d.deviceShort,
                                color = c.inkSubtle,
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        if (!d.isSelf) PathBadge(d.path)
                        StatusDot(isDeviceOnline(d), Modifier.padding(start = 12.dp))
                    }
                    HorizontalDivider(color = c.line)
                }
            }
            Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = onAdd, modifier = Modifier.fillMaxWidth()) { Text(stringResource(R.string.devices_add)) }
                OutlinedButton(
                    onClick = onJoin,
                    modifier = Modifier.fillMaxWidth(),
                    border = BorderStroke(1.dp, c.line),
                ) { Text(stringResource(R.string.devices_join), color = c.signal) }
            }
        }
    }
}

/** core 按 peers 判断 online，本机不在 peers 中，恒为 false；本机视为在线。 */
internal fun isDeviceOnline(d: DeviceView) = d.isSelf || d.online
