package name.apk.lanmsg.ui.convs

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.LocalPrefs
import name.apk.lanmsg.core.call
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class ConversationListViewModel(private val core: CoreApi, private val prefs: LocalPrefs) : ViewModel() {
    data class Row(val summary: ConversationSummary, val muted: Boolean)

    data class UiState(val rows: List<Row> = emptyList(), val loading: Boolean = true, val error: String? = null)

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    private var summaries: List<ConversationSummary> = emptyList()
    private var muted: Set<String> = emptySet()

    init {
        viewModelScope.launch { prefs.mutedConversations.collect { muted = it; publish() } }
        viewModelScope.launch { core.events.collect(::onEvent) }
        viewModelScope.launch { core.lagged.collect { refresh() } }
        refresh()
    }

    fun refresh() {
        viewModelScope.launch {
            try {
                summaries = core.call<Response.Conversations>(Request.ListConversations).value
                _state.update { it.copy(loading = false, error = null) }
                publish()
            } catch (e: ApiException) {
                _state.update { it.copy(loading = false, error = e.message ?: e.code.name) }
            }
        }
    }

    fun toggleMute(conv: String) {
        viewModelScope.launch { prefs.setMuted(conv, conv !in muted) }
    }

    private fun onEvent(e: Event) {
        when (e) {
            is Event.ConversationUpdated -> {
                summaries = summaries.filter { it.conv != e.summary.conv } + e.summary
                publish()
            }
            is Event.PeerOnline, is Event.PeerOffline -> refresh()
            else -> Unit
        }
    }

    /** 按最后消息时间降序，没有消息的排最后。 */
    private fun publish() {
        val rows = summaries
            .sortedByDescending { it.lastHlcMillis ?: Long.MIN_VALUE }
            .map { Row(it, it.conv in muted) }
        _state.update { it.copy(rows = rows) }
    }
}
