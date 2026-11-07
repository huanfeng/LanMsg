package name.apk.lanmsg.ui.me

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
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Settings
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.Avatar
import name.apk.lanmsg.ui.common.SectionHeader
import name.apk.lanmsg.ui.common.TextInputDialog
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.StatusDto

@Composable
fun MeScreen(onOpenDevices: () -> Unit, onOpenSettings: () -> Unit) {
    val vm: MeViewModel = viewModel(factory = viewModelFactory { initializer { MeViewModel(ServiceLocator.coreHolder.api) } })
    val state by vm.state.collectAsStateWithLifecycle()
    MeContent(state, vm::rename, vm::setStatus, onOpenDevices, onOpenSettings)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MeContent(
    state: MeViewModel.UiState,
    onRename: (String) -> Unit,
    onStatus: (StatusDto) -> Unit,
    onOpenDevices: () -> Unit,
    onOpenSettings: () -> Unit,
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(R.string.err_generic, it) }
    LaunchedEffect(errorText) { errorText?.let { snackbar.showSnackbar(it) } }
    var editing by remember { mutableStateOf(false) }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.tab_me)) },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = c.surface, titleContentColor = c.ink),
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { inner ->
        val self = state.self
        if (self == null) {
            Box(Modifier.padding(inner).fillMaxSize(), contentAlignment = Alignment.Center) {
                if (state.error == null) CircularProgressIndicator(color = LanMsgControls.progress)
            }
            return@Scaffold
        }
        val name = self.displayName.orEmpty()
        Column(Modifier.padding(inner).fillMaxSize().verticalScroll(rememberScrollState())) {
            Row(
                Modifier.fillMaxWidth().background(c.surface).padding(16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Avatar(name = name, userIdHex = self.userId.orEmpty(), size = 64.dp)
                Column(Modifier.weight(1f)) {
                    Text(name, color = c.ink, style = MaterialTheme.typography.titleLarge)
                    Text(self.userShort.orEmpty(), color = c.inkSubtle, style = MaterialTheme.typography.bodySmall)
                }
                IconButton(onClick = { editing = true }) {
                    Icon(Icons.Outlined.Edit, contentDescription = stringResource(R.string.me_edit_name), tint = c.signal)
                }
            }
            HorizontalDivider(color = c.line)
            SectionHeader(stringResource(R.string.me_status))
            Column(Modifier.fillMaxWidth().background(c.surface)) {
                StatusOption(R.string.status_online, StatusDto.ONLINE, self.status, onStatus)
                StatusOption(R.string.status_invisible, StatusDto.INVISIBLE, self.status, onStatus)
                StatusOption(R.string.status_busy, StatusDto.BUSY, self.status, onStatus)
            }
            HorizontalDivider(color = c.line)
            Column(Modifier.padding(top = 16.dp).fillMaxWidth().background(c.surface)) {
                NavRow(Icons.Outlined.Devices, R.string.me_my_devices, onOpenDevices)
                HorizontalDivider(color = c.line)
                NavRow(Icons.Outlined.Settings, R.string.me_settings, onOpenSettings)
            }
            HorizontalDivider(color = c.line)
        }
        if (editing) {
            TextInputDialog(
                title = stringResource(R.string.me_edit_name),
                initial = name,
                onConfirm = { editing = false; onRename(it) },
                onDismiss = { editing = false },
            )
        }
    }
}

@Composable
private fun StatusOption(label: Int, value: StatusDto, current: StatusDto, onSelect: (StatusDto) -> Unit) {
    val c = LanMsgColors.current
    Row(
        Modifier.fillMaxWidth().clickable { onSelect(value) }.padding(horizontal = 16.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RadioButton(
            selected = current == value,
            onClick = null,
            colors = RadioButtonDefaults.colors(selectedColor = c.signal, unselectedColor = c.inkMuted),
        )
        Text(stringResource(label), color = c.ink, modifier = Modifier.padding(start = 16.dp, top = 12.dp, bottom = 12.dp))
    }
}

@Composable
private fun NavRow(icon: androidx.compose.ui.graphics.vector.ImageVector, label: Int, onClick: () -> Unit) {
    val c = LanMsgColors.current
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = c.signal)
        Text(stringResource(label), color = c.ink, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f).padding(start = 16.dp))
        Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null, tint = c.inkSubtle)
    }
}
