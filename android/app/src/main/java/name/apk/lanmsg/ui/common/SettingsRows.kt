package name.apk.lanmsg.ui.common

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls

/** 设置类页面共用的行：分组标题、开关行、下拉选项行。 */
@Composable
fun SectionHeader(text: String) {
    Text(
        text,
        color = LanMsgColors.current.inkMuted,
        style = MaterialTheme.typography.labelLarge,
        modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 6.dp),
    )
}

@Composable
fun SwitchRow(title: String, checked: Boolean, onChange: (Boolean) -> Unit, description: String? = null) {
    val c = LanMsgColors.current
    Row(
        Modifier.fillMaxWidth().clickable { onChange(!checked) }.padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, color = c.ink, style = MaterialTheme.typography.bodyLarge)
            if (description != null) Text(description, color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
        }
        // 整行可点，开关本身不再单独响应，避免点击区域重叠
        Switch(checked = checked, onCheckedChange = null, colors = LanMsgControls.switch())
    }
}

/** 标题 + 当前值，点击弹出下拉菜单选择。[options] 不含当前值时也能正常显示当前值。 */
@Composable
fun <T> OptionRow(title: String, current: String, options: List<T>, label: @Composable (T) -> String, onSelect: (T) -> Unit) {
    val c = LanMsgColors.current
    var open by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().clickable { open = true }.padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(title, color = c.ink, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        Box {
            TextButton(onClick = { open = true }, colors = LanMsgButtons.text(), modifier = Modifier.widthIn(min = 64.dp)) {
                Text(current)
            }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }, containerColor = c.surface) {
                for (o in options) {
                    DropdownMenuItem(
                        text = { Text(label(o), color = c.ink) },
                        onClick = { open = false; onSelect(o) },
                    )
                }
            }
        }
    }
}
