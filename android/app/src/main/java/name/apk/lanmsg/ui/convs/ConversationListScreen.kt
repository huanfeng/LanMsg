package name.apk.lanmsg.ui.convs

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Chat
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls

@Composable
fun ConversationListScreen(onOpenChat: (String) -> Unit, onSearch: () -> Unit) {
    val vm: ConversationListViewModel = viewModel(
        factory = viewModelFactory {
            initializer { ConversationListViewModel(ServiceLocator.coreHolder.api, ServiceLocator.localPrefs) }
        },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    ConversationListContent(state, onOpenChat, vm::toggleMute, onSearch)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConversationListContent(
    state: ConversationListViewModel.UiState,
    onOpenChat: (String) -> Unit,
    onToggleMute: (String) -> Unit,
    onSearch: () -> Unit,
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(R.string.err_generic, it) }
    LaunchedEffect(errorText) { errorText?.let { snackbar.showSnackbar(it) } }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.tab_messages)) },
                actions = {
                    IconButton(onClick = onSearch) {
                        Icon(Icons.Outlined.Search, contentDescription = stringResource(R.string.search_hint))
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = c.surface,
                    titleContentColor = c.ink,
                    actionIconContentColor = c.ink,
                ),
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { inner ->
        Box(Modifier.padding(inner).fillMaxSize()) {
            when {
                state.loading -> CircularProgressIndicator(color = LanMsgControls.progress, modifier = Modifier.align(Alignment.Center))
                state.rows.isEmpty() -> EmptyState(stringResource(R.string.convs_empty), Icons.AutoMirrored.Outlined.Chat)
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(state.rows, key = { it.summary.conv }) { row ->
                        ConversationRow(row, onClick = { onOpenChat(row.summary.conv) }, onToggleMute = { onToggleMute(row.summary.conv) })
                    }
                }
            }
        }
    }
}
