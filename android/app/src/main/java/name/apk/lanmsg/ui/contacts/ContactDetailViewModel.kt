package name.apk.lanmsg.ui.contacts

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
import uniffi.lanmsg_api.ContactView
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class ContactDetailViewModel(private val core: CoreApi, private val user: String) : ViewModel() {
    data class UiState(val contact: ContactView? = null, val openConv: String? = null, val error: String? = null)

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    init {
        viewModelScope.launch { core.events.collect { if (it == Event.ContactsChanged) refresh() } }
        viewModelScope.launch { core.lagged.collect { refresh() } }
        refresh()
    }

    fun refresh() {
        viewModelScope.launch {
            try {
                val found = core.call<Response.Contacts>(Request.ListContacts).value.firstOrNull { it.userId == user }
                _state.update { it.copy(contact = found, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.message ?: e.code.name) }
            }
        }
    }

    fun openDm() {
        viewModelScope.launch {
            try {
                val conv = core.call<Response.ConvId>(Request.OpenDm(user)).value
                _state.update { it.copy(openConv = conv, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.message ?: e.code.name) }
            }
        }
    }

    /** 界面已跳转到聊天页后调用，避免返回时重复跳转。 */
    fun openConvHandled() = _state.update { it.copy(openConv = null) }

    fun setAutoAccept(v: Boolean) = setPolicy { it.copy(autoAccept = v) }

    fun setAskAbove(bytes: ULong?) = setPolicy { it.copy(askAboveBytes = bytes) }

    /** SetContactPolicy 总是整体回写两个字段；先乐观更新，失败回滚。 */
    private fun setPolicy(change: (ContactView) -> ContactView) {
        val old = _state.value.contact ?: return
        val new = change(old)
        _state.update { it.copy(contact = new) }
        viewModelScope.launch {
            try {
                core.call<Response.Ok>(Request.SetContactPolicy(user, new.autoAccept, new.askAboveBytes))
            } catch (e: ApiException) {
                _state.update { s ->
                    s.copy(contact = if (s.contact == new) old else s.contact, error = e.message ?: e.code.name)
                }
            }
        }
    }

    companion object {
        private val MB = 1024uL * 1024uL

        /** 「超过此大小仍询问」选项：null = 不限。 */
        val ASK_ABOVE: List<ULong?> = listOf(null, 10uL * MB, 50uL * MB, 100uL * MB, 500uL * MB)
    }
}
