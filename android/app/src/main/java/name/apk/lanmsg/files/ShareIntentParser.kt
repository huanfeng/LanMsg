package name.apk.lanmsg.files

import android.content.Intent
import android.net.Uri
import androidx.core.content.IntentCompat

object ShareIntentParser {
    /** ACTION_SEND 取单个 EXTRA_STREAM，SEND_MULTIPLE 取列表；其他返回空。 */
    fun parse(intent: Intent): List<Uri> = parseAll(intent).filter { it.scheme == "content" }

    /** 只放行 content://：file:// 会让本应用以自己的身份读取私有文件。 */
    private fun parseAll(intent: Intent): List<Uri> = when (intent.action) {
        Intent.ACTION_SEND -> listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
        Intent.ACTION_SEND_MULTIPLE ->
            IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
        else -> emptyList()
    }
}

object ShareText {
    /** ACTION_SEND 且没有 EXTRA_STREAM 时，返回 EXTRA_TEXT。 */
    fun extract(intent: Intent): String? {
        if (intent.action != Intent.ACTION_SEND) return null
        if (IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java) != null) return null
        return intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()?.takeIf { it.isNotBlank() }
    }
}
