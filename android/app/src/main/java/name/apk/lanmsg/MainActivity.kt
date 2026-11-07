package name.apk.lanmsg

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.util.Log
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.navigation.compose.rememberNavController
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import name.apk.lanmsg.core.CoreHolder
import name.apk.lanmsg.core.call
import name.apk.lanmsg.files.ShareIntentParser
import name.apk.lanmsg.files.ShareText
import name.apk.lanmsg.ui.nav.LanMsgNavHost
import name.apk.lanmsg.ui.nav.Routes
import name.apk.lanmsg.ui.share.SharePickViewModel
import name.apk.lanmsg.ui.theme.LanMsgButtons
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgTheme
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SelfInfo

/** 外部分享带来的待发送内容。 */
private class ShareData(val uris: List<Uri>, val text: String?)

class MainActivity : ComponentActivity() {
    /** Activity 级：分享内容经它传给选择页。首次访问才创建，须在 core 就绪后。 */
    private val shareVm: SharePickViewModel by viewModels {
        viewModelFactory { initializer { SharePickViewModel(ServiceLocator.coreHolder.api, ServiceLocator.importer) } }
    }
    private var incomingShare by mutableStateOf<ShareData?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // 旋转等重建时不重复处理同一个分享 intent
        if (savedInstanceState == null) handleShare(intent)
        setContent {
            LanMsgTheme {
                Launcher(incomingShare, onShareConsumed = { incomingShare = null }, shareVm = { shareVm })
            }
        }
    }

    override fun onStart() {
        super.onStart()
        // UNDISPATCHED：owner 在主线程同步登记，保证 onStop 的 release 一定排在它之后；创建 core 在 IO 线程
        lifecycleScope.launch(Dispatchers.IO, start = CoroutineStart.UNDISPATCHED) {
            try {
                ServiceLocator.coreHolder.acquire(CoreHolder.UI)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // 创建失败只记日志；Launcher 在每次尝试（含“重试”）里会重新 acquire 并显示错误
                Log.w("MainActivity", "Failed to acquire core", e)
            }
        }
    }

    override fun onStop() {
        super.onStop()
        // 旋转等配置变更会 stop 再 start：此时放手会让“关闭”档位下的 core 白白重启
        if (!isChangingConfigurations) ServiceLocator.coreHolder.release(CoreHolder.UI)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleShare(intent)
    }

    private fun handleShare(intent: Intent) {
        val uris = ShareIntentParser.parse(intent)
        val text = ShareText.extract(intent)
        if (uris.isNotEmpty() || text != null) incomingShare = ShareData(uris, text)
    }
}

/**
 * 在 IO 线程先 acquire UI（同 owner 幂等；上次创建失败时这里会重新创建），再经代理 GetSelf。
 * 代理自己不创建 core，所以重试必须经由这里。
 */
internal suspend fun fetchSelf(holder: CoreHolder): SelfInfo = withContext(Dispatchers.IO) {
    holder.acquire(CoreHolder.UI)
    holder.api.call<Response.SelfInfo>(Request.GetSelf).value
}

/** 据 [fetchSelf] 的结果选起始路由；等待期间只显示主题背景，失败显示错误并可重试。 */
@Composable
private fun Launcher(incoming: ShareData?, onShareConsumed: () -> Unit, shareVm: () -> SharePickViewModel) {
    var start by remember { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    LaunchedEffect(attempt) {
        failure = null
        try {
            val self = fetchSelf(ServiceLocator.coreHolder)
            start = if (self.initialized) Routes.CONVS else Routes.ONBOARDING
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            failure = e.message ?: e.toString()
        }
    }
    val route = start
    val c = LanMsgColors.current
    when {
        route != null -> {
            val nav = rememberNavController()
            val ctx = LocalContext.current
            LaunchedEffect(incoming) {
                val data = incoming ?: return@LaunchedEffect
                onShareConsumed()
                if (route == Routes.ONBOARDING) {
                    // 未初始化：提示后留在引导页
                    Toast.makeText(ctx, R.string.share_need_setup, Toast.LENGTH_LONG).show()
                } else {
                    shareVm().setPending(data.uris, data.text)
                    nav.navigate(Routes.SHARE) { launchSingleTop = true }
                }
            }
            LanMsgNavHost(nav, route, shareVm)
        }
        failure != null -> Box(Modifier.fillMaxSize().background(c.canvas).safeDrawingPadding().padding(24.dp)) {
            Column {
                Text(stringResource(R.string.err_generic, failure.orEmpty()), color = c.danger)
                TextButton(onClick = { attempt++ }, colors = LanMsgButtons.text()) { Text(stringResource(R.string.retry)) }
            }
        }
        else -> Box(Modifier.fillMaxSize().background(c.canvas))
    }
}
