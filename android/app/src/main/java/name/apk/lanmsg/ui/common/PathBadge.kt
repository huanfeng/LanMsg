package name.apk.lanmsg.ui.common

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.PathClassDto

/** 连接路径角标：LAN 与未知不显示；VPN/公网/中继显示小号描边标签。 */
@Composable
fun PathBadge(path: PathClassDto?) {
    val label = when (path) {
        PathClassDto.VPN -> R.string.path_vpn
        PathClassDto.INTERNET -> R.string.path_internet
        PathClassDto.RELAY -> R.string.path_relay
        PathClassDto.LAN, null -> return
    }
    val c = LanMsgColors.current
    Surface(
        shape = MaterialTheme.shapes.small,
        color = c.surface,
        border = BorderStroke(1.dp, c.line),
    ) {
        Text(
            stringResource(label),
            color = c.inkMuted,
            fontSize = 12.sp,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 1.dp),
        )
    }
}
