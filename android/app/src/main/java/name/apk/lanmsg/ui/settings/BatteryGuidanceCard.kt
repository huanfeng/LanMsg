package name.apk.lanmsg.ui.settings

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import name.apk.lanmsg.R
import name.apk.lanmsg.service.BatteryGuidance
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.BackgroundMode

/** 实时档下的后台保活引导：电池优化未豁免时提示，小米系再追加厂商省电策略说明。 */
@Composable
fun BatteryGuidanceCard(mode: BackgroundMode, modifier: Modifier = Modifier) {
    val ctx = LocalContext.current
    var ignoring by remember { mutableStateOf(isIgnoringBatteryOptimizations(ctx)) }
    // 从系统对话框 / 设置页回来后重新读取
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) ignoring = isIgnoringBatteryOptimizations(ctx)
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    val showBattery = BatteryGuidance.shouldShowCard(mode, ignoring)
    val vendor = if (mode == BackgroundMode.REALTIME) BatteryGuidance.vendorOf(Build.MANUFACTURER) else null
    BatteryGuidanceCardContent(
        showBattery = showBattery,
        vendor = vendor,
        onRequestIgnore = { requestIgnoreBatteryOptimizations(ctx) },
        onOpenAppDetails = { openAppDetails(ctx) },
        modifier = modifier,
    )
}

@Composable
internal fun BatteryGuidanceCardContent(
    showBattery: Boolean,
    vendor: BatteryGuidance.Vendor?,
    onRequestIgnore: () -> Unit,
    onOpenAppDetails: () -> Unit,
    modifier: Modifier = Modifier,
) {
    if (!showBattery && vendor == null) return
    val c = LanMsgColors.current
    Surface(
        modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
        shape = RoundedCornerShape(12.dp),
        color = c.surface,
        border = BorderStroke(1.dp, c.danger),
    ) {
        Column(Modifier.padding(16.dp)) {
            if (showBattery) {
                Text(stringResource(R.string.battery_card_title), color = c.ink, style = MaterialTheme.typography.titleMedium)
                Text(
                    stringResource(R.string.battery_card_body),
                    color = c.inkMuted,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(top = 4.dp, bottom = 8.dp),
                )
                Button(onClick = onRequestIgnore) { Text(stringResource(R.string.battery_card_action)) }
            }
            if (vendor == BatteryGuidance.Vendor.XIAOMI) {
                Text(
                    stringResource(R.string.battery_vendor_xiaomi),
                    color = c.inkMuted,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(top = if (showBattery) 16.dp else 0.dp, bottom = 4.dp),
                )
                TextButton(onClick = onOpenAppDetails, colors = LanMsgButtons.text()) {
                    Text(stringResource(R.string.battery_vendor_action))
                }
            }
        }
    }
}

private fun isIgnoringBatteryOptimizations(ctx: Context): Boolean {
    val pm = ctx.getSystemService(Context.POWER_SERVICE) as PowerManager
    return pm.isIgnoringBatteryOptimizations(ctx.packageName)
}

/** 本应用不上架 Google Play，可直接弹系统豁免对话框；失败时退回电池优化设置列表。 */
private fun requestIgnoreBatteryOptimizations(ctx: Context) {
    val direct = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:${ctx.packageName}"))
    startSafely(ctx, direct) {
        startSafely(ctx, Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)) {}
    }
}

private fun openAppDetails(ctx: Context) {
    startSafely(ctx, Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${ctx.packageName}"))) {}
}

private fun startSafely(ctx: Context, intent: Intent, onMissing: () -> Unit) {
    try {
        ctx.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    } catch (e: ActivityNotFoundException) {
        Log.w("BatteryGuidanceCard", "Settings screen not available: ${intent.action}", e)
        onMissing()
    }
}
