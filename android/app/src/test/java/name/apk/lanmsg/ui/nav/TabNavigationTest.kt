package name.apk.lanmsg.ui.nav

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** 与真实图相同的路由形状，页面内容为空；只验证返回栈。 */
@RunWith(AndroidJUnit4::class)
class TabNavigationTest {
    @get:Rule val rule = createComposeRule()

    private lateinit var nav: NavHostController

    private fun setUp(start: String) {
        rule.setContent {
            nav = rememberNavController()
            NavHost(nav, startDestination = start) {
                composable(Routes.ONBOARDING) {}
                composable(Routes.CONVS) {}
                composable(Routes.FILES) {}
                composable(Routes.CONTACTS) {}
                composable(Routes.ME) {}
                composable(Routes.DEVICES) {}
                composable(Routes.CHAT) {}
            }
        }
        rule.waitForIdle()
    }

    private fun stack(): List<String?> =
        rule.runOnIdle { nav.currentBackStack.value.mapNotNull { it.destination.route }.filter { it.isNotEmpty() && it != nav.graph.route } }

    private fun act(block: NavHostController.() -> Unit) {
        rule.runOnIdle { nav.block() }
        rule.waitForIdle()
    }

    @Test fun switching_tabs_after_onboarding_keeps_single_root() {
        setUp(Routes.ONBOARDING)
        assertFalse(rule.runOnIdle { nav.hasMainRoot() })
        act { navigate(Routes.CONVS) { popUpTo(Routes.ONBOARDING) { inclusive = true } } }
        assertTrue(rule.runOnIdle { nav.hasMainRoot() })

        act { navigateToTab(Routes.FILES) }
        act { navigateToTab(Routes.CONTACTS) }
        act { navigateToTab(Routes.ME) }
        act { navigateToTab(Routes.FILES) }
        assertEquals(listOf(Routes.CONVS, Routes.FILES), stack())

        // 返回一次回到消息，再返回就退出（栈里只剩根）
        act { popBackStack() }
        assertEquals(listOf(Routes.CONVS), stack())
    }

    @Test fun switching_tabs_on_initialized_start_keeps_single_root() {
        setUp(Routes.CONVS)
        act { navigateToTab(Routes.ME) }
        act { navigateToTab(Routes.CONTACTS) }
        act { navigateToTab(Routes.CONVS) }
        assertEquals(listOf(Routes.CONVS), stack())
    }

    @Test fun tab_substack_is_restored_when_returning_to_tab() {
        setUp(Routes.CONVS)
        act { navigateToTab(Routes.ME) }
        act { navigate(Routes.DEVICES) }
        act { navigateToTab(Routes.FILES) }
        assertEquals(listOf(Routes.CONVS, Routes.FILES), stack())
        act { navigateToTab(Routes.ME) }
        assertEquals(listOf(Routes.CONVS, Routes.ME, Routes.DEVICES), stack())
    }
}
