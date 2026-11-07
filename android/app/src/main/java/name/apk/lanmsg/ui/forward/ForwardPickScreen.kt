package name.apk.lanmsg.ui.forward

import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Chat
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.common.ConversationAvatar
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.ConversationSummary

/** 转发：选择目标会话。[hashes] 为空时直接返回。成功后 [onDone] 收到目标会话。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ForwardPickScreen(hashes: List<String>, onBack: () -> Unit, onDone: (String) -> Unit) {
    if (hashes.isEmpty()) {
        LaunchedEffect(Unit) { onBack() }
        return
    }
    val vm: ForwardPickViewModel = viewModel(
        key = "forward-${hashes.joinToString(",")}",
        factory = viewModelFactory { initializer { ForwardPickViewModel(ServiceLocator.core, hashes) } },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    ForwardPickContent(state, onBack, vm::pick, vm::clearError, onDone)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ForwardPickContent(
    state: ForwardPickViewModel.UiState,
    onBack: () -> Unit,
    onPick: (String) -> Unit,
    onDismissError: () -> Unit,
    onDone: (String) -> Unit,
) {
    val c = LanMsgColors.current
    val ctx = LocalContext.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(R.string.err_generic, it) }
    LaunchedEffect(errorText) { errorText?.let { snackbar.showSnackbar(it); onDismissError() } }
    val forwarded = stringResource(R.string.toast_forwarded)
    LaunchedEffect(state.done) {
        state.done?.let {
            Toast.makeText(ctx, forwarded, Toast.LENGTH_SHORT).show()
            onDone(it)
        }
    }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.forward_title)) },
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
            when {
                state.coreOutdated -> EmptyState(stringResource(R.string.err_core_outdated), Icons.AutoMirrored.Outlined.Chat)
                state.items.isEmpty() -> EmptyState(stringResource(R.string.convs_empty), Icons.AutoMirrored.Outlined.Chat)
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(state.items, key = ConversationSummary::conv) { s ->
                        PickRow(s) { onPick(s.conv) }
                    }
                }
            }
        }
    }
}

@Composable
private fun PickRow(s: ConversationSummary, onClick: () -> Unit) {
    val c = LanMsgColors.current
    val title = if (s.kind == ConvKind.SELF_DEVICES) stringResource(R.string.conv_self_title) else s.title
    Column(Modifier.fillMaxWidth().background(c.surface).clickable(onClick = onClick)) {
        Row(Modifier.padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            ConversationAvatar(s, size = 40.dp, online = s.peerOnline)
            Spacer(Modifier.width(12.dp))
            Text(title, color = c.ink, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        HorizontalDivider(color = c.line)
    }
}
