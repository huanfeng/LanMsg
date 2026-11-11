package name.apk.lanmsg.ui.settings

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.call
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.UpdateCheckView

/**
 * 设置页“检查更新”入口的 ViewModel。手动检查不受 24h 节流（core 只对启动自动检查节流）；
 * Android 不做应用内安装，有新版本时给出“打开发布页”的引导（主控修订：不做静默安装）。
 */
class UpdateViewModel(private val core: CoreApi, private val io: CoroutineDispatcher = Dispatchers.IO) : ViewModel() {
    data class UiState(
        val checking: Boolean = false,
        /** 有新版本时的检查结果；对话框展示后调用 [dialogShown] 复位。 */
        val view: UpdateCheckView? = null,
        /** 已是最新：界面以 snackbar 提示，展示后调用 [shown] 复位。 */
        val upToDate: Boolean = false,
        /** 检查失败：界面以 snackbar 提示，展示后调用 [shown] 复位。 */
        val failed: Boolean = false,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** 手动触发一次检查；进行中重复调用忽略。 */
    fun checkUpdate() {
        if (_state.value.checking) return
        _state.update { it.copy(checking = true, upToDate = false, failed = false) }
        viewModelScope.launch {
            val v = withContext(io) {
                try {
                    core.call<Response.UpdateCheck>(Request.CheckUpdate).value
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    null
                }
            }
            _state.update {
                when {
                    v != null && v.update != null -> it.copy(checking = false, view = v)
                    v != null -> it.copy(checking = false, upToDate = true)
                    else -> it.copy(checking = false, failed = true)
                }
            }
        }
    }

    fun dialogShown() = _state.update { it.copy(view = null) }

    fun shown() = _state.update { it.copy(upToDate = false, failed = false) }
}
