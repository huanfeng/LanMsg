package name.apk.lanmsg.ui.common

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.stringResource
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls

/** 单行文本输入对话框；空白内容时确定按钮不可点。 */
@Composable
fun TextInputDialog(title: String, initial: String, onConfirm: (String) -> Unit, onDismiss: () -> Unit) {
    val c = LanMsgColors.current
    var text by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = c.surface,
        titleContentColor = c.ink,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                singleLine = true,
                colors = LanMsgControls.textField(),
                supportingText = if (text.isBlank()) {
                    { Text(stringResource(R.string.onb_name_required), color = c.inkMuted) }
                } else {
                    null
                },
            )
        },
        dismissButton = {
            TextButton(onClick = onDismiss, colors = LanMsgButtons.text()) { Text(stringResource(R.string.cancel)) }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(text.trim()) }, enabled = text.isNotBlank(), colors = LanMsgButtons.text()) {
                Text(stringResource(R.string.confirm))
            }
        },
    )
}
