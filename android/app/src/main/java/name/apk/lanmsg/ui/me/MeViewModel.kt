package name.apk.lanmsg.ui.me

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
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SelfInfo
import uniffi.lanmsg_api.StatusDto

class MeViewModel(private val core: CoreApi) : ViewModel() {
    data class UiState(val self: SelfInfo? = null, val error: String? = null)

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    init {
        viewModelScope.launch { core.lagged.collect { reload() } }
        viewModelScope.launch { reload() }
    }

    fun rename(name: String) {
        val trimmed = name.trim()
        if (trimmed.isEmpty()) return
        viewModelScope.launch { change(Request.SetProfile(trimmed)) }
    }

    fun setStatus(s: StatusDto) {
        viewModelScope.launch { change(Request.SetStatus(s)) }
    }

    private suspend fun change(req: Request) {
        try {
            core.call<Response.Ok>(req)
            reload()
        } catch (e: ApiException) {
            _state.update { it.copy(error = e.message ?: e.code.name) }
        }
    }

    private suspend fun reload() {
        try {
            val self = core.call<Response.SelfInfo>(Request.GetSelf).value
            _state.update { it.copy(self = self, error = null) }
        } catch (e: ApiException) {
            _state.update { it.copy(error = e.message ?: e.code.name) }
        }
    }
}
