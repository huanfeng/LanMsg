package name.apk.lanmsg.ui.common

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.Language
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.ConversationSummary

/** 会话头像：“我的设备”用设备图标、网页访客用地球图标（标题为空，无首字母可取） + signalSoft 圆底，其余用 [Avatar]；[online] 时右下角叠在线点。 */
@Composable
fun ConversationAvatar(summary: ConversationSummary, size: Dp, online: Boolean = false) {
    if (summary.kind != ConvKind.SELF_DEVICES && summary.kind != ConvKind.GUEST) {
        Avatar(name = summary.title, userIdHex = summary.conv, size = size, online = online)
        return
    }
    val c = LanMsgColors.current
    Box(Modifier.size(size)) {
        Box(Modifier.size(size).background(c.signalSoft, CircleShape), contentAlignment = Alignment.Center) {
            Icon(
                if (summary.kind == ConvKind.GUEST) Icons.Outlined.Language else Icons.Outlined.Devices,
                contentDescription = null,
                tint = c.signal,
            )
        }
        if (online) {
            Box(
                Modifier.align(Alignment.BottomEnd).size(11.dp).background(c.surface, CircleShape),
                contentAlignment = Alignment.Center,
            ) { StatusDot(online = true) }
        }
    }
}
