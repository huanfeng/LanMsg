package name.apk.lanmsg.ui.devices

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.core.app.ActivityCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.SectionHeader
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors

@Composable
fun PairJoinScreen(onBack: () -> Unit, onDone: () -> Unit) {
    val ctx = LocalContext.current
    val vm: PairViewModel = viewModel(
        key = "pair-join",
        factory = viewModelFactory { initializer { PairViewModel(ServiceLocator.core, PairViewModel.Role.JOIN) } },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    // 拒绝后系统不再弹窗（且无需解释）= 永久拒绝，只能去应用设置里开
    var permanentlyDenied by remember { mutableStateOf(false) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        vm.setCameraGranted(granted)
        val activity = ctx.findActivity()
        permanentlyDenied = !granted && activity != null &&
            !ActivityCompat.shouldShowRequestPermissionRationale(activity, Manifest.permission.CAMERA)
    }
    LaunchedEffect(Unit) {
        vm.start()
        if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            vm.setCameraGranted(true)
        } else {
            permission.launch(Manifest.permission.CAMERA)
        }
    }
    // 从应用设置回来后重新检查权限
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        vm.setCameraGranted(ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    PairDoneEffect(state.pairState, onDone)
    PairJoinContent(
        state = state,
        onBack = onBack,
        onSwitchToSas = vm::switchToSas,
        onGrantCamera = {
            if (permanentlyDenied) {
                ctx.startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", ctx.packageName, null)),
                )
            } else {
                permission.launch(Manifest.permission.CAMERA)
            }
        },
        onScanned = vm::onQrScanned,
        onPick = vm::pick,
        onRefresh = vm::refreshCandidates,
        onConfirm = vm::confirm,
        onRetry = vm::retry,
    )
}

@Composable
fun PairJoinContent(
    state: PairViewModel.UiState,
    onSwitchToSas: () -> Unit,
    onGrantCamera: () -> Unit,
    onScanned: (String) -> Unit,
    onPick: (String) -> Unit,
    onConfirm: (Boolean) -> Unit,
    onRetry: () -> Unit,
    onBack: () -> Unit = {},
    onRefresh: () -> Unit = {},
) {
    val c = LanMsgColors.current
    Scaffold(
        containerColor = c.canvas,
        topBar = { PairTopBar(R.string.pair_join_title, onBack) },
    ) { inner ->
        val started = state.session != null || state.pairState != null
        Column(Modifier.padding(inner).fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally) {
            when {
                started -> Column(Modifier.padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    PairProgress(state, onConfirm, onRetry)
                }
                state.joinMode == PairViewModel.JoinMode.PICK -> {
                    SectionHeader(stringResource(R.string.pair_join_pick))
                    TextButton(onClick = onRefresh, colors = LanMsgButtons.text()) { Text(stringResource(R.string.pair_join_refresh)) }
                    if (state.candidates.isEmpty()) {
                        Spacer(Modifier.height(24.dp))
                        Text(stringResource(R.string.pair_waiting), color = c.inkMuted, style = MaterialTheme.typography.bodyMedium)
                    }
                    LazyColumn(Modifier.fillMaxWidth()) {
                        items(state.candidates, key = { it.deviceId }) { p ->
                            Column(Modifier.fillMaxWidth().clickable { onPick(p.deviceId) }.padding(horizontal = 16.dp, vertical = 14.dp)) {
                                Text(p.displayName, color = c.ink, style = MaterialTheme.typography.bodyLarge)
                                Text(p.deviceName, color = c.inkSubtle, style = MaterialTheme.typography.bodySmall)
                            }
                            HorizontalDivider(color = c.line)
                        }
                    }
                }
                !state.cameraGranted -> Column(
                    Modifier.fillMaxSize().padding(24.dp),
                    verticalArrangement = androidx.compose.foundation.layout.Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(
                        stringResource(R.string.pair_need_camera),
                        color = c.ink,
                        style = MaterialTheme.typography.bodyLarge,
                        textAlign = TextAlign.Center,
                    )
                    Spacer(Modifier.height(16.dp))
                    Button(onClick = onGrantCamera) { Text(stringResource(R.string.pair_grant_camera)) }
                    TextButton(onClick = onSwitchToSas, colors = LanMsgButtons.text()) { Text(stringResource(R.string.pair_use_sas)) }
                }
                else -> {
                    Text(
                        stringResource(R.string.pair_join_scan_hint),
                        color = c.inkMuted,
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(16.dp),
                    )
                    QrScanner(onResult = onScanned, modifier = Modifier.weight(1f).fillMaxWidth())
                    TextButton(onClick = onSwitchToSas, colors = LanMsgButtons.text(), modifier = Modifier.padding(8.dp)) {
                        Text(stringResource(R.string.pair_use_sas))
                    }
                }
            }
            state.error?.let {
                Text(stringResource(R.string.err_generic, it), color = c.danger, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(16.dp))
            }
        }
    }
}

private tailrec fun Context.findActivity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.findActivity()
    else -> null
}
