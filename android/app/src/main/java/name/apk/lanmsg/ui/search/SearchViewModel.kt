package name.apk.lanmsg.ui.search

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
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.MessageView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class SearchViewModel(private val core: CoreApi) : ViewModel() {
    /** [selfDevices] 为“我的设备”会话：标题由界面换成本地化文案（ViewModel 拿不到 Context）。 */
    data class Row(val msg: MessageView, val convTitle: String, val selfDevices: Boolean = false)

    data class UiState(
        val query: String = "",
        val rows: List<Row> = emptyList(),
        val loading: Boolean = false,
        val error: Int? = null,
        /** SearchMessages 未实现时为 true，界面显示 err_core_outdated。 */
        val coreOutdated: Boolean = false,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** conv -> 会话摘要缓存：首次搜索前拉取，ConversationUpdated 时更新，结果里出现缓存没有的会话时重拉。 */
    private var convs: MutableMap<String, ConversationSummary>? = null
    private var job: Job? = null

    /** 重拉后仍不在会话列表里的 conv（如已删除的会话）：视为已确认缺失，不再每次搜索都重拉。 */
    private val missing = mutableSetOf<String>()

    init {
        viewModelScope.launch {
            core.events.collect { e ->
                if (e is Event.ConversationUpdated) {
                    missing -= e.summary.conv
                    convs?.put(e.summary.conv, e.summary)
                }
            }
        }
    }

    private suspend fun loadConvs(): MutableMap<String, ConversationSummary> =
        core.call<Response.Conversations>(Request.ListConversations).value
            .associateByTo(mutableMapOf()) { it.conv }.also { convs = it }

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
                val found = core.call<Response.Messages>(Request.SearchMessages(text, null, LIMIT)).value
                var map = convs ?: loadConvs()
                if (found.any { it.conv !in map && it.conv !in missing }) {
                    map = loadConvs()
                    missing += found.map { it.conv }.filter { it !in map }
                }
                val rows = found.map { m ->
                    val summary = map[m.conv]
                    Row(m, summary?.title.orEmpty(), summary?.kind == ConvKind.SELF_DEVICES)
                }
                _state.update { it.copy(rows = rows, loading = false, error = null, coreOutdated = false) }
            } catch (e: ApiException) {
                if (e.isNotImplemented()) {
                    _state.update { it.copy(rows = emptyList(), loading = false, error = null, coreOutdated = true) }
                } else {
                    _state.update { it.copy(loading = false, error = errorTextRes(e.code)) }
                }
            }
        }
    }

    companion object {
        private const val DEBOUNCE_MS = 300L
        private val LIMIT = 50u
    }
}
