package name.apk.lanmsg.ui.contacts

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import name.apk.lanmsg.R
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.call
import uniffi.lanmsg_api.ContactView
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.PeerView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class ContactsViewModel(private val core: CoreApi) : ViewModel() {
    data class UiState(
        val contacts: List<ContactView> = emptyList(),
        /** 附近的人：排除联系人与自己的设备，同一用户多台设备只留一行。 */
        val nearby: List<PeerView> = emptyList(),
        val requestCount: Int = 0,
        /** 字符串资源 id，界面显示后调用 [toastShown]。 */
        val toast: Int? = null,
        /** 对方不在线时为 [ERROR_PEER_OFFLINE]（界面显示 err_peer_offline），其余为 core 的错误消息。 */
        val error: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    init {
        viewModelScope.launch { core.events.collect(::onEvent) }
        viewModelScope.launch { core.lagged.collect { refresh() } }
        refresh()
    }

    fun refresh() {
        loadContacts()
        loadPeers()
        loadRequestCount()
    }

    fun add(user: String) {
        viewModelScope.launch {
            try {
                core.call<Response.Ok>(Request.RequestContact(user))
                _state.update { it.copy(toast = R.string.toast_request_sent, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorText(e)) }
            }
        }
    }

    fun toastShown() = _state.update { it.copy(toast = null) }

    private fun onEvent(e: Event) {
        when (e) {
            Event.ContactsChanged -> { loadContacts(); loadPeers() } // 成为联系人后附近列表也要去掉
            is Event.PeerOnline, is Event.PeerOffline -> loadPeers()
            is Event.IncomingRequest -> loadRequestCount()
            else -> Unit
        }
    }

    private fun loadContacts() {
        viewModelScope.launch {
            try {
                val list = core.call<Response.Contacts>(Request.ListContacts).value
                _state.update { it.copy(contacts = list, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorText(e)) }
            }
        }
    }

    private fun loadPeers() {
        viewModelScope.launch {
            try {
                val peers = core.call<Response.Peers>(Request.ListPeers).value
                val nearby = peers.filter { !it.isContact && !it.isMine }.distinctBy { it.userId }
                _state.update { it.copy(nearby = nearby, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorText(e)) }
            }
        }
    }

    private fun loadRequestCount() {
        viewModelScope.launch {
            try {
                val n = core.call<Response.Requests>(Request.ListRequests).value.size
                _state.update { it.copy(requestCount = n, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorText(e)) }
            }
        }
    }

    private fun errorText(e: ApiException) =
        if (e.code == ErrorCode.PEER_OFFLINE) ERROR_PEER_OFFLINE else e.message ?: e.code.name

    companion object {
        const val ERROR_PEER_OFFLINE = "peer_offline"
    }
}
