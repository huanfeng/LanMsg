package name.apk.lanmsg.notify

import kotlinx.coroutines.flow.MutableStateFlow

/**
 * 当前正在前台显示的会话 id（没有则为 null）。ChatViewModel.onVisible/onHidden 设置；
 * Task 12 的 Notifier 据此抑制通知，签名保持不变。
 */
object VisibleConversation {
    val current: MutableStateFlow<String?> = MutableStateFlow(null)
}
