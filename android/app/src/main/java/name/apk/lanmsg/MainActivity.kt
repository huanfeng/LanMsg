package name.apk.lanmsg

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.navigation.compose.rememberNavController
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import name.apk.lanmsg.core.call
import name.apk.lanmsg.ui.nav.LanMsgNavHost
import name.apk.lanmsg.ui.nav.Routes
import name.apk.lanmsg.ui.theme.LanMsgColors
import name.apk.lanmsg.ui.theme.LanMsgTheme
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent { LanMsgTheme { Launcher() } }
    }
}

/** 先在 IO 线程创建 core 并 GetSelf，据此选起始路由；等待期间只显示主题背景。 */
@Composable
private fun Launcher() {
    var start by remember { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        try {
            val self = withContext(Dispatchers.IO) {
                ServiceLocator.core.call<Response.SelfInfo>(Request.GetSelf).value
            }
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
        route != null -> LanMsgNavHost(rememberNavController(), route)
        failure != null -> Box(Modifier.fillMaxSize().background(c.canvas).safeDrawingPadding().padding(24.dp)) {
            Text(stringResource(R.string.err_generic, failure.orEmpty()), color = c.danger)
        }
        else -> Box(Modifier.fillMaxSize().background(c.canvas))
    }
}
