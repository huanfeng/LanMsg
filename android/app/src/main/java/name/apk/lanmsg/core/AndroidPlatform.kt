package name.apk.lanmsg.core

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import uniffi.lanmsg_ffi.FfiPlatform
import uniffi.lanmsg_ffi.ThumbnailResult

/** core 需要的平台能力：网络状态与缩略图。 */
class AndroidPlatform(ctx: Context, private val thumbnailer: Thumbnailer) : FfiPlatform {
    private val cm = ctx.applicationContext.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

    override fun isMetered(): Boolean = cm.isActiveNetworkMetered

    override fun isWifi(): Boolean {
        val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
    }

    override fun thumbnail(path: String, mime: String, maxEdge: UInt): ThumbnailResult? =
        thumbnailer.thumbnail(path, mime, maxEdge.toInt())
}
