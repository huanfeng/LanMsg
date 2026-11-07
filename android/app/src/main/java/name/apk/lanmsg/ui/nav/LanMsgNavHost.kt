package name.apk.lanmsg.ui.nav

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.padding
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavController
import androidx.navigation.NavHostController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.navArgument
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.chat.ChatScreen
import name.apk.lanmsg.ui.chat.ImageViewerScreen
import name.apk.lanmsg.ui.contacts.ContactDetailScreen
import name.apk.lanmsg.ui.contacts.ContactsScreen
import name.apk.lanmsg.ui.contacts.RequestsScreen
import name.apk.lanmsg.ui.devices.DevicesScreen
import name.apk.lanmsg.ui.devices.PairHostScreen
import name.apk.lanmsg.ui.devices.PairJoinScreen
import name.apk.lanmsg.ui.convs.ConversationListScreen
import name.apk.lanmsg.ui.files.FilesScreen
import name.apk.lanmsg.ui.forward.ForwardPickScreen
import name.apk.lanmsg.ui.me.MeScreen
import name.apk.lanmsg.ui.onboarding.OnboardingScreen
import name.apk.lanmsg.ui.onboarding.OnboardingViewModel
import name.apk.lanmsg.ui.search.SearchScreen
import name.apk.lanmsg.ui.settings.SettingsScreen
import name.apk.lanmsg.ui.share.SharePickScreen
import name.apk.lanmsg.ui.share.SharePickViewModel
import name.apk.lanmsg.ui.theme.LanMsgColors

private class Tab(val route: String, val label: Int, val icon: ImageVector)

private val tabs = listOf(
    Tab(Routes.CONVS, R.string.tab_messages, Icons.AutoMirrored.Outlined.Chat),
    Tab(Routes.FILES, R.string.tab_files, Icons.Outlined.Folder),
    Tab(Routes.CONTACTS, R.string.tab_contacts, Icons.Outlined.Group),
    Tab(Routes.ME, R.string.tab_me, Icons.Outlined.Person),
)

/**
 * 切换底部标签。返回栈的根固定是会话列表：首次使用时图的起点是引导页，完成引导后引导页已被弹出，
 * 不能用 findStartDestination() 作为 popUpTo 目标（目标不在栈里时 popUpTo 不生效，标签页会层层堆叠）。
 * 效果：任意标签之间切换后，返回键先回到「消息」，再退出应用；各标签内的子页面状态按标签保存与恢复。
 */
fun NavController.navigateToTab(route: String) {
    navigate(route.substringBefore('?')) {
        popUpTo(Routes.CONVS) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }
}

/** 主界面已建立（会话列表在返回栈中），即已完成引导；用于判断外部分享与深链能否进入。 */
fun NavController.hasMainRoot(): Boolean =
    try {
        getBackStackEntry(Routes.CONVS)
        true
    } catch (e: IllegalArgumentException) {
        false
    }

/**
 * 导航骨架。底部导航栏只在四个标签路由显示。
 *
 * 系统栏 insets 约定：外层 Scaffold 不消费状态栏 inset，只把底部导航栏高度（含导航栏 inset）
 * 作为 padding 传给内容并标记已消费；各页面自己用 Scaffold/TopAppBar 处理顶部状态栏，
 * 不要再给页面加 safeDrawingPadding 的底部部分。
 */
