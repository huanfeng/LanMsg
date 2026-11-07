package name.apk.lanmsg.ui.contacts

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Group
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.Avatar
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.common.SectionHeader
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors

@Composable
fun ContactsScreen(onOpenRequests: () -> Unit, onOpenContact: (String) -> Unit, onAddDevice: () -> Unit) {
    val vm: ContactsViewModel = viewModel(factory = viewModelFactory { initializer { ContactsViewModel(ServiceLocator.coreHolder.api) } })
    val state by vm.state.collectAsStateWithLifecycle()
    ContactsContent(state, vm::add, vm::toastShown, onOpenRequests, onOpenContact, onAddDevice)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ContactsContent(
    state: ContactsViewModel.UiState,
    onAdd: (String) -> Unit,
    onToastShown: () -> Unit,
    onOpenRequests: () -> Unit,
    onOpenContact: (String) -> Unit,
    onAddDevice: () -> Unit,
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val toastText = state.toast?.let { stringResource(it) }
    val errorText = state.error?.let { stringResource(it) }
    LaunchedEffect(toastText) {
        if (toastText != null) {
            snackbar.showSnackbar(toastText)
            onToastShown()
        }
    }
    LaunchedEffect(errorText) { errorText?.let { snackbar.showSnackbar(it) } }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.tab_contacts)) },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = c.surface, titleContentColor = c.ink),
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { inner ->
        Column(Modifier.padding(inner).fillMaxSize()) {
            Row(
                Modifier.fillMaxWidth().background(c.surface).clickable(onClick = onOpenRequests).padding(horizontal = 16.dp, vertical = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Outlined.PersonAdd, contentDescription = null, tint = c.signal)
                Text(
                    stringResource(R.string.contacts_requests, state.requestCount),
                    color = c.ink,
                    style = MaterialTheme.typography.bodyLarge,
                    modifier = Modifier.weight(1f).padding(start = 16.dp),
                )
                Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null, tint = c.inkSubtle)
            }
            HorizontalDivider(color = c.line)
            if (state.contacts.isEmpty() && state.nearby.isEmpty()) {
                EmptyState(stringResource(R.string.contacts_empty), Icons.Outlined.Group) {
                    Button(onClick = onAddDevice) { Text(stringResource(R.string.devices_add)) }
                }
            } else {
                LazyColumn(Modifier.fillMaxSize()) {
                    items(state.contacts, key = { "c:" + it.userId }) { contact ->
                        PersonRow(contact.displayName, contact.userId, contact.userShort, contact.online, onClick = { onOpenContact(contact.userId) })
                    }
                    if (state.nearby.isNotEmpty()) {
                        item(key = "nearby-header") { SectionHeader(stringResource(R.string.contacts_nearby)) }
                        items(state.nearby, key = { "n:" + it.userId }) { peer ->
                            PersonRow(peer.displayName, peer.userId, peer.deviceName, true, onClick = null) {
                                TextButton(onClick = { onAdd(peer.userId) }, colors = LanMsgButtons.text()) {
                                    Text(stringResource(R.string.contacts_add))
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun PersonRow(
    name: String,
    userId: String,
    subtitle: String,
    online: Boolean,
    onClick: (() -> Unit)?,
    trailing: @Composable (() -> Unit)? = null,
) {
    val c = LanMsgColors.current
    Column(Modifier.background(c.surface)) {
        Row(
            Modifier.fillMaxWidth().let { if (onClick != null) it.clickable(onClick = onClick) else it }
                .padding(horizontal = 16.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Avatar(name = name, userIdHex = userId, online = online)
            Column(Modifier.weight(1f)) {
                Text(name, color = c.ink, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(subtitle, color = c.inkSubtle, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            trailing?.invoke()
        }
        HorizontalDivider(color = c.line)
    }
}
