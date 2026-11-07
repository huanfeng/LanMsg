package name.apk.lanmsg.service

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.lanmsg_api.BackgroundMode

class BootReceiverTest {
    private class Actions : ServiceActions {
        val service = mutableListOf<Boolean>()

        override fun setServiceRunning(run: Boolean): Boolean {
            service += run
            return true
        }

        override fun setWorkerScheduled(scheduled: Boolean) = Unit
    }

    @Test fun non_off_last_mode_starts_service_immediately() = runTest {
        for (mode in listOf(BackgroundMode.REALTIME, BackgroundMode.POWER_SAVING)) {
            val a = Actions()
            assertTrue(BootReceiver.startServiceFromLastMode({ mode }, a))
            assertEquals(listOf(true), a.service)
        }
    }

    @Test fun off_or_unknown_last_mode_starts_nothing() = runTest {
        for (mode in listOf(BackgroundMode.OFF, null)) {
            val a = Actions()
            assertFalse(BootReceiver.startServiceFromLastMode({ mode }, a))
            assertEquals(emptyList<Boolean>(), a.service)
        }
    }
}
