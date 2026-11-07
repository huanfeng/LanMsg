package name.apk.lanmsg.ui.chat

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.BrokenImage
import androidx.compose.material.icons.outlined.SaveAlt
import androidx.compose.material3.Button
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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import coil.compose.AsyncImage
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.call
import name.apk.lanmsg.notify.VisibleConversation
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.FileItemView
import uniffi.lanmsg_api.MessageBodyView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import java.io.File

/** 图片查看页：从会话最近的消息里按 hash 找到文件条目；未下载时显示缩略图和「下载原图」。 */
class ImageViewerViewModel(private val core: CoreApi, private val conv: String, private val hash: String) : ViewModel() {
    data class UiState(
        val item: FileItemView? = null,
        val progress: Pair<ULong, ULong>? = null,
        val loading: Boolean = true,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    init {
        viewModelScope.launch {
            core.events.collect { e ->
                when (e) {
                    is Event.TransferFinished -> if (e.hash == hash) {
                        _state.update { it.copy(progress = null) }
                        load()
                    }
                    is Event.TransferProgress -> if (e.hash == hash) _state.update { it.copy(progress = e.done to e.total) }
                    is Event.MessageUpdated -> if (e.msg.conv == conv) findItem(e.msg.body)?.let { found -> _state.update { it.copy(item = found) } }
                    else -> Unit
                }
            }
        }
        viewModelScope.launch { core.lagged.collect { load() } }
        load()
    }

    fun download() {
        viewModelScope.launch {
            try {
                core.request(Request.DownloadBlob(hash))
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.message ?: e.code.name) }
            }
        }
    }

    fun clearError() = _state.update { it.copy(error = null) }

    private fun load() {
        viewModelScope.launch {
            try {
                val msgs = core.call<Response.Messages>(Request.ListMessages(conv, null, LOOKUP_LIMIT)).value
                val found = msgs.firstNotNullOfOrNull { findItem(it.body) }
                _state.update { it.copy(item = found ?: it.item, loading = false, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(loading = false, error = e.message ?: e.code.name) }
            }
        }
    }

    private fun findItem(body: MessageBodyView): FileItemView? = when (body) {
        is MessageBodyView.Files -> body.items.firstOrNull { it.hash == hash }
        is MessageBodyView.Folder -> body.items.firstOrNull { it.hash == hash }
        else -> null
    }

    private companion object {
        const val LOOKUP_LIMIT = 200u
    }
}

@Composable
fun ImageViewerScreen(conv: String, hash: String, onBack: () -> Unit) {
    val vm: ImageViewerViewModel = viewModel(
        key = "image-$conv-$hash",
        factory = viewModelFactory { initializer { ImageViewerViewModel(ServiceLocator.coreHolder.api, conv, hash) } },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    // 看图期间聊天页已被移出组合并清掉“正在看的会话”，这里补登记，避免本会话来消息时弹通知
    DisposableEffect(conv) {
        VisibleConversation.pin(conv)
        onDispose { VisibleConversation.unpin(conv) }
    }
    val scope = rememberCoroutineScope()
    var saveResult by remember { mutableStateOf<Int?>(null) }
    ImageViewerContent(
        state,
        onBack,
        vm::download,
        vm::clearError,
        onSaveToGallery = {
            val item = state.item
            val local = item?.localPath
            if (item != null && local != null) {
                scope.launch {
                    val uri = ServiceLocator.mediaSaver.saveToGallery(local, item.mime, item.name)
                    saveResult = if (uri != null) R.string.file_saved else R.string.file_save_failed
                }
            }
        },
        message = saveResult?.let { stringResource(it) },
        onMessageShown = { saveResult = null },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ImageViewerContent(
    state: ImageViewerViewModel.UiState,
    onBack: () -> Unit,
    onDownload: () -> Unit,
    onDismissError: () -> Unit,
    onSaveToGallery: () -> Unit = {},
    /** 一次性提示（保存到相册的结果），显示后调用 [onMessageShown]。 */
    message: String? = null,
    onMessageShown: () -> Unit = {},
) {
    val c = LanMsgColors.current
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(message) {
        if (message != null) {
            snackbar.showSnackbar(message)
            onMessageShown()
        }
    }
    val errorText = state.error?.let { stringResource(R.string.err_generic, it) }
    LaunchedEffect(errorText) {
        if (errorText != null) {
            snackbar.showSnackbar(errorText)
            onDismissError()
        }
    }
    val item = state.item
    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(item?.name.orEmpty(), maxLines = 1) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.back))
                    }
                },
                actions = {
                    if (item != null && item.state == BlobState.LOCAL && item.localPath != null) {
                        IconButton(onClick = onSaveToGallery) {
                            Icon(Icons.Outlined.SaveAlt, contentDescription = stringResource(R.string.file_save_to_gallery))
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
        snackbarHost = { SnackbarHost(snackbar) },
    ) { inner ->
        Box(Modifier.padding(inner).fillMaxSize()) {
            if (item == null) {
                if (state.loading) {
                    CircularProgressIndicator(color = LanMsgControls.progress, modifier = Modifier.align(Alignment.Center))
                } else {
                    EmptyState(stringResource(R.string.image_not_found), Icons.Outlined.BrokenImage)
                }
                return@Box
            }
            val local = item.localPath
            val image = if (item.state == BlobState.LOCAL && local != null) local else item.thumbPath
            Column(Modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally) {
                if (image != null) {
                    AsyncImage(
                        model = File(image),
                        contentDescription = item.name,
                        contentScale = ContentScale.Fit,
                        modifier = Modifier.weight(1f).fillMaxWidth(),
                    )
                } else {
                    Spacer(Modifier.weight(1f))
                }
                if (item.state != BlobState.LOCAL) {
                    Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) {
                        if (item.state == BlobState.DOWNLOADING) {
                            val p = state.progress?.takeIf { it.second > 0uL }
                            if (p != null) {
                                CircularProgressIndicator(progress = { (p.first.toDouble() / p.second.toDouble()).toFloat().coerceIn(0f, 1f) }, color = LanMsgControls.progress, modifier = Modifier.size(40.dp))
                            } else {
                                CircularProgressIndicator(color = LanMsgControls.progress, modifier = Modifier.size(40.dp))
                            }
                        } else {
                            Button(onClick = onDownload, enabled = item.state == BlobState.REMOTE) {
                                Text(stringResource(R.string.file_download_original))
                            }
                        }
                    }
                    if (item.state == BlobState.SOURCE_OFFLINE) {
                        Text(stringResource(R.string.file_source_offline), color = c.inkMuted, modifier = Modifier.padding(bottom = 16.dp))
                    }
                }
            }
        }
    }
}
