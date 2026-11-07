package name.apk.lanmsg.ui.contacts

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
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.RequestAction
import uniffi.lanmsg_api.RequestView
import uniffi.lanmsg_api.Response

class RequestsViewModel(private val core: CoreApi) : ViewModel() {
    data class UiState(val items: List<RequestView> = emptyList(), val error: Int? = null)

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    init {
        viewModelScope.launch { core.events.collect { if (it is Event.IncomingRequest) refresh() } }
        viewModelScope.launch { core.lagged.collect { refresh() } }
        refresh()
    }

    fun refresh() {
        viewModelScope.launch {
            try {
                val list = core.call<Response.Requests>(Request.ListRequests).value
                _state.update { it.copy(items = list, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorTextRes(e.code)) }
            }
        }
    }

    /** 成功后才从列表移除；失败保留并给出错误。 */
    fun respond(request: String, action: RequestAction) {
        viewModelScope.launch {
            try {
                core.call<Response.Ok>(Request.RespondRequest(request, action))
                _state.update { s -> s.copy(items = s.items.filter { it.request != request }, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorTextRes(e.code)) }
            }
        }
    }

    fun clearError() = _state.update { it.copy(error = null) }
}
