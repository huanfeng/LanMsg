package name.apk.lanmsg.service

import uniffi.lanmsg_api.BackgroundMode

/** 电池优化与厂商省电引导的判定。纯函数。 */
object BatteryGuidance {
    /** 实时档且系统仍在对本应用做电池优化时，才显示提示卡。 */
    fun shouldShowCard(mode: BackgroundMode, isIgnoringBatteryOptimizations: Boolean): Boolean =
        mode == BackgroundMode.REALTIME && !isIgnoringBatteryOptimizations

    /** 需要额外厂商指引的系统；以后扩展 OPPO、vivo、华为时在此加枚举值与判断。 */
    enum class Vendor { XIAOMI }

    /** 按 Build.MANUFACTURER 判断厂商，不区分大小写；无特殊指引返回 null。 */
    fun vendorOf(manufacturer: String?): Vendor? = when (manufacturer?.trim()?.lowercase()) {
        "xiaomi", "redmi", "poco" -> Vendor.XIAOMI
        else -> null
    }

    /** 原“允许后台运行”入口与提示卡互斥：卡片显示时隐藏它，卡片不显示时保留。 */
    fun showLegacyEntry(mode: BackgroundMode, isIgnoringBatteryOptimizations: Boolean): Boolean =
        !shouldShowCard(mode, isIgnoringBatteryOptimizations)

    /** 点过“打开应用详情”后，厂商指引折叠成一行；用户展开后恢复完整显示。 */
    fun vendorHintCollapsed(seen: Boolean, expanded: Boolean): Boolean = seen && !expanded
}
