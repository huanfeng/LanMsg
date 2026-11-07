package name.apk.lanmsg.service

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.lanmsg_api.BackgroundMode

class BackgroundPolicyTest {
    private val off = Conditions(appVisible = false, screenOn = false, charging = false, onTrustedWifi = false)

    @Test fun realtime_always_runs() {
        assertTrue(BackgroundPolicy.shouldRunService(BackgroundMode.REALTIME, off))
    }

    @Test fun power_saving_runs_only_with_a_condition() {
        assertFalse(BackgroundPolicy.shouldRunService(BackgroundMode.POWER_SAVING, off))
        assertTrue(BackgroundPolicy.shouldRunService(BackgroundMode.POWER_SAVING, off.copy(charging = true)))
        assertTrue(BackgroundPolicy.shouldRunService(BackgroundMode.POWER_SAVING, off.copy(onTrustedWifi = true)))
        assertTrue(BackgroundPolicy.shouldRunService(BackgroundMode.POWER_SAVING, off.copy(appVisible = true)))
        assertTrue(BackgroundPolicy.shouldRunService(BackgroundMode.POWER_SAVING, off.copy(screenOn = true)))
    }

    @Test fun off_never_runs_service() {
        val all = Conditions(appVisible = true, screenOn = true, charging = true, onTrustedWifi = true)
        assertFalse(BackgroundPolicy.shouldRunService(BackgroundMode.OFF, all))
    }

    @Test fun worker_only_for_power_saving() {
        assertFalse(BackgroundPolicy.shouldScheduleWorker(BackgroundMode.REALTIME))
        assertTrue(BackgroundPolicy.shouldScheduleWorker(BackgroundMode.POWER_SAVING))
        assertFalse(BackgroundPolicy.shouldScheduleWorker(BackgroundMode.OFF))
    }
}
