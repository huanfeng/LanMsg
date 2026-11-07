package name.apk.lanmsg

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.net.Uri
import android.os.Bundle
import android.util.Log
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.navigation.NavController
import androidx.navigation.compose.rememberNavController
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
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

    /** 通知点击带来的待导航路由；Launcher 初始化完成后消费。 */
    private var pendingRoute by mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // 旋转等重建时不重复处理同一个分享 intent
        if (savedInstanceState == null) {
            handleShare(intent)
            pendingRoute = intent.getStringExtra(EXTRA_ROUTE)
        }
        setContent {
            LanMsgTheme {
                Launcher(
                    incomingShare, onShareConsumed = { incomingShare = null }, shareVm = { shareVm },
                    pendingRoute, onRouteConsumed = { pendingRoute = null },
                )
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
        intent.getStringExtra(EXTRA_ROUTE)?.let { pendingRoute = it }
    }

    private fun handleShare(intent: Intent) {
        val uris = ShareIntentParser.parse(intent)
        val text = ShareText.extract(intent)
        if (uris.isNotEmpty() || text != null) incomingShare = ShareData(uris, text)
    }

    companion object {
        /** 通知点击携带的目标路由（Routes 中的值）。 */
        const val EXTRA_ROUTE = "route"
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

/**
 * 只在前台（STARTED）期间获取：onStop 会 release("ui")，后台里的在途 acquire 会被取消，
 * 也不会在后台重新登记 "ui"；下次进入前台时若还没有结果自动重试。
 * 非生命周期取消的 [CancellationException]（如创建完成前 owner 被放手）按失败处理，避免界面空白。
 */
internal suspend fun fetchSelfWhileStarted(
    lifecycle: Lifecycle,
    holder: CoreHolder,
    hasResult: () -> Boolean,
    onStart: () -> Unit,
    onSelf: (SelfInfo) -> Unit,
    onFailure: (String) -> Unit,
) {
    lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
        if (hasResult()) return@repeatOnLifecycle
        onStart()
        try {
            onSelf(fetchSelf(holder))
        } catch (e: CancellationException) {
            if (!currentCoroutineContext().isActive) throw e
            onFailure(e.message ?: e.toString())
        } catch (e: Exception) {
            onFailure(e.message ?: e.toString())
        }
    }
}

/** Android 13+：第一次进入主界面（会话列表）时申请通知权限，只问一次，拒绝后不再弹。 */
@Composable
private fun NotificationPermissionOnFirstMain(nav: NavController) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
    val ctx = LocalContext.current
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
    LaunchedEffect(nav) {
        nav.currentBackStackEntryFlow.first { it.destination.route == Routes.CONVS }
        val granted = ContextCompat.checkSelfPermission(ctx, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
        if (!granted && !ServiceLocator.localPrefs.notificationPermissionAsked.first()) {
            ServiceLocator.localPrefs.setNotificationPermissionAsked()
            launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
}

/** 据 [fetchSelf] 的结果选起始路由；等待期间只显示主题背景，失败显示错误并可重试。 */
@Composable
private fun Launcher(
    incoming: ShareData?,
    onShareConsumed: () -> Unit,
    shareVm: () -> SharePickViewModel,
    deepLink: String?,
    onRouteConsumed: () -> Unit,
) {
    var start by remember { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(attempt) {
        fetchSelfWhileStarted(
            lifecycle, ServiceLocator.coreHolder,
            hasResult = { start != null },
            onStart = { failure = null },
            onSelf = { start = if (it.initialized) Routes.CONVS else Routes.ONBOARDING },
            onFailure = { failure = it },
        )
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
            LaunchedEffect(deepLink) {
                val target = deepLink ?: return@LaunchedEffect
                onRouteConsumed()
                // 未初始化时没有可去的页面；路由来自外部 intent，无效时不能崩溃
                if (route != Routes.ONBOARDING) {
                    try {
                        nav.navigate(target) { launchSingleTop = true }
                    } catch (e: IllegalArgumentException) {
                        Log.w("MainActivity", "Ignoring unknown route: $target", e)
                    }
                }
            }
            NotificationPermissionOnFirstMain(nav)
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
