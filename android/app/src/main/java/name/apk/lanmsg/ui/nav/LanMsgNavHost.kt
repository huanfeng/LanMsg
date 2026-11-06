package name.apk.lanmsg.ui.nav

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Chat
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Group
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavHostController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.navArgument
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.theme.LanMsgColors

private class Tab(val route: String, val label: Int, val icon: ImageVector)

private val tabs = listOf(
    Tab(Routes.CONVS, R.string.tab_messages, Icons.AutoMirrored.Outlined.Chat),
    Tab(Routes.FILES, R.string.tab_files, Icons.Outlined.Folder),
    Tab(Routes.CONTACTS, R.string.tab_contacts, Icons.Outlined.Group),
    Tab(Routes.ME, R.string.tab_me, Icons.Outlined.Person),
)

/**
 * 导航骨架。底部导航栏只在四个标签路由显示。
 *
 * 系统栏 insets 约定：外层 Scaffold 不消费状态栏 inset，只把底部导航栏高度（含导航栏 inset）
 * 作为 padding 传给内容并标记已消费；各页面自己用 Scaffold/TopAppBar 处理顶部状态栏，
 * 不要再给页面加 safeDrawingPadding 的底部部分。
 */
@Composable
fun LanMsgNavHost(nav: NavHostController, startDestination: String) {
    val entry by nav.currentBackStackEntryAsState()
    val destination = entry?.destination
    val showBar = tabs.any { tab -> destination?.hierarchy?.any { it.route == tab.route } == true }
    val c = LanMsgColors.current

    Scaffold(
        containerColor = c.canvas,
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        bottomBar = {
            if (showBar) {
                Column {
                    HorizontalDivider(color = c.line)
                    NavigationBar(containerColor = c.surface, tonalElevation = 0.dp) {
                        for (tab in tabs) {
                            NavigationBarItem(
                                selected = destination?.hierarchy?.any { it.route == tab.route } == true,
                                onClick = {
                                    nav.navigate(tab.route.substringBefore('?')) {
                                        popUpTo(nav.graph.findStartDestination().id) { saveState = true }
                                        launchSingleTop = true
                                        restoreState = true
                                    }
                                },
                                icon = { Icon(tab.icon, contentDescription = null) },
                                label = { Text(stringResource(tab.label)) },
                                colors = NavigationBarItemDefaults.colors(
                                    selectedIconColor = c.signal,
                                    selectedTextColor = c.signal,
                                    indicatorColor = c.signalSoft,
                                    unselectedIconColor = c.inkMuted,
                                    unselectedTextColor = c.inkMuted,
                                ),
                            )
                        }
                    }
                }
            }
        },
    ) { inner ->
        NavHost(
            navController = nav,
            startDestination = startDestination,
            modifier = Modifier.padding(inner).consumeWindowInsets(inner),
        ) {
            composable(Routes.ONBOARDING) { Placeholder(Routes.ONBOARDING) }
            composable(Routes.CONVS) { Placeholder(Routes.CONVS) }
            composable(
                Routes.FILES,
                arguments = listOf(navArgument("conv") { type = NavType.StringType; nullable = true; defaultValue = null }),
            ) { Placeholder(Routes.FILES) }
            composable(Routes.CONTACTS) { Placeholder(Routes.CONTACTS) }
            composable(Routes.REQUESTS) { Placeholder(Routes.REQUESTS) }
            composable(Routes.ME) { Placeholder(Routes.ME) }
            composable(Routes.DEVICES) { Placeholder(Routes.DEVICES) }
            composable(Routes.PAIR_HOST) { Placeholder(Routes.PAIR_HOST) }
            composable(Routes.PAIR_JOIN) { Placeholder(Routes.PAIR_JOIN) }
            composable(Routes.SETTINGS) { Placeholder(Routes.SETTINGS) }
            composable(Routes.SHARE) { Placeholder(Routes.SHARE) }
            composable(Routes.SEARCH) { Placeholder(Routes.SEARCH) }
            composable(Routes.FORWARD) { Placeholder(Routes.FORWARD) }
            composable(Routes.CHAT) { Placeholder(Routes.CHAT) }
            composable(Routes.IMAGE) { Placeholder(Routes.IMAGE) }
            composable(Routes.CONTACT) { Placeholder(Routes.CONTACT) }
        }
    }
}

// 占位页：后续任务逐个把上面的 composable 换成真实页面；阶段验收时全文搜索 Placeholder( 必须为 0 处调用。
@Composable
private fun Placeholder(route: String) {
    Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
        Text(route, color = LanMsgColors.current.ink)
    }
}
