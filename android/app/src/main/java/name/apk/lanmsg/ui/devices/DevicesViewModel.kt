package name.apk.lanmsg.ui.devices

import name.apk.lanmsg.ui.common.errorTextRes
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
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
import uniffi.lanmsg_api.PairStateDto
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class DevicesViewModel(private val core: CoreApi) : ViewModel() {
    data class UiState(
        val devices: List<DeviceView> = emptyList(),
        val confirmRemove: DeviceView? = null,
        val error: Int? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    private var loadJob: Job? = null

    init {
        viewModelScope.launch { core.events.collect { if (affectsDevices(it)) load() } }
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
                _state.update { it.copy(error = errorTextRes(e.code)) }
            }
            load()
        }
    }

    /** 设备列表受影响的事件：配对完成时设备行与在线状态可能晚于 DevicesChanged 才就绪，所以一并重拉。 */
    private fun affectsDevices(e: Event): Boolean = when (e) {
        Event.DevicesChanged, is Event.PeerOnline, is Event.PeerOffline -> true
        is Event.PairStatus -> e.state == PairStateDto.Done
        else -> false
    }

    /** 250ms 防抖并取消在途的旧加载，避免连发事件的乱序响应覆盖新结果。 */
    private fun load() {
        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            delay(LOAD_DEBOUNCE_MS)
            try {
                val list = core.call<Response.Devices>(Request.ListMyDevices).value
                _state.update { it.copy(devices = list) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorTextRes(e.code)) }
            }
        }
    }

    companion object {
        private const val LOAD_DEBOUNCE_MS = 250L
    }

    fun clearError() = _state.update { it.copy(error = null) }
}
