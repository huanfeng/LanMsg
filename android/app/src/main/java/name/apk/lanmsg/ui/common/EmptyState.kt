package name.apk.lanmsg.ui.common

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import name.apk.lanmsg.ui.theme.LanMsgColors

/**
 * 空状态：图标 + 一句说明；页面有可行动操作时通过 [action] 给出入口按钮（规范 §5）。
 */
@Composable
fun EmptyState(text: String, icon: ImageVector, action: (@Composable () -> Unit)? = null) {
    val c = LanMsgColors.current
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, contentDescription = null, tint = c.inkSubtle, modifier = Modifier.size(48.dp))
        Spacer(Modifier.height(12.dp))
        Text(text, color = c.inkMuted, style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center)
        if (action != null) {
            Spacer(Modifier.height(16.dp))
            action()
        }
    }
}
