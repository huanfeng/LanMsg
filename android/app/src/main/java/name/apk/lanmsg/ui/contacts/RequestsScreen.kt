package name.apk.lanmsg.ui.contacts

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.common.requestPreviewText
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.Avatar
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.RequestAction
import uniffi.lanmsg_api.RequestKind
import uniffi.lanmsg_api.RequestView

@Composable
fun RequestsScreen(onBack: () -> Unit) {
    val vm: RequestsViewModel = viewModel(factory = viewModelFactory { initializer { RequestsViewModel(ServiceLocator.coreHolder.api) } })
    val state by vm.state.collectAsStateWithLifecycle()
    RequestsContent(state, vm::respond, onBack, vm::clearError)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RequestsContent(state: RequestsViewModel.UiState, onRespond: (String, RequestAction) -> Unit, onBack: () -> Unit, onDismissError: () -> Unit = {}) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(it) }
    LaunchedEffect(errorText) { errorText?.let { snackbar.showSnackbar(it); onDismissError() } }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.requests_title)) },
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
        if (state.items.isEmpty()) {
            Column(Modifier.padding(inner).fillMaxSize()) {
                EmptyState(stringResource(R.string.requests_empty), Icons.Outlined.PersonAdd)
            }
        } else {
            LazyColumn(Modifier.padding(inner).fillMaxSize()) {
                items(state.items, key = { it.request }) { RequestItem(it, onRespond) }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun RequestItem(item: RequestView, onRespond: (String, RequestAction) -> Unit) {
    val c = LanMsgColors.current
    val kind = when (item.kind) {
        RequestKind.CONTACT -> R.string.req_kind_contact
        RequestKind.MESSAGE -> R.string.req_kind_message
        RequestKind.FILE -> R.string.req_kind_file
    }
    Column(Modifier.fillMaxWidth().background(c.surface)) {
        Row(Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(name = item.fromName, userIdHex = item.fromUser)
            Column(Modifier.weight(1f).padding(start = 12.dp)) {
                Text(item.fromName, color = c.ink, style = MaterialTheme.typography.bodyLarge)
                Text(stringResource(kind), color = c.inkMuted, style = MaterialTheme.typography.bodyMedium)
                item.sourceAddr?.let {
                    Text(stringResource(R.string.req_source_addr, it), color = c.inkSubtle, style = MaterialTheme.typography.bodySmall)
                }
                if (item.addrChanged) {
                    Text(stringResource(R.string.req_addr_changed), color = c.danger, style = MaterialTheme.typography.bodySmall)
                }
                requestPreviewText(item.preview)?.let {
                    Text(it, color = c.inkSubtle, style = MaterialTheme.typography.bodySmall, maxLines = 2)
                }
            }
        }
        FlowRow(Modifier.padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            TextButton(onClick = { onRespond(item.request, RequestAction.ACCEPT_ONCE) }, colors = LanMsgButtons.text()) {
                Text(stringResource(R.string.req_accept_once))
            }
            if (!item.addrChanged) {
                TextButton(onClick = { onRespond(item.request, RequestAction.ADD_CONTACT) }, colors = LanMsgButtons.text()) {
                    Text(stringResource(R.string.req_add_contact))
                }
            }
            TextButton(onClick = { onRespond(item.request, RequestAction.REJECT) }, colors = LanMsgButtons.text()) {
                Text(stringResource(R.string.req_reject))
            }
            TextButton(
                onClick = { onRespond(item.request, RequestAction.BLOCK) },
                colors = ButtonDefaults.textButtonColors(contentColor = c.danger),
            ) { Text(stringResource(R.string.req_block)) }
        }
        HorizontalDivider(color = c.line)
    }
}
