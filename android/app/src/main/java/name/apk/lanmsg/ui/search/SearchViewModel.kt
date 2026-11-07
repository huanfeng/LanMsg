package name.apk.lanmsg.ui.search

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
import uniffi.lanmsg_api.MessageView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class SearchViewModel(private val core: CoreApi) : ViewModel() {
    data class Row(val msg: MessageView, val convTitle: String)

    data class UiState(
        val query: String = "",
        val rows: List<Row> = emptyList(),
        val loading: Boolean = false,
        val error: String? = null,
        /** SearchMessages 未实现时为 true，界面显示 err_core_outdated。 */
        val coreOutdated: Boolean = false,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** conv -> 标题缓存，首次搜索前拉取一次。 */
    private var titles: Map<String, String>? = null
    private var job: Job? = null

    /** 300ms 防抖；trim 后为空 → 清空结果，不发请求。 */
    fun setQuery(q: String) {
        job?.cancel()
        _state.update { it.copy(query = q) }
        val text = q.trim()
        if (text.isEmpty()) {
            _state.update { it.copy(rows = emptyList(), loading = false, error = null) }
            return
        }
        _state.update { it.copy(loading = true) }
        job = viewModelScope.launch {
            delay(DEBOUNCE_MS)
            try {
                val map = titles ?: core.call<Response.Conversations>(Request.ListConversations).value
                    .associate { it.conv to it.title }.also { titles = it }
                val found = core.call<Response.Messages>(Request.SearchMessages(text, null, LIMIT)).value
                val rows = found.map { Row(it, map[it.conv].orEmpty()) }
                _state.update { it.copy(rows = rows, loading = false, error = null, coreOutdated = false) }
            } catch (e: ApiException) {
                if (e.isNotImplemented()) {
                    _state.update { it.copy(rows = emptyList(), loading = false, error = null, coreOutdated = true) }
                } else {
                    _state.update { it.copy(loading = false, error = e.message ?: e.code.name) }
                }
            }
        }
    }

    companion object {
        private const val DEBOUNCE_MS = 300L
        private val LIMIT = 50u
    }
}
