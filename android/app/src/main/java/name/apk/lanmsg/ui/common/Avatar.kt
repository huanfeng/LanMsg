package name.apk.lanmsg.ui.common

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import name.apk.lanmsg.ui.theme.AvatarPalette
import name.apk.lanmsg.ui.theme.LanMsgColors

/** 圆形头像：底色由 id 的 djb2 取色（两端一致），中间显示显示名第一个字符，在线时右下 8dp 绿点。 */
@Composable
fun Avatar(name: String, userIdHex: String, size: Dp = 40.dp, online: Boolean = false) {
    val c = LanMsgColors.current
    val initial = name.trim().takeIf { it.isNotEmpty() }?.let { String(Character.toChars(it.codePointAt(0))) } ?: "?"
    Box(Modifier.size(size)) {
        Box(
            Modifier.size(size).background(AvatarPalette[Format.avatarColorIndex(userIdHex)], CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Text(initial, color = c.onFill, fontSize = (size.value * 0.42f).sp)
        }
        if (online) {
            StatusDot(
                online = true,
                modifier = Modifier.align(Alignment.BottomEnd).border(1.5.dp, c.surface, CircleShape),
            )
        }
    }
}
