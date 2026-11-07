package name.apk.lanmsg.ui.onboarding

import android.os.Build
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import name.apk.lanmsg.R
import name.apk.lanmsg.ServiceLocator
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls

/** 有状态入口：完成后通过 [onDone] 通知导航。 */
@Composable
fun OnboardingScreen(onDone: (OnboardingViewModel.Destination) -> Unit) {
    val vm: OnboardingViewModel = viewModel(
        factory = viewModelFactory {
            initializer {
                OnboardingViewModel(ServiceLocator.coreHolder.api, Build.MODEL.orEmpty()) {
                    ServiceLocator.coreHolder.onInitialized()
                }
            }
        },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    LaunchedEffect(state.done) { state.done?.let(onDone) }
    OnboardingContent(
        state = state,
        onNext = vm::next,
        onDisplayName = vm::setDisplayName,
        onDeviceName = vm::setDeviceName,
        onFirstDevice = vm::chooseFirstDevice,
        onJoinExisting = vm::chooseJoinExisting,
    )
}

@Composable
fun OnboardingContent(
    state: OnboardingViewModel.UiState,
    onNext: () -> Unit,
    onDisplayName: (String) -> Unit,
    onDeviceName: (String) -> Unit,
    onFirstDevice: () -> Unit,
    onJoinExisting: () -> Unit,
) {
    val c = LanMsgColors.current
    Surface(color = c.canvas, modifier = Modifier.fillMaxSize()) {
        Column(
            Modifier.safeDrawingPadding().verticalScroll(rememberScrollState()).padding(24.dp),
            verticalArrangement = Arrangement.Center,
        ) {
            Spacer(Modifier.height(48.dp))
            when (state.step) {
                OnboardingViewModel.Step.WELCOME -> {
                    Text(stringResource(R.string.onb_welcome_title), color = c.ink, style = MaterialTheme.typography.headlineMedium)
                    Spacer(Modifier.height(12.dp))
                    Text(stringResource(R.string.onb_welcome_body), color = c.inkMuted, style = MaterialTheme.typography.bodyLarge)
                    Spacer(Modifier.height(32.dp))
                    Button(onClick = onNext, modifier = Modifier.fillMaxWidth()) { Text(stringResource(R.string.onb_start)) }
                }
                OnboardingViewModel.Step.NAME -> {
                    Text(stringResource(R.string.onb_name_title), color = c.ink, style = MaterialTheme.typography.headlineMedium)
                    Spacer(Modifier.height(16.dp))
                    OutlinedTextField(
                        value = state.displayName,
                        onValueChange = onDisplayName,
                        singleLine = true,
                        placeholder = { Text(stringResource(R.string.onb_name_hint), color = c.inkSubtle) },
                        colors = LanMsgControls.textField(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Spacer(Modifier.height(16.dp))
                    OutlinedTextField(
                        value = state.deviceName,
                        onValueChange = onDeviceName,
                        singleLine = true,
                        label = { Text(stringResource(R.string.onb_device_name)) },
                        colors = LanMsgControls.textField(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Spacer(Modifier.height(24.dp))
                    val blank = state.displayName.isBlank()
                    Button(onClick = onNext, enabled = !blank, modifier = Modifier.fillMaxWidth()) {
                        Text(stringResource(R.string.onb_next))
                    }
                    // 禁用状态旁必须说明原因（规范 §5.1）
                    if (blank) {
                        Spacer(Modifier.height(8.dp))
                        Text(stringResource(R.string.onb_name_required), color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
                    }
                }
                OnboardingViewModel.Step.MODE -> {
                    Text(stringResource(R.string.onb_mode_title), color = c.ink, style = MaterialTheme.typography.headlineMedium)
                    Spacer(Modifier.height(24.dp))
                    Button(onClick = onFirstDevice, enabled = !state.busy, modifier = Modifier.fillMaxWidth()) {
                        Text(stringResource(R.string.onb_mode_first))
                    }
                    Spacer(Modifier.height(4.dp))
                    Text(stringResource(R.string.onb_mode_first_desc), color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
                    Spacer(Modifier.height(24.dp))
                    OutlinedButton(
                        onClick = onJoinExisting,
                        enabled = !state.busy,
                        colors = ButtonDefaults.outlinedButtonColors(containerColor = c.surface, contentColor = c.ink),
                        border = BorderStroke(1.dp, c.line),
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text(stringResource(R.string.onb_mode_join)) }
                    Spacer(Modifier.height(4.dp))
                    Text(stringResource(R.string.onb_mode_join_desc), color = c.inkMuted, style = MaterialTheme.typography.bodySmall)
                    if (state.busy) {
                        Spacer(Modifier.height(16.dp))
                        CircularProgressIndicator(color = LanMsgControls.progress, modifier = Modifier.padding(4.dp))
                    }
                }
            }
            val error = state.error
            if (error != null && error != OnboardingViewModel.ERROR_NAME_REQUIRED) {
                Spacer(Modifier.height(16.dp))
                Text(stringResource(R.string.err_generic, error), color = c.danger, style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}
