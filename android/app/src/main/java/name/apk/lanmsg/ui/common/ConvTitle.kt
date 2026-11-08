package name.apk.lanmsg.ui.common

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import name.apk.lanmsg.R
import uniffi.lanmsg_api.ConvKind

/** 会话标题的界面文案：core 对“我的设备”与网页访客会话不给标题，由界面按类型本地化。 */
@Composable
fun convTitle(kind: ConvKind?, title: String): String = when (kind) {
    ConvKind.SELF_DEVICES -> stringResource(R.string.conv_self_title)
    ConvKind.GUEST -> stringResource(R.string.conv_guest_title)
    else -> title
}
