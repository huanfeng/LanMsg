package name.apk.lanmsg.ui.common

import java.time.LocalDateTime
import java.time.ZoneId
import org.junit.Assert.assertEquals
import org.junit.Test

class FormatTest {
    private val zone = ZoneId.of("Asia/Shanghai")
    private fun ms(s: String) = LocalDateTime.parse(s).atZone(zone).toInstant().toEpochMilli()
    private val y: (String) -> String = { "昨天 $it" }

    @Test fun time_today_yesterday_this_year_older() {
        val now = ms("2026-10-28T15:00:00")
        assertEquals("09:05", Format.time(ms("2026-10-28T09:05:00"), now, zone, y))
        assertEquals("昨天 23:59", Format.time(ms("2026-10-27T23:59:00"), now, zone, y))
        assertEquals("3月1日", Format.time(ms("2026-03-01T10:00:00"), now, zone, y))
        assertEquals("2025/12/31", Format.time(ms("2025-12-31T10:00:00"), now, zone, y))
    }

    @Test fun size_units() {
        assertEquals("512 B", Format.size(512)); assertEquals("1 KB", Format.size(1024))
        assertEquals("1.5 MB", Format.size(1572864)); assertEquals("2 GB", Format.size(2L shl 30))
    }

    @Test fun duration_formats() {
        assertEquals("0:07", Format.duration(7000)); assertEquals("1:02:03", Format.duration(3723000))
    }

    // 期望值由独立脚本按 djb2(u32 回绕)%6 算出，与桌面 style.rs 同一输入同一下标。
    @Test fun avatar_color_index_matches_desktop_djb2() {
        assertEquals(0, Format.avatarColorIndex("c-dm"))
        assertEquals(djb2Reference("0123456789abcdef".repeat(4)), Format.avatarColorIndex("0123456789abcdef".repeat(4)))
        assertEquals(djb2Reference("张三"), Format.avatarColorIndex("张三"))
    }

    // 用 Long 掩码实现的参考版本，覆盖多字节 UTF-8 与长串回绕。
    private fun djb2Reference(s: String): Int {
        var h = 5381L
        for (b in s.toByteArray(Charsets.UTF_8)) h = (h * 33 + (b.toLong() and 0xFF)) and 0xFFFFFFFFL
        return (h % 6).toInt()
    }
}
