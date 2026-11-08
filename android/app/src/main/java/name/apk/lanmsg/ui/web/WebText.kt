package name.apk.lanmsg.ui.web

private val OS_NAMES = listOf(
    "Android" to "Android", "iPhone" to "iPhone", "Windows" to "Windows", "Mac OS X" to "macOS", "Linux" to "Linux",
)

/** 8 位配对码分成 4+4 两组便于口述；其他长度原样返回。 */
fun groupCode(code: String): String = if (code.length == 8) "${code.substring(0, 4)} ${code.substring(4)}" else code

/**
 * 浏览器 User-Agent 的简短描述，如 "Chrome (Android)"。
 * 浏览器与系统名是专有名词，不随语言变化；无法识别浏览器时取前 40 个字符。
 */
fun browserSummary(ua: String): String {
    val browser = when {
        "Edg/" in ua -> "Edge"
        "Chrome/" in ua -> "Chrome"
        "Firefox/" in ua -> "Firefox"
        "Safari/" in ua -> "Safari"
        else -> ua.take(40)
    }
    // Android 与 iPhone 的 UA 里分别含 "Linux" 与 "Mac OS X"，所以移动系统排在前面
    val os = OS_NAMES.firstOrNull { it.first in ua }?.second
    return if (os == null) browser else "$browser ($os)"
}
