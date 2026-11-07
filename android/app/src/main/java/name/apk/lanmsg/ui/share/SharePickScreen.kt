package name.apk.lanmsg.ui.share

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Chat
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.common.Avatar
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.ConvKind

/** 分享入口的会话选择页；发送成功后调用 [onSent] 进入聊天页。 */
@Composable
fun SharePickScreen(vm: SharePickViewModel, onBack: () -> Unit, onSent: (String) -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    LaunchedEffect(state.sentTo) {
        state.sentTo?.let {
            vm.reset()
            onSent(it)
        }
    }
    SharePickContent(state, onBack, vm::pick, vm::clearError)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SharePickContent(
    state: SharePickViewModel.UiState,
    onBack: () -> Unit,
    onPick: (String) -> Unit,
    onDismissError: () -> Unit,
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let {
        if (it == SharePickViewModel.ERROR_IMPORT_FAILED) stringResource(R.string.share_import_failed) else stringResource(R.string.err_generic, it)
    }
    LaunchedEffect(errorText) {
        if (errorText != null) {
            snackbar.showSnackbar(errorText)
            onDismissError()
        }
    }
    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.share_pick_title)) },
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
        Box(Modifier.padding(inner).fillMaxSize()) {
            if (state.items.isEmpty()) {
                EmptyState(stringResource(R.string.share_empty), Icons.AutoMirrored.Outlined.Chat)
            } else {
                LazyColumn(Modifier.fillMaxSize()) {
                    items(state.items, key = { it.conv }) { s ->
                        val selfDevices = s.kind == ConvKind.SELF_DEVICES
                        val title = if (selfDevices) stringResource(R.string.conv_self_title) else s.title
                        Column(Modifier.fillMaxWidth().background(c.surface).clickable(enabled = !state.busy) { onPick(s.conv) }) {
                            Row(Modifier.padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                                if (selfDevices) {
                                    Box(Modifier.size(40.dp).background(c.signalSoft, CircleShape), contentAlignment = Alignment.Center) {
                                        Icon(Icons.Outlined.Devices, contentDescription = null, tint = c.signal)
                                    }
                                } else {
                                    Avatar(name = title, userIdHex = s.conv, size = 40.dp, online = s.peerOnline)
                                }
                                Spacer(Modifier.width(12.dp))
                                Text(title, color = c.ink, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                            HorizontalDivider(color = c.line)
                        }
                    }
                }
            }
            if (state.busy) {
                CircularProgressIndicator(color = LanMsgControls.progress, modifier = Modifier.align(Alignment.Center))
            }
        }
    }
}
