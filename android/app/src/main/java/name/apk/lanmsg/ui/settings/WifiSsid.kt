package name.apk.lanmsg.ui.settings

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.wifi.WifiManager
import androidx.core.content.ContextCompat

/** 读取当前连接的 Wi-Fi 名称。需要 ACCESS_FINE_LOCATION；ViewModel 只接收这里返回的字符串。 */
object WifiSsid {
    /** 读取结果：区分「读到」「确实没连 Wi-Fi」「无权限或读取失败」。 */
    sealed interface Reading {
        data class Connected(val ssid: String) : Reading
        data object NotConnected : Reading
        data object Unavailable : Reading
    }

    @Suppress("DEPRECATION") // connectionInfo 在 API 31 起弃用，但仍是 minSdk 26 起统一可用的读取方式
    fun current(ctx: Context): Reading {
        if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED) {
            return Reading.Unavailable
        }
        val wifi = ctx.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager ?: return Reading.Unavailable
        return try {
            val info = wifi.connectionInfo
            val name = parse(info?.ssid)
            when {
                name != null -> Reading.Connected(name)
                // 未关联网络时 networkId 为 -1；已关联却读不到名称（如位置服务关闭）视为读取失败
                info == null || info.networkId == -1 -> Reading.NotConnected
                else -> Reading.Unavailable
            }
        } catch (e: SecurityException) {
            Reading.Unavailable
        }
    }

    /** 系统在读不到名称时返回的占位值（WifiManager.UNKNOWN_SSID 需 API 30，这里用字面量）。 */
    private const val UNKNOWN_SSID = "<unknown ssid>"

    /** 去掉系统加的引号；未知或空视为无。 */
    fun parse(raw: String?): String? {
        val name = raw?.removeSurrounding("\"")?.trim()
        return name?.takeIf { it.isNotEmpty() && it != UNKNOWN_SSID }
    }
}
