package name.apk.lanmsg.ui.settings

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.call
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.InterfaceView
import uniffi.lanmsg_api.ManualPeerView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SettingsDto

/** 网络设置页：手动添加设备、网段扫描、单播通告目标、LocalSend。 */
class NetworkSettingsViewModel(private val core: CoreApi) : ViewModel() {
    data class ScanResult(val probed: UInt, val found: UInt)

    data class UiState(
        val interfaces: List<InterfaceView> = emptyList(),
        val manualPeers: List<ManualPeerView> = emptyList(),
        /** 正在扫描的网段；非空时不能再发起扫描。 */
        val scanning: String? = null,
        val scanResult: ScanResult? = null,
        /** 手动添加在途。连接最坏要 13 秒才报 PeerOffline，期间只显示进度，不显示失败。 */
        val manualBusy: Boolean = false,
        /** 最近一次手动添加成功。 */
        val manualOk: Boolean = false,
        val manualError: ErrorCode? = null,
        /** 单播通告目标输入框内容（每行一个）。 */
        val unicastText: String = "",
        /** 单播通告目标有格式不合法的行（未提交）。 */
        val unicastInvalid: Boolean = false,
        val settings: SettingsDto? = null,
        val error: ErrorCode? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** 用户改过输入框后，core 侧设置的变化不再覆盖输入框，保存成功后才恢复同步。 */
    private var unicastEdited = false

    /** SetSettings 是整体回写，读-改-写必须串行。 */
    private val writeLock = Mutex()

    init {
        viewModelScope.launch { core.events.collect(::onEvent) }
        viewModelScope.launch { core.lagged.collect { load() } }
        load()
    }

    fun load() {
        viewModelScope.launch {
            try {
                val settings = core.call<Response.Settings>(Request.GetSettings).value
                val interfaces = core.call<Response.Interfaces>(Request.ListInterfaces).value
                val peers = core.call<Response.ManualPeers>(Request.ListManualPeers).value
                _state.update {
                    it.copy(
                        settings = settings, interfaces = interfaces, manualPeers = peers,
                        unicastText = if (unicastEdited) it.unicastText else settings.unicastTargets.joinToString("\n"),
                    )
                }
            } catch (e: ApiException) {
                Log.w(TAG, "Failed to load network settings: ${e.code}", e)
                _state.update { it.copy(error = e.code) }
            }
        }
    }

    fun addManualPeer(addr: String) {
        val a = addr.trim()
        if (a.isEmpty() || _state.value.manualBusy) return
        _state.update { it.copy(manualBusy = true, manualOk = false, manualError = null) }
        viewModelScope.launch {
            try {
                core.call<Response.Ok>(Request.AddManualPeer(a))
                _state.update { it.copy(manualOk = true) }
            } catch (e: ApiException) {
                _state.update { it.copy(manualError = e.code) }
            } finally {
                _state.update { it.copy(manualBusy = false) }
            }
            reloadPeers()
        }
    }

    fun removeManualPeer(addr: String) {
        viewModelScope.launch {
            try {
                core.call<Response.Ok>(Request.RemoveManualPeer(addr))
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.code) }
            }
            reloadPeers()
        }
    }

    fun scan(cidr: String) {
        val c = cidr.trim()
        if (c.isEmpty() || _state.value.scanning != null) return
        _state.update { it.copy(scanning = c, scanResult = null) }
        viewModelScope.launch {
            try {
                core.call<Response.Ok>(Request.ScanSubnet(c))
            } catch (e: ApiException) {
                _state.update { it.copy(scanning = null, error = e.code) }
            }
        }
    }

    fun editUnicast(text: String) {
        unicastEdited = true
        _state.update { it.copy(unicastText = text, unicastInvalid = false) }
    }

    /** 提交输入框内容。 */
    fun saveUnicast() = setUnicastTargets(_state.value.unicastText.lines())

    /** 每行一个目标；空行忽略，任一行不合法则整体不提交。 */
    fun setUnicastTargets(lines: List<String>) {
        val targets = lines.map { it.trim() }.filter { it.isNotEmpty() }
        if (!targets.all(::isValidUnicastTarget)) {
            _state.update { it.copy(unicastInvalid = true) }
            return
        }
        _state.update { it.copy(unicastInvalid = false) }
        write(onDone = {
            unicastEdited = false
            _state.update { st -> st.copy(unicastText = targets.joinToString("\n")) }
        }) { it.copy(unicastTargets = targets) }
    }

    fun setLocalSendEnabled(v: Boolean) = write { it.copy(localsendEnabled = v) }

    fun clearError() = _state.update { it.copy(error = null) }

    /** 以 core 当前设置为基准回写，避免覆盖设置页等其他入口的修改。 */
    private fun write(onDone: () -> Unit = {}, transform: (SettingsDto) -> SettingsDto) {
        viewModelScope.launch {
            writeLock.withLock {
                try {
                    val base = core.call<Response.Settings>(Request.GetSettings).value
                    val new = transform(base.copy())
                    core.call<Response.Ok>(Request.SetSettings(new))
                    _state.update { it.copy(settings = new) }
                    onDone()
                } catch (e: ApiException) {
                    _state.update { it.copy(error = e.code) }
                    reloadSettings()
                }
            }
        }
    }

    private suspend fun reloadSettings() {
        try {
            val s = core.call<Response.Settings>(Request.GetSettings).value
            _state.update { it.copy(settings = s) }
        } catch (e: ApiException) {
            Log.w(TAG, "Failed to reload settings: ${e.code}", e)
        }
    }

    private suspend fun reloadPeers() {
        try {
            val peers = core.call<Response.ManualPeers>(Request.ListManualPeers).value
            _state.update { it.copy(manualPeers = peers) }
        } catch (e: ApiException) {
            Log.w(TAG, "Failed to list manual peers: ${e.code}", e)
        }
    }

    private fun onEvent(e: Event) {
        if (e is Event.ScanFinished) {
            _state.update { it.copy(scanning = null, scanResult = ScanResult(e.probed, e.found)) }
        }
    }

    companion object {
        private const val TAG = "NetworkSettingsVM"

        /**
         * 与 core 的 parse_unicast_targets 同一规则（Rust 标准库的解析）：IP、IP:端口（端口 1 到 65535）、
         * [IPv6]:端口（可带数字 scope，如 [fe80::1%1]:80）、IPv4 网段（前缀 24 到 32）；不接受主机名、
         * 不带端口的方括号 IPv6、IPv6 网段。
         */
        fun isValidUnicastTarget(item: String): Boolean {
            val s = item.trim()
            if (s.isEmpty()) return false
            if (s.contains('/')) {
                val parts = s.split('/')
                if (parts.size != 2) return false
                val p = parts[1].takeIf { it.isNotEmpty() && it.all(Char::isDigit) && it.length <= 2 }?.toInt() ?: return false
                return isIpv4(parts[0]) && p in 24..32
            }
            if (s.startsWith("[")) {
                val end = s.indexOf(']')
                if (end < 0) return false
                val host = s.substring(1, end)
                val ip = host.substringBefore('%')
                val scope = if (host.contains('%')) host.substringAfter('%') else null
                val rest = s.substring(end + 1)
                val scopeOk = scope == null || (scope.isNotEmpty() && scope.all(Char::isDigit))
                return isIpv6(ip) && scopeOk && rest.startsWith(":") && isPort(rest.substring(1))
            }
            if (isIpv4(s) || isIpv6(s)) return true
            val colon = s.lastIndexOf(':')
            return colon > 0 && isIpv4(s.substring(0, colon)) && isPort(s.substring(colon + 1))
        }

        private fun isPort(s: String): Boolean = s.isNotEmpty() && s.all(Char::isDigit) && s.length <= 5 && s.toInt() in 1..65535

        private fun isIpv4(s: String): Boolean {
            val parts = s.split('.')
            return parts.size == 4 && parts.all { p -> p.isNotEmpty() && p.length <= 3 && p.all(Char::isDigit) && p.toInt() <= 255 }
        }

        /** IPv6 字面量：8 组十六进制（每组 1 到 4 位），至多一个 "::"，末尾可为 IPv4（算 2 组）。不走系统解析，避免 DNS。 */
        private fun isIpv6(s: String): Boolean {
            if (s.isEmpty() || !s.all { it.isDigit() || it in 'a'..'f' || it in 'A'..'F' || it == ':' || it == '.' }) return false
            val dbl = s.indexOf("::")
            if (dbl >= 0 && s.indexOf("::", dbl + 1) >= 0) return false
            val head = if (dbl >= 0) s.substring(0, dbl) else s
            val tail = if (dbl >= 0) s.substring(dbl + 2) else ""
            val h = countGroups(head, lastMayBeIpv4 = dbl < 0) ?: return false
            val t = if (dbl >= 0) countGroups(tail, lastMayBeIpv4 = true) ?: return false else 0
            return if (dbl >= 0) h + t <= 7 else h == 8
        }

        private fun countGroups(part: String, lastMayBeIpv4: Boolean): Int? {
            if (part.isEmpty()) return 0
            val groups = part.split(':')
            var n = 0
            for ((i, g) in groups.withIndex()) {
                if (g.isEmpty()) return null
                if (g.contains('.')) {
                    if (!(lastMayBeIpv4 && i == groups.lastIndex && isIpv4(g))) return null
                    n += 2
                } else {
                    if (g.length > 4) return null
                    n += 1
                }
            }
            return n
        }
    }
}
