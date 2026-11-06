package name.apk.lanmsg.core

import android.util.Log
import kotlinx.coroutines.flow.MutableSharedFlow
import uniffi.lanmsg_api.Event

/**
 * 把 core 回调投递到 Flow。事件缓冲满而投递失败时，界面会停在旧状态，
 * 所以转为触发 lagged，让上层全量重拉。独立成类以便不依赖 LanMsgCore 做 JVM 测试。
 */
internal class EventPump(
    private val events: MutableSharedFlow<Event>,
    private val lagged: MutableSharedFlow<Unit>,
) {
    // UniFFI 对无返回值的外部回调抛异常时，Rust 侧转发任务会 panic 并静默终止，
    // 之后再也收不到任何事件，所以这里必须吞掉所有异常，只记日志。
    fun onEvent(event: Event) {
        try {
            if (!events.tryEmit(event)) {
                Log.w(TAG, "事件缓冲已满，丢弃事件并触发全量重拉")
                lagged.tryEmit(Unit)
            }
        } catch (t: Throwable) {
            Log.e(TAG, "onEvent 处理失败", t)
        }
    }

    fun onLagged() {
        try {
            lagged.tryEmit(Unit)
        } catch (t: Throwable) {
            Log.e(TAG, "onLagged 处理失败", t)
        }
    }

    private companion object {
        const val TAG = "EventPump"
    }
}
