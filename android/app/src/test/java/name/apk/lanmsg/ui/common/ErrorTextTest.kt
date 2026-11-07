package name.apk.lanmsg.ui.common

import name.apk.lanmsg.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.TransferFailReason

class ErrorTextTest {
    @Test fun every_error_code_maps_to_a_distinct_string_resource() {
        val ids = ErrorCode.entries.map { errorTextRes(it) }
        ids.forEach { assertNotEquals(0, it) }
        assertEquals(ErrorCode.entries.size, ids.toSet().size)
    }

    @Test fun every_transfer_fail_reason_maps_to_a_distinct_string_resource() {
        val ids = TransferFailReason.entries.map { transferFailRes(it) }
        ids.forEach { assertNotEquals(0, it) }
        assertEquals(TransferFailReason.entries.size, ids.toSet().size)
    }

    @Test fun peer_offline_keeps_its_dedicated_text() {
        assertEquals(R.string.err_peer_offline, errorTextRes(ErrorCode.PEER_OFFLINE))
    }
}
