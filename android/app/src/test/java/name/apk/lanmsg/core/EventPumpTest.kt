package name.apk.lanmsg.core

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.Event

@RunWith(AndroidJUnit4::class)
class EventPumpTest {
    private fun event(): Event = Event.PeerOffline("d")

    @Test fun buffer_overflow_emits_lagged() = runTest {
        val events = MutableSharedFlow<Event>(replay = 0, extraBufferCapacity = 2)
        val lagged = MutableSharedFlow<Unit>(replay = 1, extraBufferCapacity = 1)
        // 订阅者挂起且永不消费：缓冲会被填满
        val stuck = launch(UnconfinedTestDispatcher(testScheduler), start = CoroutineStart.UNDISPATCHED) {
            events.collect { awaitCancellation() }
        }
        val pump = EventPump(events, lagged)
        repeat(10) { pump.onEvent(event()) }
        assertEquals(Unit, lagged.replayCache.firstOrNull())
        stuck.cancel()
    }

    @Test fun no_overflow_no_lagged() = runTest {
        val events = MutableSharedFlow<Event>(replay = 0, extraBufferCapacity = 8)
        val lagged = MutableSharedFlow<Unit>(replay = 1, extraBufferCapacity = 1)
        EventPump(events, lagged).onEvent(event())
        assertNull(lagged.replayCache.firstOrNull())
    }
}
