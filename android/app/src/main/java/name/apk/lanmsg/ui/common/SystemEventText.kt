package name.apk.lanmsg.ui.common

import androidx.annotation.StringRes
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import name.apk.lanmsg.R
import uniffi.lanmsg_api.SystemEventView

/** 系统消息对应的文案资源与格式参数。 */
data class EventText(@StringRes val res: Int, val args: List<Any>)

/** 系统消息 → 资源 id + 参数：core 只给结构，文案来自本地化资源。 */
fun systemEventFormat(event: SystemEventView): EventText = when (event) {
    is SystemEventView.DeviceAdded -> EventText(R.string.sys_device_added, listOf(event.name))
    is SystemEventView.DeviceRemoved -> EventText(R.string.sys_device_removed, listOf(event.name))
    is SystemEventView.ContactAdded -> EventText(R.string.sys_contact_added, listOf(event.name))
    is SystemEventView.ContactUpdated -> EventText(R.string.sys_contact_updated, listOf(event.name))
    is SystemEventView.NicknameUpdated -> EventText(R.string.sys_nickname_updated, listOf(event.name))
    is SystemEventView.MessagesDeleted -> EventText(R.string.sys_messages_deleted, listOf(event.count.toInt()))
    is SystemEventView.Recalled ->
        if (event.byMe) EventText(R.string.sys_recalled_by_me, emptyList())
        else EventText(R.string.sys_recalled_by_peer, listOf(event.authorName))
    is SystemEventView.LocalSendPin -> EventText(R.string.sys_localsend_pin, listOf(event.name))
    is SystemEventView.LocalSendBusy -> EventText(R.string.sys_localsend_busy, listOf(event.name))
}

@Composable
fun systemEventText(event: SystemEventView): String {
    val f = systemEventFormat(event)
    return stringResource(f.res, *f.args.toTypedArray())
}
