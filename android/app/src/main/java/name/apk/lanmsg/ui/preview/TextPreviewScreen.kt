package name.apk.lanmsg.ui.preview

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.BrokenImage
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import name.apk.lanmsg.R
import name.apk.lanmsg.files.FileOpener
import name.apk.lanmsg.files.FilePreview
import name.apk.lanmsg.ui.common.EmptyState
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import java.io.File
import java.io.IOException

/** 文本预览：读取与按行拆分都在 IO 线程完成，界面只拿到现成的行列表。 */
class TextPreviewViewModel(private val path: String) : ViewModel() {
    sealed interface UiState {
        data object Loading : UiState
        data class Ready(val lines: List<String>, val truncated: Boolean) : UiState

        /** 文件头里有 NUL：不预览，交给系统程序。 */
        data object Binary : UiState
        data object Failed : UiState
    }

    private val _state = MutableStateFlow<UiState>(UiState.Loading)
    val state: StateFlow<UiState> = _state.asStateFlow()

    init {
        viewModelScope.launch {
            _state.value = withContext(Dispatchers.IO) {
                try {
                    val t = FilePreview.readText(File(path))
                    if (t == null) UiState.Binary else UiState.Ready(splitLines(t.text), t.truncated)
                } catch (_: IOException) {
                    UiState.Failed
                }
            }
        }
    }

    companion object {
        /** 按 \n 拆行并去掉行尾的 \r，空行保留。 */
        fun splitLines(text: String): List<String> = text.split('\n').map { it.removeSuffix("\r") }
    }
}

@Composable
fun TextPreviewScreen(path: String, name: String, mime: String, onBack: () -> Unit) {
    val vm: TextPreviewViewModel = viewModel(
        key = "text-$path",
        factory = viewModelFactory { initializer { TextPreviewViewModel(path) } },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    val ctx = LocalContext.current
    val openWith = { FileOpener.open(ctx, path, mime) }
    // 二进制内容不预览：回退到系统程序打开，并退出预览页
    LaunchedEffect(state) {
        if (state == TextPreviewViewModel.UiState.Binary) {
            openWith()
            onBack()
        }
    }
    TextPreviewContent(state, name, onBack, onOpenWith = openWith)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TextPreviewContent(
    state: TextPreviewViewModel.UiState,
    name: String,
    onBack: () -> Unit,
    onOpenWith: () -> Unit,
) {
    val c = LanMsgColors.current
    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(name, maxLines = 1) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.back))
                    }
                },
                actions = {
                    IconButton(onClick = onOpenWith) {
                        Icon(Icons.AutoMirrored.Outlined.OpenInNew, contentDescription = stringResource(R.string.preview_open_with))
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
        bottomBar = {
            if (state is TextPreviewViewModel.UiState.Ready && state.truncated) {
                Text(
                    stringResource(R.string.preview_truncated),
                    color = c.inkSubtle,
                    fontSize = 12.sp,
                    modifier = Modifier.fillMaxWidth().background(c.surface).navigationBarsPadding().padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }
        },
    ) { inner ->
        Box(Modifier.padding(inner).fillMaxSize()) {
            when (state) {
                is TextPreviewViewModel.UiState.Loading, is TextPreviewViewModel.UiState.Binary ->
                    CircularProgressIndicator(color = LanMsgControls.progress, modifier = Modifier.align(Alignment.Center))
                is TextPreviewViewModel.UiState.Failed ->
                    EmptyState(stringResource(R.string.preview_failed), Icons.Outlined.BrokenImage)
                is TextPreviewViewModel.UiState.Ready ->
                    // 可选中复制；按行懒加载，1 MiB 文本也不会一次性排版
                    SelectionContainer(Modifier.fillMaxSize()) {
                        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp)) {
                            items(state.lines) { line ->
                                Text(
                                    line.ifEmpty { " " },
                                    color = c.ink,
                                    fontFamily = FontFamily.Monospace,
                                    fontSize = 13.sp,
                                    lineHeight = 18.sp,
                                )
                            }
                        }
                    }
            }
        }
    }
}
