package name.apk.lanmsg.core

import kotlinx.coroutines.flow.SharedFlow
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

/** ViewModel 与界面访问 core 的唯一入口；测试中用 FakeCoreApi 替换。 */
interface CoreApi {
    /** 失败抛 [ApiException]。 */
    suspend fun request(req: Request): Response

    /** replay = 0，extraBufferCapacity = 256。 */
    val events: SharedFlow<Event>

    /** 收到即全量重拉。 */
    val lagged: SharedFlow<Unit>

    fun shutdown()
}

/** 发送请求并断言响应类型，类型不符抛 [ApiException]。 */
suspend inline fun <reified T : Response> CoreApi.call(req: Request): T {
    val resp = request(req)
    return resp as? T ?: throw ApiException(ErrorCode.INTERNAL, "响应类型不符")
}
