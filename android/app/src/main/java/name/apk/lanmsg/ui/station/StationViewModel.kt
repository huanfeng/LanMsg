package name.apk.lanmsg.ui.station

import android.net.Uri
import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.call
import name.apk.lanmsg.files.Importer
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.StationItemView
import uniffi.lanmsg_api.StationPutItem
import uniffi.lanmsg_api.StoreMode

/** 中转站页面。手机端只能使用已有中转站，放入一律复制（[StoreMode.COPY]）。 */
class StationViewModel(
    private val core: CoreApi,
    private val station: String,
    private val importer: Importer,
    private val clock: () -> Long = System::currentTimeMillis,
) : ViewModel() {
    enum class Expiry(val days: Int?) { NEVER(null), DAY1(1), DAY7(7), DAY30(30) }

    data class UiState(
        val name: String = "",
        val hostOnline: Boolean = false,
        val items: List<StationItemView> = emptyList(),
        val expiry: Expiry = Expiry.NEVER,
        /** 有请求在途。连接主机最坏要 13 秒才报 PeerOffline，期间只显示进度，不显示失败。 */
        val busy: Boolean = false,
        /** 最近一次失败的错误码，界面用 ErrorText 本地化；消费后 [clearError]。 */
        val error: ErrorCode? = null,
        val loading: Boolean = true,
        /** 用户下拉刷新中。 */
        val refreshing: Boolean = false,
        /** hash -> (已完成, 总量)。 */
        val progress: Map<String, Pair<ULong, ULong>> = emptyMap(),
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** hash -> transfer id，CancelTransfer 用。 */
    private val transfers = mutableMapOf<String, String>()
    private var pending = 0
    private var refreshJob: Job? = null

    init {
        viewModelScope.launch { core.events.collect(::onEvent) }
        viewModelScope.launch { core.lagged.collect { refresh() } }
        refresh()
    }

    /** 重取本站点的主机状态与条目；失败保留旧列表。新刷新取消在途的旧刷新，避免旧响应覆盖新结果。 */
    fun refresh(): Job {
        refreshJob?.cancel()
        val job = viewModelScope.launch {
            try {
                val view = core.call<Response.Stations>(Request.StationList).value.firstOrNull { it.station == station }
                val items = core.call<Response.StationItems>(Request.StationItems(station)).value
                _state.update {
                    it.copy(
                        name = view?.name ?: it.name,
                        hostOnline = view?.hostOnline ?: false,
                        items = items,
                        loading = false,
                    )
                }
            } catch (e: ApiException) {
                Log.w(TAG, "Failed to refresh station: ${e.code}", e)
                _state.update { it.copy(loading = false, error = e.code) }
            }
        }
        refreshJob = job
        return job
    }

    /** 下拉刷新：刷新期间 [UiState.refreshing] 为 true。 */
    fun pullRefresh() {
        viewModelScope.launch {
            _state.update { it.copy(refreshing = true) }
            try {
                refresh().join()
            } finally {
                _state.update { it.copy(refreshing = false) }
            }
        }
    }

    fun putUris(uris: List<Uri>) {
        if (uris.isEmpty()) return
        launchBusy {
            val paths = importer.import(uris)
            // 全部落地失败：没有可放入的内容，按参数错误提示
            if (paths.isEmpty()) throw ApiException(ErrorCode.INVALID_ARGUMENT, "No file could be imported")
            put(StationPutItem.Paths(paths))
        }
    }

    fun putText(text: String) {
        if (text.isBlank()) return
        launchBusy { put(StationPutItem.Text(text)) }
    }

    /** 只作用于之后的放入。 */
    fun setExpiry(e: Expiry) = _state.update { it.copy(expiry = e) }

    fun setPinned(item: String, pinned: Boolean) =
        launchBusy { core.call<Response.Ok>(Request.StationSetPinned(station, item, pinned)) }

    fun remove(item: String) = launchBusy { core.call<Response.Ok>(Request.StationRemove(station, item)) }

    fun download(hash: String) = launchBusy {
        val r = core.request(Request.DownloadBlob(hash))
        if (r is Response.TransferId) transfers[hash] = r.value
    }

    fun cancel(hash: String) {
        // 没有传输映射（事件尚未到达或已结束）：无可取消，刷新一次让界面回到真实状态
        val transfer = transfers[hash] ?: run { refresh(); return }
        launchBusy { core.call<Response.Ok>(Request.CancelTransfer(transfer)) }
    }

    fun clearError() = _state.update { it.copy(error = null) }

    /** 放入请求。远程站点（主机是别的设备）的复制由主机拉取后完成，手机只登记来源。 */
    private suspend fun put(item: StationPutItem) {
        val days = _state.value.expiry.days
        val expiresAt = days?.let { clock() + it * DAY_MILLIS }
        core.call<Response.Ok>(Request.StationPut(station, item, StoreMode.COPY, expiresAt))
    }

    /** 在途期间 busy 为 true；成功后重取列表，失败记录错误码。 */
    private fun launchBusy(block: suspend () -> Unit) {
        pending++
        _state.update { it.copy(busy = true) }
        viewModelScope.launch {
            try {
                block()
                refresh()
            } catch (e: ApiException) {
                _state.update { it.copy(error = e.code) }
            } finally {
                pending--
                if (pending == 0) _state.update { it.copy(busy = false) }
            }
        }
    }

    private fun onEvent(e: Event) {
        when (e) {
            is Event.StationUpdated -> if (e.station == station) refresh()
            is Event.TransferProgress -> {
                transfers[e.hash] = e.transfer
                _state.update { it.copy(progress = it.progress + (e.hash to (e.done to e.total))) }
            }
            is Event.TransferFinished -> {
                transfers.remove(e.hash)
                _state.update { it.copy(progress = it.progress - e.hash) }
                refresh()
            }
            else -> Unit
        }
    }

    companion object {
        private const val TAG = "StationViewModel"
        private const val DAY_MILLIS = 86_400_000L
    }
}
