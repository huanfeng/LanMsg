package name.apk.lanmsg.ui.forward

import name.apk.lanmsg.ui.common.errorTextRes
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
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class ForwardPickViewModel(private val core: CoreApi, private val hashes: List<String>) : ViewModel() {
    data class UiState(
        val items: List<ConversationSummary> = emptyList(),
        /** 转发成功的目标会话。 */
        val done: String? = null,
        val error: Int? = null,
        /** ForwardFiles / ListConversations 未实现时为 true，界面显示 err_core_outdated。 */
        val coreOutdated: Boolean = false,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    private var picking = false

    init {
        viewModelScope.launch {
            try {
                val list = core.call<Response.Conversations>(Request.ListConversations).value
                _state.update { it.copy(items = list.sortedByDescending { s -> s.lastHlcMillis ?: Long.MIN_VALUE }) }
            } catch (e: ApiException) {
                _state.update(fail(e))
            }
        }
    }

    fun pick(conv: String) {
        if (picking || _state.value.done != null) return
        picking = true
        viewModelScope.launch {
            try {
                core.request(Request.ForwardFiles(conv, hashes))
                _state.update { it.copy(done = conv, error = null) }
            } catch (e: ApiException) {
                _state.update(fail(e))
            } finally {
                picking = false
            }
        }
    }

    fun clearError() = _state.update { it.copy(error = null) }

    private fun fail(e: ApiException): (UiState) -> UiState = {
        if (e.isNotImplemented()) it.copy(coreOutdated = true, error = null) else it.copy(error = errorTextRes(e.code))
    }
}
