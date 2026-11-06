package name.apk.lanmsg.ui.onboarding

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
import uniffi.lanmsg_api.InitMode
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class OnboardingViewModel(private val core: CoreApi, private val defaultDeviceName: String) : ViewModel() {
    enum class Step { WELCOME, NAME, MODE }
    enum class Destination { HOME, PAIR_JOIN }

    data class UiState(
        val step: Step = Step.WELCOME,
        val displayName: String = "",
        val deviceName: String,
        val busy: Boolean = false,
        /** 名字为空时为 [ERROR_NAME_REQUIRED]（界面显示 onb_name_required），其余为 core 的错误消息。 */
        val error: String? = null,
        val done: Destination? = null,
    )

    private val _state = MutableStateFlow(UiState(deviceName = defaultDeviceName))
    val state: StateFlow<UiState> = _state.asStateFlow()

    fun setDisplayName(v: String) = _state.update { it.copy(displayName = v, error = null) }

    fun setDeviceName(v: String) = _state.update { it.copy(deviceName = v) }

    fun next() {
        val s = _state.value
        when (s.step) {
            Step.WELCOME -> _state.update { it.copy(step = Step.NAME, error = null) }
            Step.NAME ->
                if (s.displayName.isBlank()) {
                    _state.update { it.copy(error = ERROR_NAME_REQUIRED) }
                } else {
                    _state.update { it.copy(step = Step.MODE, error = null) }
                }
            Step.MODE -> Unit
        }
    }

    fun chooseFirstDevice() = init(InitMode.FIRST_DEVICE, Destination.HOME)

    fun chooseJoinExisting() = init(InitMode.JOIN_EXISTING, Destination.PAIR_JOIN)

    private fun init(mode: InitMode, destination: Destination) {
        val s = _state.value
        if (s.busy) return
        _state.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                val device = s.deviceName.trim().ifEmpty { defaultDeviceName }
                core.call<Response.Ok>(Request.Init(s.displayName.trim(), device, mode))
                _state.update { it.copy(busy = false, done = destination) }
            } catch (e: ApiException) {
                _state.update { it.copy(busy = false, error = e.message ?: e.code.name) }
            }
        }
    }

    companion object {
        const val ERROR_NAME_REQUIRED = "name_required"
    }
}
