package name.apk.lanmsg.ui.web

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
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
import name.apk.lanmsg.ui.common.ConversationAvatar
import name.apk.lanmsg.ui.common.QrImage
import name.apk.lanmsg.ui.common.SectionHeader
import name.apk.lanmsg.ui.common.convTitle
import name.apk.lanmsg.ui.common.errorTextRes
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgControls
import uniffi.lanmsg_api.ConversationSummary

@Composable
fun WebHostScreen(onBack: () -> Unit, onOpenGuest: (String) -> Unit) {
    val vm: WebHostViewModel = viewModel(
        factory = viewModelFactory {
            initializer {
                WebHostViewModel(
                    core = ServiceLocator.coreHolder.api,
                    setWebHosting = ServiceLocator.deviceConditions::setWebHosting,
                    detachedScope = ServiceLocator.appScope,
                )
            }
        },
    )
    val state by vm.state.collectAsStateWithLifecycle()
    val ctx = LocalContext.current
    val errorText = state.error?.let { stringResource(errorTextRes(it)) }
    LaunchedEffect(Unit) { vm.start() }
    LaunchedEffect(errorText) {
        if (errorText != null) {
            Toast.makeText(ctx, errorText, Toast.LENGTH_LONG).show()
            vm.clearError()
        }
    }
    // 会话在别处结束（空闲超时、启动失败、点击结束）：回到上一页
    LaunchedEffect(state.ended) { if (state.ended) onBack() }
    WebHostContent(
        state = state,
        onBack = onBack,
        onStop = vm::stop,
        onSelectUrl = vm::selectUrl,
        onRespond = vm::respond,
        onOpenGuest = onOpenGuest,
        onCopy = { url ->
            val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            cm.setPrimaryClip(ClipData.newPlainText(null, url))
            Toast.makeText(ctx, R.string.toast_copied, Toast.LENGTH_SHORT).show()
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WebHostContent(
    state: WebHostViewModel.UiState,
    onBack: () -> Unit,
    onStop: () -> Unit,
    onSelectUrl: (Int) -> Unit,
    onRespond: (Boolean) -> Unit,
    onOpenGuest: (String) -> Unit,
    onCopy: (String) -> Unit,
) {
    val c = LanMsgColors.current
    Scaffold(
        containerColor = c.canvas,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.web_title)) },
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
        },
    ) { inner ->
        val info = state.info
        if (info == null) {
            Box(Modifier.padding(inner).fillMaxSize(), contentAlignment = Alignment.Center) {
                Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    CircularProgressIndicator(color = LanMsgControls.progress)
                    Text(stringResource(R.string.web_starting), color = c.inkMuted, style = MaterialTheme.typography.bodyMedium)
                }
            }
            return@Scaffold
        }
        val urls = info.urls?.takeIf { it.isNotEmpty() } ?: listOfNotNull(info.url.takeIf { it.isNotBlank() })
        val current = urls.getOrNull(state.urlIndex)
        Column(
            Modifier.padding(inner).fillMaxSize().verticalScroll(rememberScrollState()).padding(vertical = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            if (current == null) {
                Text(
                    stringResource(R.string.web_no_address),
                    color = c.inkMuted,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(horizontal = 24.dp, vertical = 32.dp),
                )
            } else {
                QrImage(current)
                Text(
                    current,
                    color = c.ink,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp),
                )
                TextButton(onClick = { onCopy(current) }, colors = LanMsgButtons.text()) {
                    Text(stringResource(R.string.web_url_copy))
                }
            }
            if (urls.size > 1) {
                Column(Modifier.fillMaxWidth().background(c.surface).padding(top = 4.dp)) {
                    HorizontalDivider(color = c.line)
                    urls.forEachIndexed { i, u ->
                        Row(
                            Modifier.fillMaxWidth().clickable { onSelectUrl(i) }.padding(horizontal = 16.dp, vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(
                                selected = i == state.urlIndex,
                                onClick = null,
                                colors = RadioButtonDefaults.colors(selectedColor = c.signal, unselectedColor = c.inkMuted),
                            )
                            Text(
                                hostOf(u),
                                color = c.ink,
                                style = MaterialTheme.typography.bodyMedium,
                                modifier = Modifier.padding(start = 16.dp, top = 8.dp, bottom = 8.dp),
                            )
                        }
                    }
                    HorizontalDivider(color = c.line)
                }
            }
            Text(
                groupCode(info.code),
                color = c.ink,
                fontSize = 32.sp,
                fontWeight = FontWeight.SemiBold,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier.padding(top = 16.dp),
            )
            Text(
                stringResource(R.string.web_code_hint),
                color = c.inkMuted,
                style = MaterialTheme.typography.bodySmall,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            )

            Column(Modifier.fillMaxWidth()) {
                SectionHeader(stringResource(R.string.web_guests))
                Column(Modifier.fillMaxWidth().background(c.surface)) {
                    HorizontalDivider(color = c.line)
                    if (state.guests.isEmpty()) {
                        Text(
                            stringResource(R.string.web_guests_empty),
                            color = c.inkMuted,
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.padding(16.dp),
                        )
                    }
                    state.guests.forEach { g ->
                        GuestRow(g) { onOpenGuest(g.conv) }
                        HorizontalDivider(color = c.line)
                    }
                }
            }

            OutlinedButton(
                onClick = onStop,
                colors = LanMsgButtons.danger(),
                border = BorderStroke(1.dp, c.danger),
                modifier = Modifier.padding(top = 24.dp, bottom = 8.dp),
            ) { Text(stringResource(R.string.web_stop)) }
        }

        state.pending?.let { p ->
            AlertDialog(
                // 必须明确选择允许或拒绝，点外部不处理
                onDismissRequest = {},
                containerColor = c.surface,
                titleContentColor = c.ink,
                textContentColor = c.inkMuted,
                title = { Text(stringResource(R.string.web_request_title)) },
                text = { Text(stringResource(R.string.web_request_body, p.ip, browserSummary(p.userAgent))) },
                dismissButton = {
                    TextButton(onClick = { onRespond(false) }, colors = LanMsgButtons.text()) { Text(stringResource(R.string.web_deny)) }
                },
                confirmButton = { Button(onClick = { onRespond(true) }) { Text(stringResource(R.string.web_allow)) } },
            )
        }
    }
}

@Composable
private fun GuestRow(g: ConversationSummary, onClick: () -> Unit) {
    val c = LanMsgColors.current
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ConversationAvatar(g, size = 40.dp, online = g.peerOnline)
        Text(
            convTitle(g.kind, g.title),
            color = c.ink,
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.padding(start = 12.dp),
        )
    }
}

/** URL 的主机部分（"http://192.168.1.5:53320/#k=…" -> "192.168.1.5:53320"），选择行不显示密钥。 */
private fun hostOf(url: String): String = url.substringAfter("://").substringBefore('/')
