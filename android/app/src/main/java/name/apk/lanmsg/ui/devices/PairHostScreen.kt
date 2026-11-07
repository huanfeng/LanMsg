package name.apk.lanmsg.ui.devices

import android.widget.Toast
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.common.QrImage
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.PairMethod
import uniffi.lanmsg_api.PairStateDto

@Composable
fun PairHostScreen(onBack: () -> Unit, onDone: () -> Unit) {
    val vm: PairViewModel = viewModel(
        key = "pair-host",
        factory = viewModelFactory { initializer { PairViewModel(ServiceLocator.core, PairViewModel.Role.HOST) } },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { vm.start() }
    DisposableEffect(Unit) { onDispose { vm.leave() } }
    PairDoneEffect(state.pairState, onDone)
    PairHostContent(state, onBack, vm::switchToSas, vm::confirm, vm::retry)
}

/** 配对成功：提示并交给导航层收尾。 */
@Composable
internal fun PairDoneEffect(pairState: PairStateDto?, onDone: () -> Unit) {
    val ctx = LocalContext.current
    LaunchedEffect(pairState is PairStateDto.Done) {
        if (pairState is PairStateDto.Done) {
            Toast.makeText(ctx, R.string.toast_pair_done, Toast.LENGTH_SHORT).show()
            onDone()
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PairHostContent(
    state: PairViewModel.UiState,
    onBack: () -> Unit,
    onSwitchToSas: () -> Unit,
    onConfirm: (Boolean) -> Unit,
    onRetry: () -> Unit,
) {
    val c = LanMsgColors.current
    Scaffold(
        containerColor = c.canvas,
        topBar = { PairTopBar(R.string.pair_host_title, onBack) },
    ) { inner ->
        Column(
            Modifier.padding(inner).fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            val showQr = state.method == PairMethod.QR && state.qrUri != null &&
                (state.pairState == null || state.pairState is PairStateDto.WaitingPeer)
            if (showQr) {
                QrImage(state.qrUri.orEmpty(), size = 240.dp)
                Spacer(Modifier.height(16.dp))
                Text(
                    stringResource(R.string.pair_host_hint),
                    color = c.inkMuted,
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                )
                TextButton(onClick = onSwitchToSas, colors = LanMsgButtons.text()) { Text(stringResource(R.string.pair_use_sas)) }
            } else {
                PairProgress(state, onConfirm, onRetry)
            }
            state.error?.let {
                Spacer(Modifier.height(12.dp))
                Text(stringResource(R.string.err_generic, it), color = c.danger, style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun PairTopBar(title: Int, onBack: () -> Unit) {
    val c = LanMsgColors.current
    TopAppBar(
        title = { Text(stringResource(title)) },
        navigationIcon = {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.back))
            }
        },
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = c.surface,
            titleContentColor = c.ink,
            navigationIconContentColor = c.ink,
        ),
    )
}

/** 二维码之外的所有配对阶段：等待 / 数字比对 / 验证中 / 失败。 */
@Composable
internal fun PairProgress(state: PairViewModel.UiState, onConfirm: (Boolean) -> Unit, onRetry: () -> Unit) {
    val c = LanMsgColors.current
    when (val s = state.pairState) {
        is PairStateDto.ShowSas -> {
            Text(
                stringResource(R.string.pair_sas_hint),
                color = c.inkMuted,
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(24.dp))
            Text(
                formatSas(s.code),
                color = c.ink,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize = 44.sp,
                letterSpacing = 4.sp,
            )
            Spacer(Modifier.height(32.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedButton(
                    onClick = { onConfirm(false) },
                    enabled = !state.confirmSent,
                    modifier = Modifier.weight(1f),
                    border = BorderStroke(1.dp, c.line),
                ) { Text(stringResource(R.string.pair_mismatch), color = c.ink) }
                Button(onClick = { onConfirm(true) }, enabled = !state.confirmSent, modifier = Modifier.weight(1f)) {
                    Text(stringResource(R.string.pair_match))
                }
            }
        }
        PairStateDto.Verifying -> Waiting(R.string.pair_verifying)
        is PairStateDto.Failed -> {
            Text(
                stringResource(R.string.pair_failed, s.reason),
                color = c.danger,
                style = MaterialTheme.typography.bodyLarge,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(16.dp))
            Button(onClick = onRetry) { Text(stringResource(R.string.retry)) }
        }
        PairStateDto.Done -> Unit
        PairStateDto.WaitingPeer, null -> Waiting(R.string.pair_waiting)
    }
}

@Composable
private fun Waiting(text: Int) {
    val c = LanMsgColors.current
    Spacer(Modifier.height(48.dp))
    CircularProgressIndicator(color = LanMsgControls.progress)
    Spacer(Modifier.height(16.dp))
    Text(stringResource(text), color = c.inkMuted, style = MaterialTheme.typography.bodyMedium)
}

/** 6 位数字 3+3 分组；非 6 位原样显示。 */
internal fun formatSas(code: String) = if (code.length == 6) code.substring(0, 3) + " " + code.substring(3) else code
