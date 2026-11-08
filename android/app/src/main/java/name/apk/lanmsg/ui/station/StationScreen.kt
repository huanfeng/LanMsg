package name.apk.lanmsg.ui.station

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.Inbox
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Notes
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.draw.alpha
import kotlinx.coroutines.launch
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.files.FileOpener
import name.apk.lanmsg.ui.chat.FileCard
import name.apk.lanmsg.ui.chat.fileKindIcon
import name.apk.lanmsg.ui.chat.fileKindOf
import name.apk.lanmsg.ui.chat.middleEllipsis
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.common.StatusDot
import name.apk.lanmsg.ui.common.errorTextRes
import name.apk.lanmsg.ui.common.formatTime
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.StationItemView

@Composable
fun StationScreen(conv: String, onBack: () -> Unit) {
    val vm: StationViewModel = viewModel(
        key = "station-$conv",
        factory = viewModelFactory {
            initializer { StationViewModel(ServiceLocator.coreHolder.api, conv, ServiceLocator.importer) }
        },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    val ctx = LocalContext.current
    val filePicker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        vm.putUris(uris)
    }
    StationContent(
        state = state,
        onBack = onBack,
        onRefresh = vm::pullRefresh,
        onExpiry = vm::setExpiry,
        onPickFile = { filePicker.launch(arrayOf("*/*")) },
        onPutText = vm::putText,
        onPin = vm::setPinned,
        onRemove = vm::remove,
        onDownload = vm::download,
        onCancel = vm::cancel,
        onOpenFile = { item -> item.localPath?.let { FileOpener.open(ctx, it, item.mime) } },
        onCopyText = { text ->
            val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            cm.setPrimaryClip(ClipData.newPlainText(null, text))
            Toast.makeText(ctx, R.string.toast_copied, Toast.LENGTH_SHORT).show()
        },
        onDismissError = vm::clearError,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun StationContent(
    state: StationViewModel.UiState,
    onBack: () -> Unit,
    onRefresh: () -> Unit,
    onExpiry: (StationViewModel.Expiry) -> Unit,
    onPickFile: () -> Unit,
    onPutText: (String) -> Unit,
    onPin: (item: String, pinned: Boolean) -> Unit,
    onRemove: (String) -> Unit,
    onDownload: (String) -> Unit,
    onCancel: (String) -> Unit,
    onOpenFile: (uniffi.lanmsg_api.FileItemView) -> Unit,
    onCopyText: (String) -> Unit,
    onDismissError: () -> Unit,
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let { stringResource(errorTextRes(it)) }
    LaunchedEffect(errorText) { errorText?.let { snackbar.showSnackbar(it); onDismissError() } }
    val scope = rememberCoroutineScope()
    val offlineText = stringResource(R.string.station_host_offline_banner)
    var expiryMenu by remember { mutableStateOf(false) }
    var addMenu by remember { mutableStateOf(false) }
    var textDialog by remember { mutableStateOf(false) }

    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(state.name, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                        StatusDot(state.hostOnline, Modifier.padding(start = 8.dp))
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.back))
                    }
                },
                actions = {
                    Box {
                        IconButton(onClick = { expiryMenu = true }) {
                            Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.station_expiry))
                        }
                        DropdownMenu(expanded = expiryMenu, onDismissRequest = { expiryMenu = false }, containerColor = c.surface) {
                            Text(stringResource(R.string.station_expiry), color = c.inkMuted, fontSize = 12.sp, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
                            for (e in StationViewModel.Expiry.entries) {
                                val selected = e == state.expiry
                                DropdownMenuItem(
                                    text = { Text(stringResource(expiryLabel(e)), color = if (selected) c.signal else c.ink) },
                                    trailingIcon = if (selected) {
                                        { Icon(Icons.Outlined.Check, contentDescription = null, tint = c.signal) }
                                    } else null,
                                    onClick = { expiryMenu = false; onExpiry(e) },
                                )
                            }
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = c.surface,
                    titleContentColor = c.ink,
                    navigationIconContentColor = c.ink,
                    actionIconContentColor = c.ink,
                ),
            )
        },
        floatingActionButton = {
            Box {
                // 主机离线时放入只会白白导入 outbox 再失败：降透明并提示，不弹添加菜单
                val hostOffline = !state.hostOnline && !state.loading
                FloatingActionButton(
                    onClick = { if (hostOffline) scope.launch { snackbar.showSnackbar(offlineText) } else addMenu = true },
                    containerColor = c.signalFill,
                    contentColor = c.onFill,
                    modifier = Modifier.alpha(if (hostOffline) 0.5f else 1f),
                ) {
                    Icon(Icons.Outlined.Add, contentDescription = stringResource(R.string.station_add))
                }
                DropdownMenu(expanded = addMenu, onDismissRequest = { addMenu = false }, containerColor = c.surface) {
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.station_add_file), color = c.ink) },
                        onClick = { addMenu = false; onPickFile() },
                    )
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.station_add_text), color = c.ink) },
                        onClick = { addMenu = false; textDialog = true },
                    )
                }
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { inner ->
        Column(Modifier.padding(inner).fillMaxSize()) {
            if (!state.hostOnline && !state.loading) {
                Text(
                    stringResource(R.string.station_host_offline_banner),
                    color = c.inkMuted,
                    fontSize = 13.sp,
                    modifier = Modifier.fillMaxWidth().background(c.signalSoft).padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }
            // 连接主机最坏要十几秒才有结果：期间只显示进度条
            if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth(), color = LanMsgControls.progress)
            PullToRefreshBox(isRefreshing = state.refreshing, onRefresh = onRefresh, modifier = Modifier.fillMaxSize()) {
                when {
                    state.loading -> CircularProgressIndicator(color = LanMsgControls.progress, modifier = Modifier.align(Alignment.Center))
                    state.items.isEmpty() -> EmptyState(stringResource(R.string.station_items_empty), Icons.Outlined.Inbox)
                    else -> LazyColumn(Modifier.fillMaxSize()) {
                        items(state.items, key = { it.item }) { item ->
                            ItemRow(item, item.file?.let { state.progress[it.hash] }, onPin, onRemove, onDownload, onCancel, onOpenFile, onCopyText)
                        }
                    }
                }
            }
        }
    }
    if (textDialog) {
        TextItemDialog(onConfirm = { textDialog = false; onPutText(it) }, onDismiss = { textDialog = false })
    }
}