@Composable
fun LanMsgNavHost(nav: NavHostController, startDestination: String, shareVm: () -> SharePickViewModel) {
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
                                onClick = { nav.navigateToTab(tab.route) },
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
            composable(Routes.ONBOARDING) {
                OnboardingScreen { destination ->
                    when (destination) {
                        OnboardingViewModel.Destination.HOME ->
                            nav.navigate(Routes.CONVS) { popUpTo(Routes.ONBOARDING) { inclusive = true } }
                        OnboardingViewModel.Destination.PAIR_JOIN -> nav.navigate(Routes.PAIR_JOIN)
                    }
                }
            }
            composable(Routes.CONVS) { ConversationListScreen(
                    onOpenChat = { nav.navigate(Routes.chat(it)) },
                    onSearch = { nav.navigate(Routes.SEARCH) },
                ) }
            composable(
                Routes.FILES,
                arguments = listOf(navArgument("conv") { type = NavType.StringType; nullable = true; defaultValue = null }),
            ) { entry ->
                FilesScreen(
                    initialConv = entry.arguments?.getString("conv"),
                    onOpenChat = { nav.navigate(Routes.chat(it)) },
                    onForward = { nav.navigate(Routes.forward(it)) },
                )
            }
            composable(Routes.CONTACTS) {
                ContactsScreen(
                    onOpenRequests = { nav.navigate(Routes.REQUESTS) },
                    onOpenContact = { nav.navigate(Routes.contact(it)) },
                    onAddDevice = { nav.navigate(Routes.DEVICES) },
                )
            }
            composable(Routes.REQUESTS) { RequestsScreen(onBack = { nav.popBackStack() }) }
            composable(Routes.ME) {
                MeScreen(onOpenDevices = { nav.navigate(Routes.DEVICES) }, onOpenSettings = { nav.navigate(Routes.SETTINGS) })
            }
            composable(Routes.DEVICES) {
                DevicesScreen(
                    onBack = { nav.popBackStack() },
                    onAdd = { nav.navigate(Routes.PAIR_HOST) },
                    onJoin = { nav.navigate(Routes.PAIR_JOIN) },
                )
            }
            composable(Routes.PAIR_HOST) { PairHostScreen(onBack = { nav.popBackStack() }, onDone = { nav.popBackStack() }) }
            composable(Routes.PAIR_JOIN) {
                PairJoinScreen(
                    onBack = { nav.popBackStack() },
                    onDone = {
                        // 引导流程中加入已有设备：配对完成后进入主界面并清掉引导页与配对页
                        if (nav.previousBackStackEntry?.destination?.route == Routes.ONBOARDING) {
                            nav.navigate(Routes.CONVS) { popUpTo(Routes.ONBOARDING) { inclusive = true } }
                        } else {
                            nav.popBackStack()
                        }
                    },
                )
            }
            composable(Routes.SETTINGS) { SettingsScreen(onBack = { nav.popBackStack() }) }
            composable(Routes.SHARE) {
                val vm = shareVm()
                SharePickScreen(
                    vm,
                    onBack = { vm.reset(); nav.popBackStack() },
                    onSent = { nav.navigate(Routes.chat(it)) { popUpTo(Routes.SHARE) { inclusive = true } } },
                )
            }
            composable(Routes.SEARCH) {
                SearchScreen(onBack = { nav.popBackStack() }, onOpenChat = { nav.navigate(Routes.chat(it)) })
            }
            composable(Routes.FORWARD, arguments = listOf(navArgument("hashes") { type = NavType.StringType })) { entry ->
                val hashes = entry.arguments?.getString("hashes").orEmpty().split(',').filter { it.isNotEmpty() }
                ForwardPickScreen(
                    hashes = hashes,
                    onBack = { nav.popBackStack() },
                    onDone = { done ->
                        // 从目标会话自身转发：直接退回，避免返回栈里出现两个相同的聊天页
                        val prev = nav.previousBackStackEntry
                        if (prev?.destination?.route == Routes.CHAT && prev.arguments?.getString("conv") == done) {
                            nav.popBackStack()
                        } else {
                            nav.navigate(Routes.chat(done)) {
                                popUpTo(Routes.FORWARD) { inclusive = true }
                                launchSingleTop = true
                            }
                        }
                    },
                )
            }
            composable(Routes.CHAT, arguments = listOf(navArgument("conv") { type = NavType.StringType })) { entry ->
                val conv = entry.arguments?.getString("conv") ?: return@composable
                ChatScreen(
                    conv = conv,
                    onBack = { nav.popBackStack() },
                    onOpenImage = { c, hash -> nav.navigate(Routes.image(c, hash)) },
                    onForward = { nav.navigate(Routes.forward(it)) },
                )
            }
            composable(
                Routes.IMAGE,
                arguments = listOf(
                    navArgument("conv") { type = NavType.StringType },
                    navArgument("hash") { type = NavType.StringType },
                ),
            ) { entry ->
                val conv = entry.arguments?.getString("conv") ?: return@composable
                val hash = entry.arguments?.getString("hash") ?: return@composable
                ImageViewerScreen(conv, hash, onBack = { nav.popBackStack() })
            }
            composable(Routes.CONTACT, arguments = listOf(navArgument("user") { type = NavType.StringType })) { entry ->
                val user = entry.arguments?.getString("user") ?: return@composable
                ContactDetailScreen(
                    user = user,
                    onBack = { nav.popBackStack() },
                    onOpenChat = { nav.navigate(Routes.chat(it)) },
                )
            }
        }
    }
}
