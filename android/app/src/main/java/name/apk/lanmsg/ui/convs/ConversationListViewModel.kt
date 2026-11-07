package name.apk.lanmsg.ui.convs

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
import name.apk.lanmsg.core.MutedStore
import name.apk.lanmsg.core.call
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class ConversationListViewModel(private val core: CoreApi, private val prefs: MutedStore) : ViewModel() {
    data class Row(val summary: ConversationSummary, val muted: Boolean)

    data class UiState(val rows: List<Row> = emptyList(), val loading: Boolean = true, val error: Int? = null)

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    private var summaries: List<ConversationSummary> = emptyList()
    private var muted: Set<String> = emptySet()

    /** 重拉在途期间收到过事件的会话：响应合并时保留事件版本，避免旧快照覆盖。 */
    private val touched = mutableSetOf<String>()
    private var inflight = 0

    init {
        viewModelScope.launch { prefs.mutedConversations.collect { muted = it; publish() } }
        viewModelScope.launch { core.events.collect(::onEvent) }
        viewModelScope.launch { core.lagged.collect { refresh() } }
        refresh()
    }

    fun refresh() {
        viewModelScope.launch {
            inflight++
            try {
                val fetched = core.call<Response.Conversations>(Request.ListConversations).value
                val newer = summaries.filter { it.conv in touched }.associateBy { it.conv }
                summaries = fetched.map { newer[it.conv] ?: it } + newer.values.filter { n -> fetched.none { it.conv == n.conv } }
                _state.update { it.copy(loading = false, error = null) }
                publish()
            } catch (e: ApiException) {
                _state.update { it.copy(loading = false, error = errorTextRes(e.code)) }
            } finally {
                if (--inflight == 0) touched.clear()
            }
        }
    }

    fun toggleMute(conv: String) {
        viewModelScope.launch { prefs.setMuted(conv, conv !in muted) }
    }

    private fun onEvent(e: Event) {
        when (e) {
            is Event.ConversationUpdated -> {
                if (inflight > 0) touched += e.summary.conv
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

    fun clearError() = _state.update { it.copy(error = null) }
}
