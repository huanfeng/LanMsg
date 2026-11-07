package name.apk.lanmsg.ui.convs

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.NotificationsOff
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.common.ConversationAvatar
import name.apk.lanmsg.ui.common.formatTime
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.ConvKind

/** 会话行（规范 §5）：点按进入聊天，长按弹出免打扰菜单。 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ConversationRow(row: ConversationListViewModel.Row, onClick: () -> Unit, onToggleMute: () -> Unit) {
    val c = LanMsgColors.current
    val s = row.summary
    var menu by remember { mutableStateOf(false) }
    val selfDevices = s.kind == ConvKind.SELF_DEVICES
    val title = if (selfDevices) stringResource(R.string.conv_self_title) else s.title

    Box {
        Column(
            Modifier.fillMaxWidth().background(c.surface).combinedClickable(onClick = onClick, onLongClick = { menu = true }),
        ) {
            Row(Modifier.padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                ConversationAvatar(s, size = 48.dp, online = s.peerOnline)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            title,
                            color = c.ink,
                            style = MaterialTheme.typography.titleMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f, fill = false),
                        )
                        if (row.muted) {
                            Spacer(Modifier.width(4.dp))
                            Icon(Icons.Outlined.NotificationsOff, contentDescription = stringResource(R.string.mute), tint = c.inkSubtle, modifier = Modifier.size(16.dp))
                        }
                    }
                    val preview = conversationPreview(s)
                    Text(
                        when {
                            preview == null -> s.lastText.orEmpty()
                            preview.arg == null -> stringResource(preview.res)
                            else -> stringResource(preview.res, preview.arg)
                        },
                        color = c.inkMuted,
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Spacer(Modifier.width(8.dp))
                Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    s.lastHlcMillis?.let { Text(formatTime(it), color = c.inkSubtle, fontSize = 12.sp) }
                    if (s.unread > 0u) {
                        val text = if (s.unread > 99u) "99+" else s.unread.toString()
                        // 免打扰：灰底；否则 signal_fill 底白字
                        val bg = if (row.muted) c.line else c.signalFill
                        val fg = if (row.muted) c.inkMuted else c.onFill
                        Box(
                            Modifier.background(bg, RoundedCornerShape(10.dp)).padding(horizontal = 6.dp, vertical = 1.dp),
                            contentAlignment = Alignment.Center,
                        ) { Text(text, color = fg, fontSize = 12.sp) }
                    }
                }
            }
            HorizontalDivider(color = c.line)
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = c.surface) {
            DropdownMenuItem(
                text = { Text(stringResource(if (row.muted) R.string.unmute else R.string.mute), color = c.ink) },
                onClick = { menu = false; onToggleMute() },
            )
        }
    }
}
