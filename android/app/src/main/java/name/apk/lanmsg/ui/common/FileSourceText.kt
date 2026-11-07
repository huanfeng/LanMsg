package name.apk.lanmsg.ui.common

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import name.apk.lanmsg.R
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.FileItemView

/** 文件来源文案：按作者是否本人、本机是否已有、持有者设备名选资源。core 只给字段。 */
fun fileSourceFormat(item: FileItemView): EventText {
    val holder = item.holderName
    val local = item.state == BlobState.LOCAL
    return when {
        item.sourceIsMe && local -> EventText(R.string.file_source_me_downloaded, emptyList())
        item.sourceIsMe && holder != null -> EventText(R.string.file_source_me_stored, listOf(holder))
        item.sourceIsMe -> EventText(R.string.file_source_me, emptyList())
        local -> EventText(R.string.file_source_downloaded, listOf(item.sourceName))
        holder != null -> EventText(R.string.file_source_stored, listOf(item.sourceName, holder))
        else -> EventText(R.string.file_source_from, listOf(item.sourceName))
    }
}

/**
 * 聊天里文件卡片的状态行：只说“已下载 / 存于 X”。“来自谁”聊天上下文已经表明
 * （多设备或自设备会话里气泡上方另有来源设备名），卡片内不重复。
 */
fun fileStatusFormat(item: FileItemView): EventText? {
    val holder = item.holderName
    return when {
        item.state == BlobState.LOCAL -> EventText(R.string.files_state_downloaded, emptyList())
        holder != null -> EventText(R.string.file_status_stored, listOf(holder))
        else -> null
    }
}

@Composable
fun fileStatusText(item: FileItemView): String? {
    val f = fileStatusFormat(item) ?: return null
    return stringResource(f.res, *f.args.toTypedArray())
}

@Composable
fun fileSourceText(item: FileItemView): String {
    val f = fileSourceFormat(item)
    return stringResource(f.res, *f.args.toTypedArray())
}
