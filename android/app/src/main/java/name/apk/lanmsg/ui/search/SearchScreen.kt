package name.apk.lanmsg.ui.search

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.runtime.remember
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.common.convTitle
import name.apk.lanmsg.ui.common.systemEventText
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.common.formatTime
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.MessageBodyView

@Composable
fun SearchScreen(onBack: () -> Unit, onOpenChat: (String) -> Unit) {
    val vm: SearchViewModel = viewModel(factory = viewModelFactory { initializer { SearchViewModel(ServiceLocator.coreHolder.api) } })
    val state by vm.state.collectAsStateWithLifecycle()
    SearchContent(state, vm::setQuery, onBack, onOpenChat)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SearchContent(
    state: SearchViewModel.UiState,
    onQuery: (String) -> Unit,
    onBack: () -> Unit,
    onOpenChat: (String) -> Unit,
) {
    val c = LanMsgColors.current
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            Column {
                TopAppBar(
                    title = {
                        TextField(
                            value = state.query,
                            onValueChange = onQuery,
                            placeholder = { Text(stringResource(R.string.search_hint), color = c.inkSubtle) },
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                            colors = TextFieldDefaults.colors(
                                focusedContainerColor = c.surface,
                                unfocusedContainerColor = c.surface,
                                focusedTextColor = c.ink,
                                unfocusedTextColor = c.ink,
                                cursorColor = c.signal,
                                focusedIndicatorColor = c.surface,
                                unfocusedIndicatorColor = c.surface,
                            ),
                            modifier = Modifier.fillMaxWidth().focusRequester(focus),
                        )
                    },
                    navigationIcon = {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.back))
                        }
                    },
                    colors = TopAppBarDefaults.topAppBarColors(containerColor = c.surface, navigationIconContentColor = c.ink),
                )
                HorizontalDivider(color = c.line)
            }
        },
    ) { inner ->
        Box(Modifier.padding(inner).fillMaxSize()) {
            when {
                state.coreOutdated -> EmptyState(stringResource(R.string.err_core_outdated), Icons.Outlined.Search)
                state.error != null -> EmptyState(stringResource(state.error), Icons.Outlined.Search)
                state.rows.isNotEmpty() -> LazyColumn(Modifier.fillMaxSize()) {
                    items(state.rows, key = { it.msg.id }) { row -> ResultRow(row) { onOpenChat(row.msg.conv) } }
                }
                state.loading -> CircularProgressIndicator(color = LanMsgControls.progress, modifier = Modifier.align(Alignment.Center))
                state.query.isNotBlank() -> EmptyState(stringResource(R.string.search_empty), Icons.Outlined.Search)
            }
        }
    }
}

@Composable
private fun ResultRow(row: SearchViewModel.Row, onClick: () -> Unit) {
    val c = LanMsgColors.current
    val msg = row.msg
    val snippet = when (val b = msg.body) {
        is MessageBodyView.Text -> b.text
        is MessageBodyView.Files -> b.items.joinToString(", ") { it.name }
        is MessageBodyView.Folder -> b.name
        is MessageBodyView.System -> systemEventText(b.event)
        else -> ""
    }
    val title = convTitle(row.kind, row.convTitle)
    Column(Modifier.fillMaxWidth().background(c.surface).clickable(onClick = onClick)) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    title,
                    color = c.ink,
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                Text(formatTime(msg.hlcMillis), color = c.inkSubtle, fontSize = 12.sp)
            }
            if (msg.authorDeviceName.isNotEmpty()) {
                Text(
                    stringResource(R.string.chat_from_device, msg.authorDeviceName),
                    color = c.inkMuted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
            Text(snippet, color = c.inkMuted, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        HorizontalDivider(color = c.line)
    }
}
