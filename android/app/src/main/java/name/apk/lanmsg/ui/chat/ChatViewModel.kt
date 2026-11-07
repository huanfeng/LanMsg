package name.apk.lanmsg.ui.chat

import android.net.Uri
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
import name.apk.lanmsg.files.Importer
import name.apk.lanmsg.notify.VisibleConversation
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.MessageBodyView
import uniffi.lanmsg_api.MessageState
import uniffi.lanmsg_api.MessageView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class ChatViewModel(
    private val core: CoreApi,
    private val conv: String,
    private val visible: VisibleConversation,
) : ViewModel() {
    data class UiState(
        val title: String = "",
        val kind: ConvKind? = null,
        val peerOnline: Boolean = false,
        /** 升序（最旧在前）。 */
        val messages: List<MessageView> = emptyList(),
        val hasMore: Boolean = true,
        val loadingMore: Boolean = false,
        val input: String = "",
        /** hash -> (已完成, 总量)。 */
        val progress: Map<String, Pair<ULong, ULong>> = emptyMap(),
        /** 对方不在线时为 [ERROR_PEER_OFFLINE]（界面显示 err_peer_offline），其余为 core 的错误消息。 */
        val error: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** hash -> transfer id，CancelTransfer 用。 */
    private val transfers = mutableMapOf<String, String>()
    private var isVisible = false
    private var hiddenBefore = false
    private var lastMarked: String? = null

    /** 重拉在途期间事件触达过的消息 id：响应合并时跳过，避免旧快照覆盖较新的事件版本。 */
    private val touched = mutableSetOf<String>()

    /** 本地删除过的消息 id：重拉在途期间合并时过滤，避免旧响应让已删消息复活；无在途重拉时清空。 */
    private val deleted = mutableSetOf<String>()
    private var inflight = 0

    init {
        viewModelScope.launch { core.events.collect(::onEvent) }
        viewModelScope.launch { core.lagged.collect { reloadAll() } }
        reloadAll()
    }

    fun setInput(v: String) = _state.update { it.copy(input = v) }

    fun clearError() = _state.update { it.copy(error = null) }

    fun sendText() {
        val text = _state.value.input
        if (text.isBlank()) return
        _state.update { it.copy(input = "") }
        send(text) { if (it.input.isEmpty()) it.copy(input = text) else it }
    }

    fun resend(msg: MessageView) {
        val body = msg.body
        if (body is MessageBodyView.Text) send(body.text) { it }
    }

    private fun send(text: String, onFail: (UiState) -> UiState) {
        viewModelScope.launch {
            guarded(onFail) { core.request(Request.SendText(conv, text)) }
        }
    }

    fun sendFiles(paths: List<String>) {
        if (paths.isEmpty()) return
        viewModelScope.launch { guarded { core.request(Request.SendFiles(conv, paths)) } }
    }

    /** 把外部 URI 落地到发件箱后发送；落地在 IO 线程，全部失败时不发请求。 */
    fun importAndSend(importer: Importer, uris: List<Uri>) {
        if (uris.isEmpty()) return
        viewModelScope.launch { sendFiles(importer.import(uris)) }
    }

    fun download(hash: String) {
        viewModelScope.launch {
            guarded {
                val r = core.request(Request.DownloadBlob(hash))
                if (r is Response.TransferId) transfers[hash] = r.value
            }
        }
    }

    fun cancel(hash: String) {
        val transfer = transfers[hash] ?: return
        viewModelScope.launch { guarded { core.request(Request.CancelTransfer(transfer)) } }
    }

    fun recall(msgId: String) {
        viewModelScope.launch { guarded { core.request(Request.Recall(msgId)) } }
    }

    fun deleteLocal(msgId: String) {
        if (inflight > 0) deleted += msgId
        viewModelScope.launch {
            try {
                core.request(Request.DeleteLocal(listOf(msgId), false))
                _state.update { s -> s.copy(messages = s.messages.filter { it.id != msgId }) }
            } catch (e: ApiException) {
                deleted -= msgId
                _state.update { it.copy(error = errorText(e)) }
            }
        }
    }

    fun loadMore() {
        val s = _state.value
        val oldest = s.messages.firstOrNull() ?: return
        if (s.loadingMore || !s.hasMore) return
        _state.update { it.copy(loadingMore = true) }
        viewModelScope.launch {
            try {
                val older = core.call<Response.Messages>(Request.ListMessages(conv, oldest.id, PAGE)).value
                _state.update { cur ->
                    val known = cur.messages.mapTo(HashSet()) { it.id }
                    cur.copy(
                        messages = older.filter { it.id !in known } + cur.messages,
                        hasMore = older.isNotEmpty(),
                    )
                }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorText(e)) }
            } finally {
                _state.update { it.copy(loadingMore = false) }
            }
        }
    }

    fun onVisible() {
        isVisible = true
        visible.current.value = conv
        // 从后台回来：全量刷新（首次显示时初始加载已在进行）
        if (hiddenBefore) reloadAll()
        markReadIfNeeded()
    }

    fun onHidden() {
        isVisible = false
        hiddenBefore = true
        if (visible.current.value == conv) visible.current.value = null
    }

    fun canRecall(msg: MessageView, nowMillis: Long): Boolean =
        msg.isMine && nowMillis - msg.hlcMillis <= RECALL_WINDOW_MS && msg.state != MessageState.RECALLED

    /** 全量重拉：摘要 + 最新一页，按 id 合并进已有列表，不产生重复。 */
    private fun reloadAll() {
        loadSummary()
        loadLatest()
    }

    private fun loadSummary() {
        viewModelScope.launch {
            guarded {
                val list = core.call<Response.Conversations>(Request.ListConversations).value
                list.firstOrNull { it.conv == conv }?.let(::applySummary)
            }
        }
    }

    private fun loadLatest() {
        viewModelScope.launch {
            inflight++
            try {
                guarded {
                    val latest = core.call<Response.Messages>(Request.ListMessages(conv, null, PAGE)).value
                    _state.update { cur ->
                        val have = cur.messages.mapTo(HashSet()) { it.id }
                        // 事件已写入列表的版本更新，不被旧响应覆盖
                        cur.copy(messages = latest.filter { it.id !in deleted && (it.id !in touched || it.id !in have) }.fold(cur.messages) { acc, m -> upsert(acc, m) })
                    }
                    markReadIfNeeded()
                }
            } finally {
                if (--inflight == 0) {
                    touched.clear()
                    deleted.clear()
                }
            }
        }
    }

    private fun applySummary(s: ConversationSummary) =
        _state.update { it.copy(title = s.title, kind = s.kind, peerOnline = s.peerOnline) }

    private fun onEvent(e: Event) {
        when (e) {
            is Event.MessageAdded -> if (e.msg.conv == conv) {
                if (inflight > 0) touched += e.msg.id
                _state.update { it.copy(messages = upsert(it.messages, e.msg)) }
                markReadIfNeeded()
            }
            is Event.MessageUpdated -> if (e.msg.conv == conv) {
                if (inflight > 0) touched += e.msg.id
                _state.update { s ->
                    if (s.messages.any { it.id == e.msg.id }) s.copy(messages = upsert(s.messages, e.msg)) else s
                }
            }
            is Event.ConversationUpdated -> if (e.summary.conv == conv) applySummary(e.summary)
            is Event.PeerOnline, is Event.PeerOffline -> loadSummary()
            is Event.TransferProgress -> {
                transfers[e.hash] = e.transfer
                _state.update { it.copy(progress = it.progress + (e.hash to (e.done to e.total))) }
            }
            is Event.TransferFinished -> {
                transfers.remove(e.hash)
                _state.update { it.copy(progress = it.progress - e.hash) }
                loadLatest()
            }
            else -> Unit
        }
    }

    /** 页面可见且末条消息 id 变化时发 MarkRead，同一个 id 只发一次。 */
    private fun markReadIfNeeded() {
        if (!isVisible) return
        val last = _state.value.messages.lastOrNull()?.id ?: return
        if (last == lastMarked) return
        lastMarked = last
        viewModelScope.launch {
            try {
                core.request(Request.MarkRead(conv, last))
            } catch (e: ApiException) {
                lastMarked = null
                _state.update { it.copy(error = errorText(e)) }
            }
        }
    }

    private suspend fun guarded(onFail: (UiState) -> UiState = { it }, block: suspend () -> Unit) {
        try {
            block()
        } catch (e: ApiException) {
            _state.update { onFail(it).copy(error = errorText(e)) }
        }
    }

    private fun errorText(e: ApiException): String =
        if (e.code == ErrorCode.PEER_OFFLINE) ERROR_PEER_OFFLINE else e.message ?: e.code.name

    companion object {
        const val ERROR_PEER_OFFLINE = "peer_offline"
        private const val PAGE = 50u
        private const val RECALL_WINDOW_MS = 120_000L

        /** 按 id 去重：已有则原位替换，否则按 hlc 插入到正确位置（hlc 相同保持到达顺序）。 */
        internal fun upsert(list: List<MessageView>, msg: MessageView): List<MessageView> {
            val idx = list.indexOfFirst { it.id == msg.id }
            if (idx >= 0) return list.toMutableList().also { it[idx] = msg }
            var pos = list.size
            while (pos > 0 && list[pos - 1].hlcMillis > msg.hlcMillis) pos--
            return list.toMutableList().also { it.add(pos, msg) }
        }
    }
}
