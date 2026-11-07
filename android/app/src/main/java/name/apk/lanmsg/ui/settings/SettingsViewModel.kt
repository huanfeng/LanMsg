package name.apk.lanmsg.ui.settings

import name.apk.lanmsg.ui.common.errorTextRes
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreApi
import name.apk.lanmsg.core.SettingsPrefs
import name.apk.lanmsg.core.call
import name.apk.lanmsg.files.OutboxStore
import uniffi.lanmsg_api.BackgroundMode
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SettingsDto

class SettingsViewModel(
    private val core: CoreApi,
    private val prefs: SettingsPrefs,
    private val outbox: OutboxStore,
    /** 后台模式写入成功后通知服务重新评估；默认在调用时才读 ServiceLocator（测试里 coreHolder 可被替换）。 */
    private val onBackgroundMode: (BackgroundMode) -> Unit = { ServiceLocator.coreHolder.onBackgroundModeChanged(it) },
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val clock: () -> Long = System::currentTimeMillis,
) : ViewModel() {
    data class UiState(
        val settings: SettingsDto? = null,
        val notifications: Boolean = true,
        val saveToGallery: Boolean = true,
        val trustedWifi: Set<String> = emptySet(),
        val outboxBytes: Long = 0,
        val error: Int? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** SetSettings 是整体回写，必须串行；读取（loadSettings）也走同一把锁，避免与写入交错。 */
    private val writeLock = Mutex()
    private val wifiLock = Mutex()

    /** core 最近一次确认的值（读取或写入成功）；写失败且重新读取也失败时回落到它。仅在 writeLock 内读写。 */
    private var confirmed: SettingsDto? = null

    /** 已排队或进行中的写入数；非 0 时界面上是乐观值，读取结果不得覆盖。仅主线程访问。 */
    private var pendingWrites = 0

    /** 写入期间收到的 lagged / 被跳过的读取，写完后补拉一次。 */
    private var reloadAfterWrites = false

    /** 被后续成功写入掩盖的失败，最后一次写完时一并输出。仅在 writeLock 内读写。 */
    private var lastFailure: Int? = null

    init {
        viewModelScope.launch { prefs.notificationsEnabled.collect { v -> _state.update { it.copy(notifications = v) } } }
        viewModelScope.launch { prefs.saveMediaToGallery.collect { v -> _state.update { it.copy(saveToGallery = v) } } }
        viewModelScope.launch { prefs.trustedWifi.collect { v -> _state.update { it.copy(trustedWifi = v) } } }
        viewModelScope.launch {
            core.lagged.collect { if (pendingWrites > 0) reloadAfterWrites = true else loadSettings() }
        }
        viewModelScope.launch { loadSettings() }
        refreshOutboxSize()
    }

    /**
     * 立即乐观更新，并以整体 SetSettings 回写。真正发送时在 core 最近确认值上重放 [transform]，
     * 所以前一次失败的修改不会被后一次带出去；失败后重新读取 core 为准，读取也失败才回落到最近确认值。
     */
    fun update(transform: (SettingsDto) -> SettingsDto) {
        val cur = _state.value.settings ?: return
        _state.update { it.copy(settings = transform(cur.copy()), error = null) }
        pendingWrites++
        viewModelScope.launch {
            writeLock.withLock {
                val base = confirmed ?: cur
                val new = transform(base.copy())
                var failure: Int? = null
                try {
                    core.call<Response.Ok>(Request.SetSettings(new))
                    confirmed = new
                    if (new.backgroundMode != base.backgroundMode) onBackgroundMode(new.backgroundMode)
                } catch (e: ApiException) {
                    failure = errorTextRes(e.code)
                    lastFailure = failure
                } finally {
                    pendingWrites--
                }
                if (pendingWrites == 0) {
                    val shown = if (failure == null) new else fetchSettings() ?: confirmed
                    val reported = lastFailure
                    lastFailure = null
                    _state.update { it.copy(settings = shown ?: it.settings, error = reported ?: it.error) }
                }
            }
            if (pendingWrites == 0 && reloadAfterWrites) {
                reloadAfterWrites = false
                loadSettings()
            }
        }
    }

    /** 读取 core 当前设置并记为最近确认值；失败返回 null。调用方须持有 writeLock。 */
    private suspend fun fetchSettings(): SettingsDto? = try {
        core.call<Response.Settings>(Request.GetSettings).value.also { confirmed = it }
    } catch (e: ApiException) {
        null
    }

    fun setNotifications(v: Boolean) {
        viewModelScope.launch { prefs.setNotificationsEnabled(v) }
    }

    fun setSaveToGallery(v: Boolean) {
        viewModelScope.launch { prefs.setSaveMediaToGallery(v) }
    }

    fun addTrustedWifi(ssid: String) {
        val name = ssid.trim()
        if (name.isEmpty()) return
        editTrustedWifi { it + name }
    }

    fun removeTrustedWifi(ssid: String) {
        editTrustedWifi { it - ssid }
    }

    /** 读-改-写必须串行并以存储中的值为准，否则连续两次添加会互相覆盖。 */
    private fun editTrustedWifi(change: (Set<String>) -> Set<String>) {
        viewModelScope.launch {
            wifiLock.withLock { prefs.setTrustedWifi(change(prefs.trustedWifi.first())) }
        }
    }

    fun clearOutbox() {
        viewModelScope.launch {
            withContext(io) { outbox.clearOlderThan(clock() - OUTBOX_KEEP_MILLIS) }
            refreshOutboxSize()
        }
    }

    private fun refreshOutboxSize() {
        viewModelScope.launch {
            val bytes = withContext(io) { outbox.outboxSizeBytes() }
            _state.update { it.copy(outboxBytes = bytes) }
        }
    }

    private suspend fun loadSettings() {
        writeLock.withLock {
            try {
                val s = core.call<Response.Settings>(Request.GetSettings).value
                confirmed = s
                // 等锁期间若来了新的写入，界面是乐观值：不覆盖，写完后会补拉
                if (pendingWrites == 0) _state.update { it.copy(settings = s, error = null) } else reloadAfterWrites = true
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorTextRes(e.code)) }
            }
        }
    }

    companion object {
        private val MB = 1024uL * 1024uL

        /** 发件箱副本保留天数：core 只记录副本路径，近期副本可能仍有对端未下载。 */
        private const val OUTBOX_KEEP_MILLIS = 7L * 24 * 60 * 60 * 1000

        /** 自动下载上限选项（字节）：关闭 = 0，不限 = ULong.MAX_VALUE。 */
        val AUTO_LIMITS: List<ULong> = listOf(0uL, 5uL * MB, 10uL * MB, 50uL * MB, 100uL * MB, ULong.MAX_VALUE)
    }
}
