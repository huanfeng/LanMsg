package name.apk.lanmsg.core

import android.content.Context
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

/** 唯一（与 CoreHolder 一起）直接接触 UniFFI 生成对象的类。 */
class FfiCoreApi private constructor(private val core: LanMsgCore) : CoreApi {
    private val _events = MutableSharedFlow<Event>(replay = 0, extraBufferCapacity = 256)
    private val _lagged = MutableSharedFlow<Unit>(replay = 0, extraBufferCapacity = 1)

    override val events: SharedFlow<Event> = _events.asSharedFlow()
    override val lagged: SharedFlow<Unit> = _lagged.asSharedFlow()

    private val pump = EventPump(_events, _lagged)

    private val listener = object : EventListener {
        override fun onEvent(event: Event) = pump.onEvent(event)

        override fun onLagged() = pump.onLagged()
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
