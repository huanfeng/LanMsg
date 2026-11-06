package name.apk.lanmsg.core

import android.content.Context
import android.util.Log
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_ffi.EventListener
import uniffi.lanmsg_ffi.FfiException
import uniffi.lanmsg_ffi.LanMsgCore
import java.io.File

private const val TAG = "FfiCoreApi"

/** 唯一（与 CoreHolder 一起）直接接触 UniFFI 生成对象的类。 */
class FfiCoreApi private constructor(private val core: LanMsgCore) : CoreApi {
    private val _events = MutableSharedFlow<Event>(replay = 0, extraBufferCapacity = 256)
    private val _lagged = MutableSharedFlow<Unit>(replay = 0, extraBufferCapacity = 1)

    override val events: SharedFlow<Event> = _events.asSharedFlow()
    override val lagged: SharedFlow<Unit> = _lagged.asSharedFlow()

    private val listener = object : EventListener {
        // UniFFI 对无返回值的外部回调抛异常时，Rust 侧转发任务会 panic 并静默终止，
        // 之后再也收不到任何事件，所以这里必须吞掉所有异常，只记日志。
        override fun onEvent(event: Event) {
            try {
                _events.tryEmit(event)
            } catch (t: Throwable) {
                Log.e(TAG, "onEvent 处理失败", t)
            }
        }

        override fun onLagged() {
            try {
                _lagged.tryEmit(Unit)
            } catch (t: Throwable) {
                Log.e(TAG, "onLagged 处理失败", t)
            }
        }
    }

    init {
        core.setEventListener(listener)
    }

    override suspend fun request(req: Request): Response = try {
        core.request(req)
    } catch (e: FfiException.Api) {
        // 生成代码里 message 即 Rust 端错误消息
        throw ApiException(e.code, e.message)
    }

    /** 阻塞调用，须在 IO 线程调用。先摘掉监听器，避免关闭过程中回调到已废弃的对象。 */
    override fun shutdown() {
        core.setEventListener(null)
        core.shutdown()
    }

    companion object {
        /** 必须在 IO 线程调用（会初始化 core 运行时并读写磁盘）。 */
        fun create(ctx: Context): FfiCoreApi {
            val dataDir = File(ctx.filesDir, "lanmsg")
            val keysDir = File(ctx.filesDir, "keys")
            val core = LanMsgCore(
                dataDir.absolutePath,
                AndroidKeyStore(keysDir, AndroidKeystoreWrapper()),
                AndroidPlatform(ctx, Thumbnailer(ctx)),
            )
            return FfiCoreApi(core)
        }
    }
}
