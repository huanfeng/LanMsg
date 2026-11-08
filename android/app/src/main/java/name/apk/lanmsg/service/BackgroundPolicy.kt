package name.apk.lanmsg.service

import uniffi.lanmsg_api.BackgroundMode

data class Conditions(
    val appVisible: Boolean,
    val screenOn: Boolean,
    val charging: Boolean,
    val onTrustedWifi: Boolean,
    /** 正在托管网页传输：浏览器随时可能连入，必须保持前台服务。 */
    val webHosting: Boolean = false,
)

/** 后台档位 + 设备条件 -> 是否保持前台服务 / 是否排周期同步任务。纯函数。 */
object BackgroundPolicy {
    fun shouldRunService(mode: BackgroundMode, c: Conditions): Boolean = c.webHosting || when (mode) {
        BackgroundMode.REALTIME -> true
        BackgroundMode.POWER_SAVING -> c.appVisible || c.screenOn || c.charging || c.onTrustedWifi
        BackgroundMode.OFF -> false
    }

    fun shouldScheduleWorker(mode: BackgroundMode): Boolean = mode == BackgroundMode.POWER_SAVING
}
