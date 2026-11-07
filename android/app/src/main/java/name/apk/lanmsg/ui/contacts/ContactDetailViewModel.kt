package name.apk.lanmsg.ui.contacts

import name.apk.lanmsg.ui.common.errorTextRes
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
import uniffi.lanmsg_api.ContactView
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class ContactDetailViewModel(private val core: CoreApi, private val user: String) : ViewModel() {
    data class UiState(val contact: ContactView? = null, val openConv: String? = null, val error: Int? = null)

    /** core 最近确认的联系人（读取或写入成功）。仅在 policyLock 内写入，refresh 也更新它。 */
    private var confirmed: ContactView? = null

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    init {
        viewModelScope.launch { core.events.collect { if (it == Event.ContactsChanged) refresh() } }
        viewModelScope.launch { core.lagged.collect { refresh() } }
        refresh()
    }

    /** 读取与写入共用 policyLock：在途的旧读取响应不会晚于成功写入而回退 confirmed。 */
    fun refresh() {
        viewModelScope.launch {
            policyLock.withLock {
                try {
                    val found = core.call<Response.Contacts>(Request.ListContacts).value.firstOrNull { it.userId == user }
                    confirmed = found
                    // 等锁期间若来了新的写入，界面是乐观值：不覆盖，写完后补拉
                    if (pendingPolicy == 0) _state.update { it.copy(contact = found, error = null) } else reloadAfterWrites = true
                } catch (e: ApiException) {
                    _state.update { it.copy(error = errorTextRes(e.code)) }
                }
            }
        }
    }

    fun openDm() {
        viewModelScope.launch {
            try {
                val conv = core.call<Response.ConvId>(Request.OpenDm(user)).value
                _state.update { it.copy(openConv = conv, error = null) }
            } catch (e: ApiException) {
                _state.update { it.copy(error = errorTextRes(e.code)) }
            }
        }
    }

    /** 界面已跳转到聊天页后调用，避免返回时重复跳转。 */
    fun openConvHandled() = _state.update { it.copy(openConv = null) }

    fun setAutoAccept(v: Boolean) = setPolicy { it.copy(autoAccept = v) }

    fun setAskAbove(bytes: ULong?) = setPolicy { it.copy(askAboveBytes = bytes) }

    /** SetContactPolicy 总是整体回写两个字段，必须串行。 */
    private val policyLock = Mutex()
    private var pendingPolicy = 0

    /** 写入期间被跳过的读取，最后一次写完后补拉一次。 */
    private var reloadAfterWrites = false

    /** 被后续成功写入掩盖的失败，最后一次写完时一并输出。仅在 policyLock 内读写。 */
    private var lastFailure: Int? = null

    /** 先乐观更新；失败后重新读取 core 为准，读取也失败才回落到最近一次确认的值。 */
    private fun setPolicy(change: (ContactView) -> ContactView) {
        val cur = _state.value.contact ?: return
        _state.update { it.copy(contact = change(cur)) }
        pendingPolicy++
        viewModelScope.launch {
            policyLock.withLock {
                // 在最近确认值上重放修改，前一次失败的改动不会被带出去
                val base = confirmed ?: cur
                val new = change(base)
                var failure: Int? = null
                try {
                    core.call<Response.Ok>(Request.SetContactPolicy(user, new.autoAccept, new.askAboveBytes))
                    confirmed = new
                } catch (e: ApiException) {
                    failure = errorTextRes(e.code)
                    lastFailure = failure
                } finally {
                    pendingPolicy--
                }
                if (pendingPolicy == 0) {
                    val shown = if (failure == null) new else fetchContact() ?: confirmed
                    val reported = lastFailure
                    lastFailure = null
                    _state.update { it.copy(contact = shown ?: it.contact, error = reported ?: it.error) }
                }
            }
            if (pendingPolicy == 0 && reloadAfterWrites) {
                reloadAfterWrites = false
                refresh()
            }
        }
    }

    private suspend fun fetchContact(): ContactView? = try {
        core.call<Response.Contacts>(Request.ListContacts).value.firstOrNull { it.userId == user }?.also { confirmed = it }
    } catch (e: ApiException) {
        null
    }

    companion object {
        private val MB = 1024uL * 1024uL

        /** 「超过此大小仍询问」选项：null = 不限。 */
        val ASK_ABOVE: List<ULong?> = listOf(null, 10uL * MB, 50uL * MB, 100uL * MB, 500uL * MB)
    }
}
