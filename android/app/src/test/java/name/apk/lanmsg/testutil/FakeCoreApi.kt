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
    val requests = mutableListOf<Request>()
    var handler: suspend (Request) -> Response = { throw ApiException(ErrorCode.INTERNAL, "fake 未配置") }
    val eventsFlow = MutableSharedFlow<Event>(extraBufferCapacity = 64)
    val laggedFlow = MutableSharedFlow<Unit>(extraBufferCapacity = 8)
    var shutdownCalled = false

    override val events: SharedFlow<Event> = eventsFlow
    override val lagged: SharedFlow<Unit> = laggedFlow

    override suspend fun request(req: Request): Response {
        requests += req
        return handler(req)
    }

    suspend fun emit(e: Event) = eventsFlow.emit(e)
    suspend fun emitLagged() = laggedFlow.emit(Unit)

    override fun shutdown() {
        shutdownCalled = true
    }
}
