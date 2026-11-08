package name.apk.lanmsg.ui.web

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.CoroutineScope
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
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.WebSessionInfo

/**
 * 网页传输页。托管期间通过 [setWebHosting] 通知服务保持前台；
 * 离开页面（ViewModel 销毁）时结束会话，避免没有界面可管理的会话与常驻通知遗留。
 */
class WebHostViewModel(
    private val core: CoreApi,
    private val setWebHosting: (Boolean) -> Unit,
    private val clock: () -> Long = System::currentTimeMillis,
    /** ViewModel 销毁后仍需完成 WebStop，所以用独立于 viewModelScope 的作用域。 */
    private val detachedScope: CoroutineScope,
) : ViewModel() {
    data class PendingRequest(val request: String, val userAgent: String, val ip: String, val receivedAt: Long)

    data class UiState(
        val info: WebSessionInfo? = null,
        val urlIndex: Int = 0,
        val guests: List<ConversationSummary> = emptyList(),
        val pending: PendingRequest? = null,
        /** 最近一次失败的错误码，界面用 errorTextRes 本地化；消费后 [clearError]。 */
        val error: ErrorCode? = null,
        /** 会话已结束（用户结束、空闲超时或启动失败），界面应返回上一页。 */
        val ended: Boolean = false,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** 待处理请求队列（按到达顺序），界面一次只显示第一条。 */
    private val queue = mutableListOf<PendingRequest>()
    private val timeouts = mutableMapOf<String, Job>()
    private var starting = false

    init {
        viewModelScope.launch { core.events.collect(::onEvent) }
        viewModelScope.launch { core.lagged.collect { loadGuests() } }
    }

    fun start() {
        if (starting || _state.value.info != null) return
        starting = true
        setWebHosting(true)
        viewModelScope.launch {
            try {
                val info = core.call<Response.WebSession>(Request.WebStart).value
                _state.update { it.copy(info = info, urlIndex = 0) }
                loadGuests()
            } catch (e: ApiException) {
                Log.w(TAG, "Failed to start web hosting: ${e.code}", e)
                setWebHosting(false)
                _state.update { it.copy(error = e.code, ended = true) }
            } finally {
                starting = false
            }
        }
    }

    fun stop() {
        finishLocal()
        // 页面随即返回，viewModelScope 会被取消，WebStop 必须在独立作用域里完成
        detachedScope.launch { requestStop() }
    }

    fun respond(allow: Boolean) {
        val req = queue.firstOrNull() ?: return
        respondTo(req.request, allow)
    }

    fun selectUrl(i: Int) {
        val n = _state.value.info?.urls?.size ?: return
        if (i in 0 until n) _state.update { it.copy(urlIndex = i) }
    }

    fun clearError() = _state.update { it.copy(error = null) }

    private fun respondTo(request: String, allow: Boolean) {
        val idx = queue.indexOfFirst { it.request == request }
        if (idx < 0) return
        queue.removeAt(idx)
        timeouts.remove(request)?.cancel()
        publishPending()
        viewModelScope.launch {
            try {
                core.call<Response.Ok>(Request.WebRespond(request, allow))
            } catch (e: ApiException) {
                // 请求可能已过期或会话已结束，用户无需处理
                Log.w(TAG, "WebRespond failed: ${e.code}", e)
            }
        }
    }

    private fun publishPending() = _state.update { it.copy(pending = queue.firstOrNull()) }

    private suspend fun loadGuests() {
        try {
            val all = core.call<Response.Conversations>(Request.ListConversations).value
            _state.update { it.copy(guests = all.filter { c -> c.kind == ConvKind.GUEST }) }
        } catch (e: ApiException) {
            Log.w(TAG, "Failed to load guest conversations: ${e.code}", e)
        }
    }

    private fun onEvent(e: Event) {
        when (e) {
            is Event.WebRequest -> {
                if (queue.any { it.request == e.request }) return
                queue += PendingRequest(e.request, e.userAgent, e.ip, clock())
                timeouts[e.request] = viewModelScope.launch {
                    delay(REQUEST_TIMEOUT_MS)
                    respondTo(e.request, false)
                }
                publishPending()
            }
            is Event.WebSessionChanged -> {
                val info = e.info
                if (info == null) {
                    finishLocal()
                } else {
                    val last = ((info.urls?.size ?: 1) - 1).coerceAtLeast(0)
                    _state.update { it.copy(info = info, urlIndex = it.urlIndex.coerceAtMost(last)) }
                }
            }
            is Event.ConversationUpdated -> {
                val s = e.summary
                if (s.kind == ConvKind.GUEST) {
                    _state.update { st ->
                        val others = st.guests.filter { it.conv != s.conv }
                        st.copy(guests = (others + s).sortedByDescending { it.lastHlcMillis ?: 0L })
                    }
                }
            }
            else -> Unit
        }
    }

    /** 会话已在 core 侧结束或用户要求结束：清本地状态并让页面返回。 */
    private fun finishLocal() {
        setWebHosting(false)
        timeouts.values.forEach { it.cancel() }
        timeouts.clear()
        queue.clear()
        _state.update { it.copy(info = null, pending = null, ended = true) }
    }

    private suspend fun requestStop() {
        try {
            core.call<Response.Ok>(Request.WebStop)
        } catch (e: ApiException) {
            Log.w(TAG, "WebStop failed: ${e.code}", e)
        }
    }

    override fun onCleared() {
        // 页面被离开时会话仍在：结束它
        if (_state.value.info != null) {
            setWebHosting(false)
            detachedScope.launch { requestStop() }
        }
    }

    companion object {
        private const val TAG = "WebHostViewModel"

        /** 请求 60 秒无人处理则按拒绝处理（与桌面端一致）。 */
        const val REQUEST_TIMEOUT_MS = 60_000L
    }
}
