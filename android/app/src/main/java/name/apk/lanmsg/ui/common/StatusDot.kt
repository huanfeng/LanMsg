package name.apk.lanmsg.ui.common

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import name.apk.lanmsg.ui.theme.LanMsgColors

/** 在线绿点；离线用 line 色灰点。 */
@Composable
fun StatusDot(online: Boolean, modifier: Modifier = Modifier) {
    val c = LanMsgColors.current
    Box(modifier.size(8.dp).background(if (online) c.online else c.line, CircleShape))
}
