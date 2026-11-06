package name.apk.lanmsg.ui.common

import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/** 展示格式，规则见 docs/ui-architecture.md §3。 */
object Format {
    private val hhmm = DateTimeFormatter.ofPattern("HH:mm")

    // "M月d日"；中文用 Unicode 转义，避免 Kotlin 代码里出现中文字面量。
    private val monthDay = DateTimeFormatter.ofPattern("M月d日")
    private val full = DateTimeFormatter.ofPattern("yyyy/M/d")

    /** 今天 HH:mm；昨天由 [yesterday] 拼接（来自 time_yesterday）；今年 M月d日；更早 yyyy/M/d。 */
    fun time(
        millis: Long,
        now: Long,
        zone: ZoneId = ZoneId.systemDefault(),
        yesterday: (String) -> String,
    ): String {
        val t = Instant.ofEpochMilli(millis).atZone(zone)
        val today = Instant.ofEpochMilli(now).atZone(zone).toLocalDate()
        val day = t.toLocalDate()
        return when {
            day == today -> hhmm.format(t)
            day == today.minusDays(1) -> yesterday(hhmm.format(t))
            day.year == today.year -> monthDay.format(t)
            else -> full.format(t)
        }
    }

    /** `N B` / `x.x KB` / `x.x MB` / `x.x GB`，一位小数并去掉 `.0`。 */
    fun size(bytes: Long): String {
        if (bytes < 1024) return "$bytes B"
        val units = listOf("KB", "MB", "GB")
        var value = bytes / 1024.0
        var i = 0
        while (value >= 1024 && i < units.lastIndex) {
            value /= 1024
            i++
        }
        return String.format(Locale.ROOT, "%.1f", value).removeSuffix(".0") + " " + units[i]
    }

    /** `m:ss`，超过 1 小时 `h:mm:ss`。 */
    fun duration(ms: Long): String {
        val total = ms / 1000
        val h = total / 3600
        val m = total % 3600 / 60
        val s = total % 60
        return if (h > 0) {
            String.format(Locale.ROOT, "%d:%02d:%02d", h, m, s)
        } else {
            String.format(Locale.ROOT, "%d:%02d", m, s)
        }
    }

    /** djb2（u32 回绕）% 6，与桌面 style.rs 的 avatar_color 同一算法，见 docs/ui-architecture.md §3。 */
    fun avatarColorIndex(id: String): Int {
        var h = 5381
        for (b in id.toByteArray(Charsets.UTF_8)) h = h * 33 + (b.toInt() and 0xFF) // Int 回绕等价 u32
        return ((h.toLong() and 0xFFFFFFFFL) % 6).toInt()
    }
}
