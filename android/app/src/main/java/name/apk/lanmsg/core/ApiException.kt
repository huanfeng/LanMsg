package name.apk.lanmsg.core

import uniffi.lanmsg_api.ErrorCode

/** core 返回的业务错误；UI 层只处理这一种异常类型。 */
class ApiException(val code: ErrorCode, message: String) : Exception(message) {
    /** 对应 P1 Task 11 的统一占位错误（功能尚未在 core 实现）。 */
    fun isNotImplemented(): Boolean = code == ErrorCode.INTERNAL && (message ?: "").startsWith("未实现：")
}
