package name.apk.lanmsg.ui.files

import uniffi.lanmsg_api.TransferFailReason
import name.apk.lanmsg.ui.common.transferFailRes
import name.apk.lanmsg.ui.common.errorTextRes
import android.util.Log
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
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.FileFilter
import uniffi.lanmsg_api.FileKindFilter
import uniffi.lanmsg_api.FileView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class FilesViewModel(private val core: CoreApi, initialConv: String?) : ViewModel() {
    /** 来源设备筛选的候选项；[self] 为本机（界面显示 devices_this）；[subLabel] 为他人设备的显示名。 */
    data class DeviceOption(val deviceId: String, val label: String, val self: Boolean, val subLabel: String? = null)

    data class UiState(
        val filter: FileFilter,
        val items: List<FileView> = emptyList(),
        val loading: Boolean = true,
        val error: Int? = null,
        /** 下载/取消失败的一次性提示；独立于 [error]，重查不会清掉它，由界面消费后 [clearDownloadError]。 */
        val downloadError: Int? = null,
        /** QueryFiles 未实现时为 true，界面显示 err_core_outdated。 */
        val coreOutdated: Boolean = false,
        val devices: List<DeviceOption> = emptyList(),
        /** hash -> (已完成, 总量)。 */
        val progress: Map<String, Pair<ULong, ULong>> = emptyMap(),
    )

    private val _state = MutableStateFlow(
        UiState(FileFilter(FileKindFilter.ALL, initialConv, null, null, null, null, LIMIT)),
    )
    val state: StateFlow<UiState> = _state.asStateFlow()

    private var queryJob: Job? = null
    private var debounceJob: Job? = null
    private var refreshJob: Job? = null

    /** hash -> transfer id，CancelTransfer 用。 */
    private val transfers = mutableMapOf<String, String>()
    private val cancelled = mutableSetOf<String>()

    init {
        viewModelScope.launch { core.events.collect(::onEvent) }
        viewModelScope.launch { core.lagged.collect { query() } }
        loadDevices()
        query()
    }

    fun setKind(kind: FileKindFilter) = changeFilter { it.copy(kind = kind) }

    fun setState(s: BlobState?) = changeFilter { it.copy(state = s) }

    fun setSourceDevice(d: String?) = changeFilter { it.copy(sourceDevice = d) }

    /** 300ms 防抖；空白查询视为 null。 */
    fun setQuery(q: String) {
        debounceJob?.cancel()
        debounceJob = viewModelScope.launch {
            delay(QUERY_DEBOUNCE_MS)
            changeFilter { it.copy(nameQuery = q.trim().ifEmpty { null }) }
        }
    }

    fun download(hash: String) {
        cancelled -= hash
        viewModelScope.launch {
            try {
                val r = core.request(Request.DownloadBlob(hash))
                if (r is Response.TransferId) transfers[hash] = r.value
                scheduleRefresh()
            } catch (e: ApiException) {
                _state.update { it.copy(downloadError = errorTextRes(e.code)) }
            }
        }
    }

    fun cancel(hash: String) {
        val transfer = transfers[hash] ?: return
        cancelled += hash
        viewModelScope.launch {
            try {
                core.request(Request.CancelTransfer(transfer))
            } catch (e: ApiException) {
                // 取消没发出去，之后的失败是真实失败，不能再当作用户取消
                cancelled -= hash
                _state.update { it.copy(downloadError = errorTextRes(e.code)) }
            }
        }
    }

    fun clearError() = _state.update { it.copy(error = null) }

    fun clearDownloadError() = _state.update { it.copy(downloadError = null) }

    private fun changeFilter(change: (FileFilter) -> FileFilter) {
        _state.update { it.copy(filter = change(it.filter)) }
        query()
    }

    private fun onEvent(e: Event) {
        when (e) {
            is Event.MessageAdded, is Event.MessageUpdated -> scheduleRefresh()
            is Event.TransferProgress -> {
                transfers[e.hash] = e.transfer
                _state.update { it.copy(progress = it.progress + (e.hash to (e.done to e.total))) }
            }
            is Event.TransferFinished -> {
                transfers.remove(e.hash)
                // 用户主动取消也会以 ok=false 结束，不当作错误提示
                val userCancelled = cancelled.remove(e.hash)
                _state.update { s ->
                    val cleared = s.copy(progress = s.progress - e.hash)
                    if (e.ok || userCancelled) cleared else cleared.copy(downloadError = transferFailRes(e.reason ?: TransferFailReason.UNKNOWN))
                }
                scheduleRefresh()
            }
            else -> Unit
        }
    }

    /** 500ms 内的连续事件合并成一次重查。 */
    private fun scheduleRefresh() {
        refreshJob?.cancel()
        refreshJob = viewModelScope.launch {
            delay(REFRESH_DEBOUNCE_MS)
            query()
        }
    }

    /** 重查；新查询取消在途的旧查询，避免旧响应覆盖新筛选的结果。 */
    private fun query() {
        queryJob?.cancel()
        queryJob = viewModelScope.launch {
            try {
                val items = core.call<Response.Files>(Request.QueryFiles(_state.value.filter)).value
                _state.update { it.copy(items = items, loading = false, error = null, coreOutdated = false) }
            } catch (e: ApiException) {
                if (e.isNotImplemented()) {
                    _state.update { it.copy(loading = false, error = null, coreOutdated = true) }
                } else {
                    _state.update { it.copy(loading = false, error = errorTextRes(e.code)) }
                }
            }
        }
    }

    /**
     * 来源设备候选：FileView 不带作者设备，故取 core 能列出的设备——ListMyDevices（含离线的自己的设备，
     * 本机排最前）加 ListPeers 中不属于自己的在线设备。离线的他人设备无法列出，是已知限制。
     */
    private fun loadDevices() {
        viewModelScope.launch {
            try {
                val mine = core.call<Response.Devices>(Request.ListMyDevices).value
                    .map { DeviceOption(it.deviceId, it.name, it.isSelf) }
                    .sortedByDescending { it.self }
                val known = mine.mapTo(HashSet()) { it.deviceId }
                val others = core.call<Response.Peers>(Request.ListPeers).value
                    .filter { it.deviceId !in known }
                    .map { DeviceOption(it.deviceId, it.deviceName, false, it.displayName) }
                _state.update { it.copy(devices = mine + others) }
            } catch (e: ApiException) {
                // 候选项只是筛选便利，失败不打扰用户
                Log.w(TAG, "Failed to list source devices: ${e.code}", e)
            }
        }
    }

    companion object {
        private const val TAG = "FilesViewModel"
        private val LIMIT = 200u
        private const val QUERY_DEBOUNCE_MS = 300L
        private const val REFRESH_DEBOUNCE_MS = 500L
    }
}
