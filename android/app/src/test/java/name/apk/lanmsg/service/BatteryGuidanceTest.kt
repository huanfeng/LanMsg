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
}
