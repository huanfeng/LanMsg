package name.apk.lanmsg.ui.common

import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import name.apk.lanmsg.R

/** 按规范 §3 格式化时间，文案取自 strings.xml。 */
@Composable
fun formatTime(millis: Long, now: Long = System.currentTimeMillis()): String {
    val context = LocalContext.current
    return Format.time(
        millis,
        now,
        yesterday = { context.getString(R.string.time_yesterday, it) },
        monthDay = stringResource(R.string.time_month_day),
    )
}
