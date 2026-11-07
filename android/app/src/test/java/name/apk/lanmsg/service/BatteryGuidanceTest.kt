package name.apk.lanmsg.service

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.lanmsg_api.BackgroundMode

class BatteryGuidanceTest {
    @Test fun card_shows_only_in_realtime_when_not_exempt() {
        assertTrue(BatteryGuidance.shouldShowCard(BackgroundMode.REALTIME, false))
        assertFalse(BatteryGuidance.shouldShowCard(BackgroundMode.REALTIME, true))
        assertFalse(BatteryGuidance.shouldShowCard(BackgroundMode.POWER_SAVING, false))
        assertFalse(BatteryGuidance.shouldShowCard(BackgroundMode.OFF, false))
    }

    @Test fun xiaomi_family_detected_case_insensitively() {
        for (m in listOf("Xiaomi", "xiaomi", "REDMI", "Redmi", "POCO", " poco ")) {
            assertEquals(m, BatteryGuidance.Vendor.XIAOMI, BatteryGuidance.vendorOf(m))
        }
    }

    @Test fun other_manufacturers_have_no_vendor_guidance() {
        for (m in listOf("Google", "samsung", "", null)) assertNull(m, BatteryGuidance.vendorOf(m))
    }

    @Test fun legacy_entry_is_exclusive_with_card() {
        for (mode in BackgroundMode.entries) for (ignoring in listOf(true, false)) {
            assertEquals(
                "$mode/$ignoring",
                !BatteryGuidance.shouldShowCard(mode, ignoring),
                BatteryGuidance.showLegacyEntry(mode, ignoring),
            )
        }
    }

    @Test fun vendor_hint_collapses_only_after_seen_and_until_expanded() {
        assertFalse(BatteryGuidance.vendorHintCollapsed(seen = false, expanded = false))
        assertTrue(BatteryGuidance.vendorHintCollapsed(seen = true, expanded = false))
        assertFalse(BatteryGuidance.vendorHintCollapsed(seen = true, expanded = true))
    }
}
