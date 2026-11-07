package name.apk.lanmsg.ui.chat

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Send
import androidx.compose.material.icons.outlined.AttachFile
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import kotlinx.coroutines.flow.filter
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.files.FileOpener
import name.apk.lanmsg.notify.VisibleConversation
import name.apk.lanmsg.ui.common.formatTime
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.MessageView

/** ChatContent 的全部回调。 */
class ChatActions(
    val onBack: () -> Unit = {},
    val onInputChange: (String) -> Unit = {},
    val onSend: () -> Unit = {},
    val onLoadMore: () -> Unit = {},
    val onDismissError: () -> Unit = {},
    val onAttachMedia: () -> Unit = {},
    val onAttachFile: () -> Unit = {},
    val bubble: BubbleActions = BubbleActions(),
)

/** 有状态入口：创建 ViewModel，随生命周期通知可见性。 */
@Composable
fun ChatScreen(
    conv: String,
    onBack: () -> Unit,
    onOpenImage: (conv: String, hash: String) -> Unit,
    onForward: (List<String>) -> Unit,
) {
    val vm: ChatViewModel = viewModel(
        key = "chat-$conv",
        factory = viewModelFactory { initializer { ChatViewModel(ServiceLocator.core, conv, VisibleConversation) } },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    LifecycleStartEffect(vm) {
        vm.onVisible()
        onStopOrDispose { vm.onHidden() }
    }
    val ctx = LocalContext.current
    val mediaPicker = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(MAX_PICK_ITEMS)) { uris ->
        vm.importAndSend(ServiceLocator.importer, uris)
    }
    val filePicker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        vm.importAndSend(ServiceLocator.importer, uris)
    }
    val actions = ChatActions(
        onBack = onBack,
        onInputChange = vm::setInput,
        onSend = vm::sendText,
        onLoadMore = vm::loadMore,
        onDismissError = vm::clearError,
        onAttachMedia = { mediaPicker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) },
        onAttachFile = { filePicker.launch(arrayOf("*/*")) },
        bubble = BubbleActions(
            onResend = vm::resend,
            onDownload = vm::download,
            onCancel = vm::cancel,
            onOpenImage = { onOpenImage(conv, it) },
            onOpenFile = { item -> item.localPath?.let { FileOpener.open(ctx, it, item.mime) } },
            onRecall = vm::recall,
            onDelete = vm::deleteLocal,
            onForward = onForward,
        ),
    )
    ChatContent(state, canRecall = { vm.canRecall(it, System.currentTimeMillis()) }, actions = actions)
}

private const val MAX_PICK_ITEMS = 30

private sealed interface ChatRow {
    data class Separator(val millis: Long) : ChatRow
    data class Message(val msg: MessageView) : ChatRow
}

private const val SEPARATOR_GAP_MS = 5 * 60 * 1000L

