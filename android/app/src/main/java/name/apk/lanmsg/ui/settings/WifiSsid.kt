package name.apk.lanmsg.ui.settings

import android.content.Context
import android.net.wifi.WifiManager

/** 读取当前连接的 Wi-Fi 名称。需要 ACCESS_FINE_LOCATION；ViewModel 只接收这里返回的字符串。 */
object WifiSsid {
    @Suppress("DEPRECATION") // connectionInfo 在 API 31 起弃用，但仍是 minSdk 26 起统一可用的读取方式
    fun current(ctx: Context): String? {
        val wifi = ctx.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager ?: return null
        return try {
            parse(wifi.connectionInfo?.ssid)
        } catch (e: SecurityException) {
            null
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
