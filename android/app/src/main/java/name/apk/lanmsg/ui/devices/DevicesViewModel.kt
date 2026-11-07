package name.apk.lanmsg.ui.devices

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
import uniffi.lanmsg_api.DeviceView
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class DevicesViewModel(private val core: CoreApi) : ViewModel() {
    data class UiState(
        val devices: List<DeviceView> = emptyList(),
        val confirmRemove: DeviceView? = null,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    init {
        viewModelScope.launch { core.events.collect { if (it == Event.DevicesChanged) load() } }
        viewModelScope.launch { core.lagged.collect { load() } }
        load()
    }

    /** 本机不能被移除。 */
    fun askRemove(d: DeviceView) {
        if (!d.isSelf) _state.update { it.copy(confirmRemove = d) }
    }

    fun dismiss() = _state.update { it.copy(confirmRemove = null) }

    fun confirmRemove() {
        val target = _state.value.confirmRemove ?: return
        _state.update { it.copy(confirmRemove = null) }
        viewModelScope.launch {
            try {
                core.call<Response.Ok>(Request.RemoveDevice(target.deviceId))
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.message ?: e.code.name) }
            }
            load()
        }
    }

    private fun load() {
        viewModelScope.launch {
            try {
                val list = core.call<Response.Devices>(Request.ListMyDevices).value
                _state.update { it.copy(devices = list) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.message ?: e.code.name) }
            }
        }
    }
}
