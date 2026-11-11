package name.apk.lanmsg.testutil

import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.core.CoreApi
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class FakeCoreApi : CoreApi {
    // request() 在服务后台线程追加、测试线程遍历断言，跨线程无同步——
    // mutableListOf 会抛 ConcurrentModificationException，用 CopyOnWriteArrayList。
    val requests = java.util.concurrent.CopyOnWriteArrayList<Request>()
    var handler: suspend (Request) -> Response = { throw ApiException(ErrorCode.INTERNAL, "fake not configured") }
    val eventsFlow = MutableSharedFlow<Event>(extraBufferCapacity = 64)
    val laggedFlow = MutableSharedFlow<Unit>(extraBufferCapacity = 8)
    var shutdownCount = 0
    val shutdownCalled: Boolean get() = shutdownCount > 0

    override val events: SharedFlow<Event> = eventsFlow
    override val lagged: SharedFlow<Unit> = laggedFlow

    override suspend fun request(req: Request): Response {
        requests += req
        return handler(req)
    }

    suspend fun emit(e: Event) = eventsFlow.emit(e)
    suspend fun emitLagged() = laggedFlow.emit(Unit)

    override fun shutdown() {
        shutdownCount++
    }
}
