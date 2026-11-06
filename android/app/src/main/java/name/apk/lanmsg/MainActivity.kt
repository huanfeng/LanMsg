package name.apk.lanmsg

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.navigation.compose.rememberNavController
import name.apk.lanmsg.ui.nav.LanMsgNavHost
import name.apk.lanmsg.ui.nav.Routes
import name.apk.lanmsg.ui.theme.LanMsgTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            // 起始路由本任务固定为会话列表，Task 6 改为根据 GetSelf 决定。
            LanMsgTheme { LanMsgNavHost(rememberNavController(), Routes.CONVS) }
        }
    }
}
