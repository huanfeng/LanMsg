package name.apk.lanmsg.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable

// 最小主题包装，Task 5 完善（配色、排版、深色模式）。
@Composable
fun LanMsgTheme(content: @Composable () -> Unit) {
    MaterialTheme(content = content)
}