/** 首条消息以及与上一条间隔 > 5 分钟的消息前插入时间分隔。 */
private fun buildRows(messages: List<MessageView>): List<ChatRow> {
    val rows = ArrayList<ChatRow>(messages.size + 8)
    var prev: Long? = null
    for (m in messages) {
        if (prev == null || m.hlcMillis - prev > SEPARATOR_GAP_MS) rows += ChatRow.Separator(m.hlcMillis)
        rows += ChatRow.Message(m)
        prev = m.hlcMillis
    }
    return rows
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatContent(
    state: ChatViewModel.UiState,
    canRecall: (MessageView) -> Boolean,
    actions: ChatActions,
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    val errorText = state.error?.let {
        if (it == ChatViewModel.ERROR_PEER_OFFLINE) stringResource(R.string.err_peer_offline) else stringResource(R.string.err_generic, it)
    }
    LaunchedEffect(errorText) {
        if (errorText != null) {
            snackbar.showSnackbar(errorText)
            actions.onDismissError()
        }
    }

    Scaffold(
        containerColor = c.canvas,
        contentWindowInsets = WindowInsets.safeDrawing,
        topBar = {
            Column {
                TopAppBar(
                    title = {
                        Text(if (state.kind == ConvKind.SELF_DEVICES) stringResource(R.string.conv_self_title) else state.title, maxLines = 1)
                    },
                    navigationIcon = {
                        IconButton(onClick = actions.onBack) {
                            Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.back))
                        }
                    },
                    colors = TopAppBarDefaults.topAppBarColors(
                        containerColor = c.surface,
                        titleContentColor = c.ink,
                        navigationIconContentColor = c.ink,
                    ),
                )
                HorizontalDivider(color = c.line)
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { inner ->
        Column(Modifier.padding(inner).fillMaxSize()) {
            if (state.kind == ConvKind.DM && !state.peerOnline) {
                Text(
                    stringResource(R.string.chat_peer_offline),
                    color = c.inkMuted,
                    fontSize = 12.sp,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(8.dp),
                )
            }
            MessageList(state, canRecall, actions, Modifier.weight(1f))
            HorizontalDivider(color = c.line)
            InputBar(state.input, actions)
        }
    }
}

@Composable
private fun MessageList(
    state: ChatViewModel.UiState,
    canRecall: (MessageView) -> Boolean,
    actions: ChatActions,
    modifier: Modifier,
) {
    val rows = remember(state.messages) { buildRows(state.messages) }
    val listState = rememberLazyListState()
    // 对方有多台设备（或“我的设备”会话）时在气泡上方标注来源设备
    val showDevice = state.kind == ConvKind.SELF_DEVICES ||
        state.messages.filter { !it.isMine }.map { it.authorDevice }.distinct().size > 1

    // 新消息到来时：首次加载、自己发的、或用户已在底部附近则滚到底部
    val lastId = state.messages.lastOrNull()?.id
    val lastMine = state.messages.lastOrNull()?.isMine == true
    LaunchedEffect(lastId) {
        if (rows.isEmpty()) return@LaunchedEffect
        val lastVisible = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: -1
        val nearBottom = lastVisible >= rows.lastIndex - 3
        if (lastVisible < 0 || nearBottom || lastMine) listState.scrollToItem(rows.lastIndex)
    }
    // 滚到顶部时加载更早的消息
    LaunchedEffect(listState) {
        snapshotFlow { listState.firstVisibleItemIndex }.filter { it == 0 }.collect { actions.onLoadMore() }
    }

    LazyColumn(
        state = listState,
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        items(rows.size, key = { i ->
            when (val r = rows[i]) {
                is ChatRow.Separator -> "t:${r.millis}:$i"
                is ChatRow.Message -> "m:${r.msg.id}"
            }
        }) { i ->
            when (val r = rows[i]) {
                is ChatRow.Separator -> Text(
                    formatTime(r.millis),
                    color = LanMsgColors.current.inkSubtle,
                    fontSize = 12.sp,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                )
                is ChatRow.Message -> MessageBubble(
                    msg = r.msg,
                    showDeviceName = showDevice,
                    canRecall = canRecall(r.msg),
                    progress = state.progress,
                    actions = actions.bubble,
                )
            }
        }
    }
}

@Composable
private fun InputBar(input: String, actions: ChatActions) {
    val c = LanMsgColors.current
    Row(
        Modifier.fillMaxWidth().background(c.surface).padding(horizontal = 4.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = actions.onAttachMedia) {
            Icon(Icons.Outlined.Image, contentDescription = stringResource(R.string.chat_attach_media), tint = c.inkMuted)
        }
        IconButton(onClick = actions.onAttachFile) {
            Icon(Icons.Outlined.AttachFile, contentDescription = stringResource(R.string.chat_attach_file), tint = c.inkMuted)
        }
        OutlinedTextField(
            value = input,
            onValueChange = actions.onInputChange,
            placeholder = { Text(stringResource(R.string.chat_input_hint), color = c.inkSubtle) },
            maxLines = 5,
            colors = LanMsgControls.textField(),
            modifier = Modifier.weight(1f),
        )
        IconButton(onClick = actions.onSend, enabled = input.isNotBlank()) {
            Icon(
                Icons.AutoMirrored.Outlined.Send,
                contentDescription = stringResource(R.string.chat_send),
                tint = if (input.isNotBlank()) c.signal else c.inkSubtle,
            )
        }
    }
}
