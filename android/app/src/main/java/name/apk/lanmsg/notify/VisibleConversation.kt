package name.apk.lanmsg.notify

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine

/**
 * 用户当前正在看的会话。[current] 由 ChatViewModel.onVisible/onHidden 设置（签名不变）；
 * 图片查看页覆盖在聊天页之上时，聊天页会先被移除并清掉 [current]，所以查看页另外用 [pin]/[unpin]
 * 登记计数，二者互不覆盖（页面切换动画期间新旧页面的生命周期回调顺序不确定，不能共用一个值）。
 */
object VisibleConversation {
    val current: MutableStateFlow<String?> = MutableStateFlow(null)

    private val pinned = MutableStateFlow<List<String>>(emptyList())

    /** 登记 [conv] 在某个覆盖页下仍视为可见；可重复登记，需与 [unpin] 成对。 */
    fun pin(conv: String) {
        pinned.value = pinned.value + conv
    }

    fun unpin(conv: String) {
        val list = pinned.value
        val i = list.indexOf(conv)
        if (i >= 0) pinned.value = list.toMutableList().also { it.removeAt(i) }
    }

    fun isVisible(conv: String): Boolean = current.value == conv || conv in pinned.value

    /** 当前可见的会话集合，变化时发射。 */
    val visible: Flow<Set<String>> = combine(current, pinned) { c, p -> p.toSet() + listOfNotNull(c) }
}
