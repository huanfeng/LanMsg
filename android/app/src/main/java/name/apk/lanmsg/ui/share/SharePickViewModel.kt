package name.apk.lanmsg.ui.share

import name.apk.lanmsg.R
import name.apk.lanmsg.ui.common.errorTextRes
import android.net.Uri
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.call
import name.apk.lanmsg.files.Importer
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

/** 分享入口：Activity 级 ViewModel，MainActivity 收到分享 intent 后 [setPending]，选择页据此发送。 */
class SharePickViewModel(private val core: CoreApi, private val importer: Importer) : ViewModel() {
    data class UiState(
        val items: List<ConversationSummary> = emptyList(),
        val pendingUris: List<Uri> = emptyList(),
        val pendingText: String? = null,
        val busy: Boolean = false,
        /** 发送成功的会话 id，界面据此跳转聊天页。 */
        val sentTo: String? = null,
        val error: Int? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    fun setPending(uris: List<Uri>, text: String?) {
        _state.update { UiState(pendingUris = uris, pendingText = text) }
        loadConversations()
    }

    fun refresh() = loadConversations()

    private fun loadConversations() {
        viewModelScope.launch {
            try {
                val list = core.call<Response.Conversations>(Request.ListConversations).value
                _state.update { it.copy(items = list.sortedByDescending { c -> c.lastHlcMillis ?: Long.MIN_VALUE }) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorTextRes(e.code)) }
            }
        }
    }

    fun pick(conv: String) {
        val s = _state.value
        if (s.busy) return
        _state.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                if (s.pendingUris.isNotEmpty()) {
                    val paths = importer.import(s.pendingUris)
                    if (paths.isEmpty()) {
                        _state.update { it.copy(busy = false, error = R.string.share_import_failed) }
                        return@launch
                    }
                    core.request(Request.SendFiles(conv, paths))
                } else if (s.pendingText != null) {
                    core.request(Request.SendText(conv, s.pendingText))
                }
                _state.update { it.copy(busy = false, sentTo = conv) }
            } catch (e: ApiException) {
                _state.update { it.copy(busy = false, error = errorTextRes(e.code)) }
            }
        }
    }

    fun clearError() = _state.update { it.copy(error = null) }

    /** 跳转聊天页后调用，清掉待发送内容，避免重复发送。 */
    fun reset() = _state.update { UiState(items = it.items) }
}
