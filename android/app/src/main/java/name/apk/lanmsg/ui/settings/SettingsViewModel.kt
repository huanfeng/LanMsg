package name.apk.lanmsg.ui.settings

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
    /** 后台模式写入成功后通知服务重新评估；默认在调用时才读 ServiceLocator，Task 11 替换后自动生效。 */
    private val onBackgroundMode: (BackgroundMode) -> Unit = { ServiceLocator.backgroundModeListener(it) },
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {
    data class UiState(
        val settings: SettingsDto? = null,
        val notifications: Boolean = true,
        val saveToGallery: Boolean = true,
        val trustedWifi: Set<String> = emptySet(),
        val outboxBytes: Long = 0,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** SetSettings 是整体回写，必须串行，否则后发的快照可能被先发的失败回滚覆盖。 */
    private val writeLock = Mutex()
    private val wifiLock = Mutex()

    init {
        viewModelScope.launch { prefs.notificationsEnabled.collect { v -> _state.update { it.copy(notifications = v) } } }
        viewModelScope.launch { prefs.saveMediaToGallery.collect { v -> _state.update { it.copy(saveToGallery = v) } } }
        viewModelScope.launch { prefs.trustedWifi.collect { v -> _state.update { it.copy(trustedWifi = v) } } }
        viewModelScope.launch { core.lagged.collect { loadSettings() } }
        viewModelScope.launch { loadSettings() }
        refreshOutboxSize()
    }

    /** 立即乐观更新，并以整体 SetSettings 回写；失败回滚并给出 error。 */
    fun update(transform: (SettingsDto) -> SettingsDto) {
        val old = _state.value.settings ?: return
        val new = transform(old.copy())
        _state.update { it.copy(settings = new, error = null) }
        viewModelScope.launch {
            writeLock.withLock {
                try {
                    core.call<Response.Ok>(Request.SetSettings(new))
                    if (new.backgroundMode != old.backgroundMode) onBackgroundMode(new.backgroundMode)
                } catch (e: ApiException) {
                    _state.update { s ->
                        s.copy(settings = if (s.settings == new) old else s.settings, error = e.message ?: e.code.name)
                    }
                }
            }
        }
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
            withContext(io) { outbox.clear() }
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
        try {
            val s = core.call<Response.Settings>(Request.GetSettings).value
            _state.update { it.copy(settings = s, error = null) }
        } catch (e: ApiException) {
            _state.update { it.copy(error = e.message ?: e.code.name) }
        }
    }

    companion object {
        private val MB = 1024uL * 1024uL

        /** 自动下载上限选项（字节）：关闭 = 0，不限 = ULong.MAX_VALUE。 */
        val AUTO_LIMITS: List<ULong> = listOf(0uL, 5uL * MB, 10uL * MB, 50uL * MB, 100uL * MB, ULong.MAX_VALUE)
    }
}
