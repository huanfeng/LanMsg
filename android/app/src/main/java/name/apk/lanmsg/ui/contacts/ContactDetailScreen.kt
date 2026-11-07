package name.apk.lanmsg.ui.contacts

import androidx.compose.foundation.background
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
import androidx.compose.material3.Button
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
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.Avatar
import name.apk.lanmsg.ui.common.Format
import name.apk.lanmsg.ui.common.OptionRow
import name.apk.lanmsg.ui.common.StatusDot
import name.apk.lanmsg.ui.common.SwitchRow
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.ContactView

@Composable
fun ContactDetailScreen(user: String, onBack: () -> Unit, onOpenChat: (String) -> Unit) {
    val vm: ContactDetailViewModel = viewModel(
        key = "contact:$user",
        factory = viewModelFactory { initializer { ContactDetailViewModel(ServiceLocator.coreHolder.api, user) } },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    LaunchedEffect(state.openConv) {
        state.openConv?.let {
            onOpenChat(it)
            vm.openConvHandled()
        }
    }
    ContactDetailContent(state, vm::openDm, vm::setAutoAccept, vm::setAskAbove, onBack, vm::clearError)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ContactDetailContent(
    state: ContactDetailViewModel.UiState,
    onOpenDm: () -> Unit,
    onAutoAccept: (Boolean) -> Unit,
    onAskAbove: (ULong?) -> Unit,
    onBack: () -> Unit,    onDismissError: () -> Unit = {},
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(it) }
    LaunchedEffect(errorText) { errorText?.let { snackbar.showSnackbar(it); onDismissError() } }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(state.contact?.displayName.orEmpty()) },
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
        val contact = state.contact
        if (contact == null) {
            Box(Modifier.padding(inner).fillMaxSize())
        } else {
            Column(Modifier.padding(inner).fillMaxSize().verticalScroll(rememberScrollState())) {
                Header(contact, onOpenDm)
                HorizontalDivider(color = c.line)
                Column(Modifier.fillMaxWidth().background(c.surface)) {
                    SwitchRow(stringResource(R.string.contact_auto_accept), contact.autoAccept, onAutoAccept)
                    HorizontalDivider(color = c.line)
                    OptionRow(
                        title = stringResource(R.string.contact_ask_above),
                        current = askLabel(contact.askAboveBytes),
                        options = ContactDetailViewModel.ASK_ABOVE,
                        label = { askLabel(it) },
                        onSelect = onAskAbove,
                    )
                }
                HorizontalDivider(color = c.line)
                Column(Modifier.fillMaxWidth().padding(top = 8.dp)) {
                    for (d in contact.devices) {
                        Row(
                            Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            StatusDot(online = d.online)
                            Text(d.name, color = c.inkMuted, style = MaterialTheme.typography.bodyMedium)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun Header(contact: ContactView, onOpenDm: () -> Unit) {
    val c = LanMsgColors.current
    Column(
        Modifier.fillMaxWidth().background(c.surface).padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Avatar(name = contact.displayName, userIdHex = contact.userId, size = 72.dp, online = contact.online)
        Text(contact.displayName, color = c.ink, style = MaterialTheme.typography.titleLarge)
        Text(contact.userShort, color = c.inkSubtle, style = MaterialTheme.typography.bodySmall)
        Button(onClick = onOpenDm) { Text(stringResource(R.string.contact_send_message)) }
    }
}

@Composable
private fun askLabel(bytes: ULong?): String =
    if (bytes == null) stringResource(R.string.size_unlimited) else Format.size(bytes.toLong())
