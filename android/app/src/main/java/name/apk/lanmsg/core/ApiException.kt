package name.apk.lanmsg.core

import uniffi.lanmsg_api.ErrorCode

/** core 返回的业务错误；UI 层只处理这一种异常类型。 */
class ApiException(val code: ErrorCode, message: String) : Exception(message) {
    /** core 尚未实现该请求（ErrorCode.UNIMPLEMENTED）。 */
    fun isNotImplemented(): Boolean = code == ErrorCode.UNIMPLEMENTED
}
