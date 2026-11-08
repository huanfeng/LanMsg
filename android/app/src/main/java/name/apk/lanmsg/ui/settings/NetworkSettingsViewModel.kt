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
import java.net.InetAddress

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
        /** 单播通告目标有格式不合法的行（未提交）。 */
        val unicastInvalid: Boolean = false,
        val settings: SettingsDto? = null,
        val error: ErrorCode? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

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
                _state.update { it.copy(settings = settings, interfaces = interfaces, manualPeers = peers) }
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

    /** 每行一个目标；空行忽略，任一行不合法则整体不提交。 */
    fun setUnicastTargets(lines: List<String>) {
        val targets = lines.map { it.trim() }.filter { it.isNotEmpty() }
        if (!targets.all(::isValidUnicastTarget)) {
            _state.update { it.copy(unicastInvalid = true) }
            return
        }
        _state.update { it.copy(unicastInvalid = false) }
        write { it.copy(unicastTargets = targets) }
    }

    fun setLocalSendEnabled(v: Boolean) = write { it.copy(localsendEnabled = v) }

    fun clearError() = _state.update { it.copy(error = null) }

    /** 以 core 当前设置为基准回写，避免覆盖设置页等其他入口的修改。 */
    private fun write(transform: (SettingsDto) -> SettingsDto) {
        viewModelScope.launch {
            writeLock.withLock {
                try {
                    val base = core.call<Response.Settings>(Request.GetSettings).value
                    val new = transform(base.copy())
                    core.call<Response.Ok>(Request.SetSettings(new))
                    _state.update { it.copy(settings = new) }
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

        /** 与 core 的 parse_unicast_targets 同一规则：IP、IP:端口（IPv6 为 [IP]:端口）、IPv4 网段（前缀 24 到 32）；不接受主机名。 */
        fun isValidUnicastTarget(item: String): Boolean {
            val s = item.trim()
            if (s.isEmpty()) return false
            if (s.contains('/')) {
                val (ip, prefix) = s.split('/').let { if (it.size == 2) it[0] to it[1] else return false }
                val p = prefix.toIntOrNull() ?: return false
                return isIpv4(ip) && p in 24..32
            }
            if (s.startsWith("[")) {
                val end = s.indexOf(']')
                if (end < 0) return false
                val rest = s.substring(end + 1)
                return isIpv6(s.substring(1, end)) && (rest.isEmpty() || (rest.startsWith(":") && isPort(rest.substring(1))))
            }
            if (isIpv4(s) || isIpv6(s)) return true
            val colon = s.lastIndexOf(':')
            return colon > 0 && isIpv4(s.substring(0, colon)) && isPort(s.substring(colon + 1))
        }

        private fun isPort(s: String): Boolean = s.isNotEmpty() && s.all { it.isDigit() } && s.length <= 5 && s.toInt() in 1..65535

        private fun isIpv4(s: String): Boolean {
            val parts = s.split('.')
            return parts.size == 4 && parts.all { p -> p.isNotEmpty() && p.length <= 3 && p.all { it.isDigit() } && p.toInt() <= 255 }
        }

        /** 只接受十六进制、冒号与点；含冒号的字符串交给 InetAddress 解析字面量（不会触发 DNS）。 */
        private fun isIpv6(s: String): Boolean {
            if (s.count { it == ':' } < 2 || !s.all { it.isDigit() || it in 'a'..'f' || it in 'A'..'F' || it == ':' || it == '.' }) return false
            return try {
                InetAddress.getByName(s) is java.net.Inet6Address
            } catch (e: java.net.UnknownHostException) {
                false
            }
        }
    }
}
