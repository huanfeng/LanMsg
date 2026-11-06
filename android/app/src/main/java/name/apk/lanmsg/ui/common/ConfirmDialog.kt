package name.apk.lanmsg.ui.common

import androidx.compose.foundation.BorderStroke
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors

/** 确认对话框：取消 + 危险确认（浅色 danger 实底白字；暗色 surface 底 + 1px danger 描边 + danger 字）。 */
@Composable
fun ConfirmDialog(
    title: String,
    text: String,
    confirmLabel: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    val c = LanMsgColors.current
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = c.surface,
        titleContentColor = c.ink,
        textContentColor = c.inkMuted,
        title = { Text(title) },
        text = { Text(text) },
        dismissButton = {
            TextButton(onClick = onDismiss, colors = LanMsgButtons.text()) { Text(stringResource(R.string.cancel)) }
        },
        confirmButton = {
            if (c.isDark) {
                OutlinedButton(
                    onClick = onConfirm,
                    colors = LanMsgButtons.danger(),
                    border = BorderStroke(1.dp, c.danger),
                ) { Text(confirmLabel) }
            } else {
                Button(onClick = onConfirm, colors = LanMsgButtons.danger()) { Text(confirmLabel) }
            }
        },
    )
}
