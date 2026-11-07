package name.apk.lanmsg.ui.devices

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.call
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.PairMethod
import uniffi.lanmsg_api.PairStateDto
import uniffi.lanmsg_api.PairTarget
import uniffi.lanmsg_api.PeerView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

/** 配对页（主持 / 加入）的状态机。只处理 session 与当前一致的 PairStatus 事件。 */
class PairViewModel(private val core: CoreApi, val role: Role) : ViewModel() {
    enum class Role { HOST, JOIN }

    enum class JoinMode { SCAN, PICK }

    data class UiState(
        val method: PairMethod = PairMethod.QR,
        val joinMode: JoinMode = JoinMode.SCAN,
        val session: String? = null,
        val qrUri: String? = null,
        val pairState: PairStateDto? = null,
        val candidates: List<PeerView> = emptyList(),
        val cameraGranted: Boolean = false,
        val confirmSent: Boolean = false,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** 已发出 PairStart / PairJoin 但响应未回：期间同步拦截重复调用（扫码会连续回调）。仅主线程访问。 */
    private var requesting = false

    /** 响应未回时先到的状态事件，拿到 session 后若匹配则补用。 */
    private var early: Event.PairStatus? = null

    init {
        viewModelScope.launch { core.events.collect(::onEvent) }
    }

    /** HOST：开始二维码配对；JOIN：不发请求，等待扫码或选设备。 */
    fun start() {
        if (role == Role.HOST && !requesting && _state.value.session == null) {
            begin(PairMethod.QR)
        }
    }

    /** 配对失败后重试：丢弃旧会话状态，重新开始。 */
    fun retry() {
        requesting = false
        early = null
        _state.update {
            it.copy(
                method = PairMethod.QR, joinMode = JoinMode.SCAN, session = null, qrUri = null,
                pairState = null, confirmSent = false, error = null,
            )
        }
        start()
    }

    fun switchToSas() {
        if (role == Role.HOST) {
            cancelCurrent()
            requesting = false
            _state.update { it.copy(session = null, qrUri = null, pairState = null, confirmSent = false) }
            begin(PairMethod.SAS)
        } else {
            _state.update { it.copy(joinMode = JoinMode.PICK) }
            loadCandidates()
        }
    }

    fun onQrScanned(text: String) {
        val s = _state.value
        if (role != Role.JOIN || s.joinMode != JoinMode.SCAN || s.session != null || requesting) return
        if (!text.startsWith(QR_PREFIX)) return
        join(PairTarget.QrUri(text))
    }

    fun pick(device: String) {
        if (role != Role.JOIN || _state.value.session != null || requesting) return
        join(PairTarget.Device(device))
    }

    /** 同一 session 只发送一次。 */
    fun confirm(matches: Boolean) {
        val s = _state.value
        val session = s.session ?: return
        if (s.confirmSent) return
        _state.update { it.copy(confirmSent = true) }
        viewModelScope.launch {
            try {
                core.call<Response.Ok>(Request.PairConfirm(session, matches))
            } catch (e: ApiException) {
                _state.update { it.copy(confirmSent = false, error = e.message ?: e.code.name) }
            }
        }
    }

    fun setCameraGranted(v: Boolean) = _state.update { it.copy(cameraGranted = v) }

    /** 离开页面：会话未结束则取消。可重复调用。 */
    fun leave() {
        cancelCurrent()
        _state.update { it.copy(session = null) }
    }

    private fun cancelCurrent() {
        val s = _state.value
        val session = s.session ?: return
        if (s.pairState is PairStateDto.Done || s.pairState is PairStateDto.Failed) return
        // 页面销毁时 viewModelScope 会被取消，取消请求必须仍然发出
        viewModelScope.launch(NonCancellable) {
            try {
                core.call<Response.Ok>(Request.PairCancel(session))
            } catch (e: ApiException) {
                // 会话可能已被对端结束，取消失败无需处理
            }
        }
    }

    private fun begin(method: PairMethod) {
        requesting = true
        _state.update { it.copy(method = method, error = null) }
        viewModelScope.launch {
            try {
                val info = core.call<Response.PairSession>(Request.PairStart(method)).value
                onSession(info.session, info.qrUri)
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.message ?: e.code.name) }
            } finally {
                requesting = false
            }
        }
    }

    private fun join(target: PairTarget) {
        requesting = true
        _state.update { it.copy(error = null) }
        viewModelScope.launch {
            try {
                val info = core.call<Response.PairSession>(Request.PairJoin(target)).value
                onSession(info.session, info.qrUri)
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.message ?: e.code.name) }
            } finally {
                requesting = false
            }
        }
    }

    private fun onSession(session: String, qrUri: String?) {
        val pending = early?.takeIf { it.session == session }
        early = null
        _state.update { it.copy(session = session, qrUri = qrUri, pairState = pending?.state, confirmSent = false) }
    }

    private fun onEvent(e: Event) {
        when (e) {
            is Event.PairStatus -> {
                val cur = _state.value.session
                when {
                    cur == e.session -> _state.update { it.copy(pairState = e.state) }
                    cur == null && requesting -> early = e
                }
            }
            is Event.PeerOnline, is Event.PeerOffline ->
                if (role == Role.JOIN && _state.value.joinMode == JoinMode.PICK) loadCandidates()
            else -> Unit
        }
    }

    private fun loadCandidates() {
        viewModelScope.launch {
            try {
                val peers = core.call<Response.Peers>(Request.ListPeers).value
                _state.update { it.copy(candidates = peers.filter { p -> !p.isContact }.distinctBy { p -> p.deviceId }) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.message ?: e.code.name) }
            }
        }
    }

    companion object {
        const val QR_PREFIX = "lanmsg://pair?"
    }
}