private fun expiryLabel(e: StationViewModel.Expiry): Int = when (e) {
    StationViewModel.Expiry.NEVER -> R.string.station_expiry_never
    StationViewModel.Expiry.DAY1 -> R.string.station_expiry_1d
    StationViewModel.Expiry.DAY7 -> R.string.station_expiry_7d
    StationViewModel.Expiry.DAY30 -> R.string.station_expiry_30d
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ItemRow(
    item: StationItemView,
    progress: Pair<ULong, ULong>?,
    onPin: (String, Boolean) -> Unit,
    onRemove: (String) -> Unit,
    onDownload: (String) -> Unit,
    onCancel: (String) -> Unit,
    onOpenFile: (uniffi.lanmsg_api.FileItemView) -> Unit,
    onCopyText: (String) -> Unit,
) {
    val c = LanMsgColors.current
    var menu by remember { mutableStateOf(false) }
    val unavailable = item.unavailable == true
    val file = item.file
    val text = item.text
    Box {
        Column(Modifier.fillMaxWidth().background(c.surface)) {
            val time = formatTime(item.addedAt)
            val pin: (@Composable () -> Unit)? = if (item.pinned) {
                {
                    Icon(Icons.Outlined.PushPin, contentDescription = null, tint = c.signal, modifier = Modifier.padding(end = 4.dp).size(14.dp))
                }
            } else null
            if (file != null && !unavailable) {
                FileCard(
                    item = file,
                    progress = progress,
                    content = c.ink,
                    muted = c.inkMuted,
                    onDownload = onDownload,
                    onCancel = onCancel,
                    onOpen = onOpenFile,
                    onLongClick = { menu = true },
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
                    fillWidth = true,
                    sizeSuffix = time,
                    titlePrefix = pin,
                )
            } else {
                // 文字条目与已失效条目：左图标 + 标题 + 副标题，与文件卡片同一节奏
                Row(
                    Modifier
                        .fillMaxWidth()
                        .combinedClickable(
                            enabled = text != null && !unavailable,
                            onClick = { text?.let(onCopyText) },
                            onLongClick = { menu = true },
                        )
                        .padding(horizontal = 16.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    val icon = if (file != null) fileKindIcon(fileKindOf(file.mime)) else Icons.Outlined.Notes
                    Icon(icon, contentDescription = null, tint = if (unavailable) c.inkSubtle else c.inkMuted, modifier = Modifier.size(32.dp))
                    Spacer(Modifier.width(10.dp))
                    Column(Modifier.weight(1f)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            pin?.invoke()
                            Text(
                                if (file != null) middleEllipsis(file.name) else text.orEmpty(),
                                color = if (unavailable) c.inkSubtle else c.ink,
                                style = MaterialTheme.typography.bodyMedium,
                                maxLines = if (file != null) 1 else 3,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        if (unavailable) {
                            Text(stringResource(R.string.station_item_unavailable) + " \u00B7 " + time, color = c.danger, fontSize = 12.sp)
                        } else {
                            Text(time, color = c.inkMuted, fontSize = 12.sp)
                        }
                    }
                }
            }
            HorizontalDivider(color = c.line)
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = c.surface) {
            DropdownMenuItem(
                text = { Text(stringResource(if (item.pinned) R.string.station_unpin else R.string.station_pin), color = c.ink) },
                onClick = { menu = false; onPin(item.item, !item.pinned) },
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.station_remove), color = c.danger) },
                onClick = { menu = false; onRemove(item.item) },
            )
        }
    }
}

@Composable
private fun TextItemDialog(onConfirm: (String) -> Unit, onDismiss: () -> Unit) {
    val c = LanMsgColors.current
    var text by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = c.surface,
        titleContentColor = c.ink,
        title = { Text(stringResource(R.string.station_add_text)) },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                minLines = 3,
                maxLines = 8,
                colors = LanMsgControls.textField(),
                modifier = Modifier.fillMaxWidth(),
            )
        },
        dismissButton = {
            TextButton(onClick = onDismiss, colors = LanMsgButtons.text()) { Text(stringResource(R.string.cancel)) }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(text) }, enabled = text.isNotBlank(), colors = LanMsgButtons.text()) {
                Text(stringResource(R.string.confirm))
            }
        },
    )
}
