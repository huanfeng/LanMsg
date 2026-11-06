package name.apk.lanmsg.ui.chat

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.FileItemView
import uniffi.lanmsg_api.MessageBodyView
import uniffi.lanmsg_api.MessageState
import uniffi.lanmsg_api.MessageView

/** 气泡上的全部用户动作，ChatContent 与 MessageBubble 共用。 */
class BubbleActions(
    val onResend: (MessageView) -> Unit = {},
    val onDownload: (String) -> Unit = {},
    val onCancel: (String) -> Unit = {},
    val onOpenImage: (String) -> Unit = {},
    val onOpenFile: (FileItemView) -> Unit = {},
    val onRecall: (String) -> Unit = {},
    val onDelete: (String) -> Unit = {},
    val onForward: (List<String>) -> Unit = {},
)

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun MessageBubble(
    msg: MessageView,
    showDeviceName: Boolean,
    canRecall: Boolean,
    progress: Map<String, Pair<ULong, ULong>>,
    actions: BubbleActions,
) {
    val c = LanMsgColors.current
    // 撤回、系统消息、不支持的类型：居中灰字，无气泡
    if (msg.state == MessageState.RECALLED) {
        CenterNote(stringResource(if (msg.isMine) R.string.msg_recalled_by_me else R.string.msg_recalled))
        return
    }
    val body = msg.body
    when (body) {
        is MessageBodyView.System -> { CenterNote(body.text); return }
        is MessageBodyView.Unsupported -> { CenterNote(stringResource(R.string.msg_unsupported)); return }
        else -> Unit
    }

    val mine = msg.isMine
    val bg = if (mine) c.signalFill else c.surface
    val fg = if (mine) c.onFill else c.ink
    // “我的”气泡底是 signal_fill，次要文字也用白色以保证对比度
    val sub = if (mine) c.onFill else c.inkSubtle
    var menu by remember { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current
    val maxWidth = (LocalConfiguration.current.screenWidthDp * 0.75f).dp

    Column(Modifier.fillMaxWidth(), horizontalAlignment = if (mine) Alignment.End else Alignment.Start) {
        if (showDeviceName) {
            Text(stringResource(R.string.chat_from_device, msg.authorDeviceName), color = c.inkSubtle, fontSize = 12.sp, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp))
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            if (mine) StateMark(msg, actions)
            Box {
                Surface(
                    color = bg,
                    shape = RoundedCornerShape(12.dp),
                    border = if (mine) null else BorderStroke(1.dp, c.line),
                    modifier = Modifier.widthIn(max = maxWidth),
                ) {
                    val textClick = Modifier.combinedClickable(
                        onClick = { if (msg.state == MessageState.FAILED) actions.onResend(msg) },
                        onLongClick = { menu = true },
                    )
                    Column(Modifier.then(if (body is MessageBodyView.Text) textClick else Modifier).padding(horizontal = 12.dp, vertical = 8.dp)) {
                        when (body) {
                            is MessageBodyView.Text -> Text(body.text, color = fg, style = MaterialTheme.typography.bodyLarge)
                            is MessageBodyView.Files -> FilesBody(body.items, progress, fg, sub, actions) { menu = true }
                            is MessageBodyView.Folder -> {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Icon(Icons.Outlined.Folder, contentDescription = null, tint = fg, modifier = Modifier.size(20.dp))
                                    Spacer(Modifier.size(6.dp))
                                    Text(body.name, color = fg, style = MaterialTheme.typography.bodyMedium)
                                }
                                FilesBody(body.items, progress, fg, sub, actions) { menu = true }
                            }
                            else -> Unit
                        }
                    }
                }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = c.surface) {
                    if (body is MessageBodyView.Text) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.msg_copy), color = c.ink) },
                            onClick = { menu = false; clipboard.setText(AnnotatedString(body.text)) },
                        )
                    }
                    val hashes = when (body) {
                        is MessageBodyView.Files -> body.items.map { it.hash }
                        is MessageBodyView.Folder -> body.items.map { it.hash }
                        else -> emptyList()
                    }
                    if (hashes.isNotEmpty()) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.action_forward), color = c.ink) },
                            onClick = { menu = false; actions.onForward(hashes) },
                        )
                    }
                    if (canRecall) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.msg_recall), color = c.ink) },
                            onClick = { menu = false; actions.onRecall(msg.id) },
                        )
                    }
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.msg_delete), color = c.danger) },
                        onClick = { menu = false; actions.onDelete(msg.id) },
                    )
                }
            }
        }
        if (mine && msg.state == MessageState.READ) {
            Text(stringResource(R.string.msg_read), color = c.inkSubtle, fontSize = 12.sp, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp))
        }
    }
}

@Composable
private fun StateMark(msg: MessageView, actions: BubbleActions) {
    val c = LanMsgColors.current
    when (msg.state) {
        MessageState.SENDING -> CircularProgressIndicator(color = c.signal, strokeWidth = 2.dp, modifier = Modifier.size(16.dp))
        MessageState.FAILED -> Icon(
            Icons.Outlined.ErrorOutline,
            contentDescription = stringResource(R.string.msg_resend),
            tint = c.danger,
            modifier = Modifier.size(20.dp).clickable { actions.onResend(msg) },
        )
        else -> Unit
    }
}

/** Files 消息：图片/视频进媒体网格，其余是文件卡片。 */
@Composable
private fun FilesBody(
    items: List<FileItemView>,
    progress: Map<String, Pair<ULong, ULong>>,
    fg: Color,
    sub: Color,
    actions: BubbleActions,
    onLongClick: () -> Unit,
) {
    val media = items.filter { fileKindOf(it.mime).let { k -> k == FileKind.IMAGE || k == FileKind.VIDEO } }
    val others = items - media.toSet()
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (media.isNotEmpty()) {
            MediaGrid(
                items = media,
                progress = progress,
                onClick = { item ->
                    when {
                        fileKindOf(item.mime) == FileKind.IMAGE -> actions.onOpenImage(item.hash)
                        item.state == BlobState.REMOTE -> actions.onDownload(item.hash)
                        else -> actions.onOpenFile(item)
                    }
                },
                onLongClick = onLongClick,
            )
        }
        for (item in others) {
            FileCard(item, progress[item.hash], fg, sub, actions.onDownload, actions.onCancel, actions.onOpenFile, onLongClick)
        }
    }
}

@Composable
private fun CenterNote(text: String) {
    Text(
        text,
        color = LanMsgColors.current.inkSubtle,
        fontSize = 12.sp,
        textAlign = TextAlign.Center,
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
    )
}
