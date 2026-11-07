package name.apk.lanmsg.ui.files

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.FilterList
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import coil.compose.AsyncImage
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.files.FileOpener
import name.apk.lanmsg.ui.chat.FileStateMark
import name.apk.lanmsg.ui.chat.fileKindIcon
import name.apk.lanmsg.ui.chat.fileKindOf
import name.apk.lanmsg.ui.chat.middleEllipsis
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.common.Format
import name.apk.lanmsg.ui.common.formatTime
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.FileKindFilter
import uniffi.lanmsg_api.FileView
import java.io.File

@Composable
fun FilesScreen(initialConv: String?, onOpenChat: (String) -> Unit, onForward: (List<String>) -> Unit) {
    val vm: FilesViewModel = viewModel(
        key = "files-$initialConv",
        factory = viewModelFactory { initializer { FilesViewModel(ServiceLocator.core, initialConv) } },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    val ctx = LocalContext.current
    FilesContent(
        state = state,
        onKind = vm::setKind,
        onState = vm::setState,
        onDevice = vm::setSourceDevice,
        onQuery = vm::setQuery,
        onDismissError = vm::clearError,
        onClick = { v ->
            when (v.item.state) {
                BlobState.REMOTE -> vm.download(v.item.hash)
                BlobState.LOCAL -> v.item.localPath?.let { FileOpener.open(ctx, it, v.item.mime) }
                else -> Unit
            }
        },
        onCancel = vm::cancel,
        onOpenChat = onOpenChat,
        onForward = onForward,
    )
}

private val kindTabs = listOf(
    FileKindFilter.ALL to R.string.files_all,
    FileKindFilter.IMAGE to R.string.files_image,
    FileKindFilter.VIDEO to R.string.files_video,
    FileKindFilter.DOCUMENT to R.string.files_document,
    FileKindFilter.ARCHIVE to R.string.files_archive,
    FileKindFilter.OTHER to R.string.files_other,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FilesContent(
    state: FilesViewModel.UiState,
    onKind: (FileKindFilter) -> Unit,
    onState: (BlobState?) -> Unit,
    onDevice: (String?) -> Unit,
    onQuery: (String) -> Unit,
    onDismissError: () -> Unit,
    onClick: (FileView) -> Unit,
    onCancel: (String) -> Unit,
    onOpenChat: (String) -> Unit,
    onForward: (List<String>) -> Unit,
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(R.string.err_generic, it) }
    LaunchedEffect(errorText) {
        errorText?.let { snackbar.showSnackbar(it); onDismissError() }
    }
    var query by remember { mutableStateOf("") }
    var filterMenu by remember { mutableStateOf(false) }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            Column {
                TopAppBar(
                    title = { Text(stringResource(R.string.tab_files)) },
                    actions = {
                        Box {
                            IconButton(onClick = { filterMenu = true }) {
                                val active = state.filter.state != null || state.filter.sourceDevice != null
                                Icon(
                                    Icons.Outlined.FilterList,
                                    contentDescription = stringResource(R.string.files_filter),
                                    tint = if (active) c.signal else c.ink,
                                )
                            }
                            FilterMenu(state, filterMenu, { filterMenu = false }, onState, onDevice)
                        }
                    },
                    colors = TopAppBarDefaults.topAppBarColors(
                        containerColor = c.surface,
                        titleContentColor = c.ink,
                        actionIconContentColor = c.ink,
                    ),
                )
                Column(Modifier.background(c.surface).padding(horizontal = 16.dp)) {
                    OutlinedTextField(
                        value = query,
                        onValueChange = { query = it; onQuery(it) },
                        placeholder = { Text(stringResource(R.string.files_search_hint), color = c.inkSubtle) },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                        colors = LanMsgControls.textField(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Row(
                        Modifier.horizontalScroll(rememberScrollState()).padding(vertical = 8.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        for ((kind, label) in kindTabs) {
                            FilterChip(
                                selected = state.filter.kind == kind,
                                onClick = { onKind(kind) },
                                label = { Text(stringResource(label)) },
                                colors = FilterChipDefaults.filterChipColors(
                                    labelColor = c.inkMuted,
                                    selectedContainerColor = c.signalSoft,
                                    selectedLabelColor = c.signal,
                                ),
                                border = FilterChipDefaults.filterChipBorder(
                                    enabled = true,
                                    selected = state.filter.kind == kind,
                                    borderColor = c.line,
                                    selectedBorderColor = c.signal,
                                ),
                            )
                        }
                    }
                }
                HorizontalDivider(color = c.line)
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { inner ->
        Box(Modifier.padding(inner).fillMaxSize()) {
            when {
                state.coreOutdated -> EmptyState(stringResource(R.string.err_core_outdated), Icons.Outlined.Folder)
                state.loading -> CircularProgressIndicator(color = LanMsgControls.progress, modifier = Modifier.align(Alignment.Center))
                state.items.isEmpty() -> EmptyState(stringResource(R.string.files_empty), Icons.Outlined.Folder)
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(state.items, key = { it.msg + it.item.hash }) { v ->
                        FileRow(v, state.filter.conv == null, state.progress[v.item.hash], onClick, onCancel, onOpenChat, onForward)
                    }
                }
            }
        }
    }
}

@Composable
private fun FilterMenu(
    state: FilesViewModel.UiState,
    expanded: Boolean,
    onDismiss: () -> Unit,
    onState: (BlobState?) -> Unit,
    onDevice: (String?) -> Unit,
) {
    val c = LanMsgColors.current
    val f = state.filter
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss, containerColor = c.surface) {
        // “全部”同时清除状态与来源设备筛选
        MenuItem(stringResource(R.string.files_all), f.state == null && f.sourceDevice == null) {
            onDismiss(); onState(null); onDevice(null)
        }
        MenuItem(stringResource(R.string.files_state_downloaded), f.state == BlobState.LOCAL) {
            onDismiss(); onState(BlobState.LOCAL)
        }
        MenuItem(stringResource(R.string.files_state_remote), f.state == BlobState.REMOTE) {
            onDismiss(); onState(BlobState.REMOTE)
        }
        if (state.devices.isNotEmpty()) HorizontalDivider(color = c.line)
        for (d in state.devices) {
            val label = when {
                d.self -> stringResource(R.string.devices_this)
                d.subLabel != null -> stringResource(R.string.files_source_device, d.label, d.subLabel)
                else -> d.label
            }
            MenuItem(label, f.sourceDevice == d.deviceId) {
                onDismiss(); onDevice(if (f.sourceDevice == d.deviceId) null else d.deviceId)
            }
        }
    }
}

@Composable
private fun MenuItem(text: String, selected: Boolean, onClick: () -> Unit) {
    val c = LanMsgColors.current
    DropdownMenuItem(
        text = { Text(text, color = if (selected) c.signal else c.ink) },
        trailingIcon = if (selected) {
            { Icon(Icons.Outlined.Check, contentDescription = null, tint = c.signal) }
        } else null,
        onClick = onClick,
    )
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FileRow(
    v: FileView,
    showConvTitle: Boolean,
    progress: Pair<ULong, ULong>?,
    onClick: (FileView) -> Unit,
    onCancel: (String) -> Unit,
    onOpenChat: (String) -> Unit,
    onForward: (List<String>) -> Unit,
) {
    val c = LanMsgColors.current
    val item = v.item
    var menu by remember { mutableStateOf(false) }
    Box {
        Column(Modifier.fillMaxWidth().background(c.surface)) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .combinedClickable(onClick = { onClick(v) }, onLongClick = { menu = true })
                    .padding(horizontal = 16.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Thumb(v)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(middleEllipsis(item.name), color = c.ink, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(Format.size(item.size.toLong()), color = c.inkMuted, fontSize = 12.sp)
                    if (item.sourceLabel.isNotEmpty()) {
                        Text(item.sourceLabel, color = c.inkSubtle, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    // 已在某个会话范围内时标题是重复信息，不再显示
                    if (showConvTitle && v.convTitle.isNotEmpty()) {
                        Text(v.convTitle, color = c.inkSubtle, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
                Spacer(Modifier.width(8.dp))
                Column(horizontalAlignment = Alignment.End) {
                    Text(formatTime(v.hlcMillis), color = c.inkSubtle, fontSize = 12.sp)
                    FileStateMark(item, progress, c.inkMuted, onCancel)
                }
            }
            HorizontalDivider(color = c.line)
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = c.surface) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.file_open_conversation), color = c.ink) },
                onClick = { menu = false; onOpenChat(v.conv) },
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.action_forward), color = c.ink) },
                onClick = { menu = false; onForward(listOf(item.hash)) },
            )
        }
    }
}

@Composable
private fun Thumb(v: FileView) {
    val c = LanMsgColors.current
    val thumb = v.item.thumbPath
    Box(Modifier.size(48.dp).clip(RoundedCornerShape(6.dp)).background(c.canvas), contentAlignment = Alignment.Center) {
        if (thumb != null) {
            AsyncImage(model = File(thumb), contentDescription = v.item.name, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        } else {
            Icon(fileKindIcon(fileKindOf(v.item.mime)), contentDescription = null, tint = c.inkSubtle, modifier = Modifier.size(28.dp))
        }
    }
}
