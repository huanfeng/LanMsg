package name.apk.lanmsg.ui.common

import name.apk.lanmsg.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.PairFailReason
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

    @Test fun every_pair_fail_reason_maps_to_a_distinct_string_resource() {
        val ids = PairFailReason.entries.map { pairFailRes(it) }
        ids.forEach { assertNotEquals(0, it) }
        assertEquals(PairFailReason.entries.size, ids.toSet().size)
    }

    /** 逐个变体断言具体资源，抓映射错位。 */
    @Test fun error_code_mapping_matches_spec() {
        val expect = mapOf(
            ErrorCode.NOT_INITIALIZED to R.string.err_not_initialized,
            ErrorCode.ALREADY_INITIALIZED to R.string.err_already_initialized,
            ErrorCode.NOT_FOUND to R.string.err_not_found,
            ErrorCode.INVALID_ARGUMENT to R.string.err_invalid_argument,
            ErrorCode.PEER_OFFLINE to R.string.err_peer_offline,
            ErrorCode.REJECTED to R.string.err_rejected,
            ErrorCode.IO to R.string.err_io,
            ErrorCode.INTERNAL to R.string.err_internal,
            ErrorCode.UNIMPLEMENTED to R.string.err_unimplemented,
        )
        assertEquals(ErrorCode.entries.toSet(), expect.keys)
        expect.forEach { (code, res) -> assertEquals("$code", res, errorTextRes(code)) }
    }

    @Test fun transfer_fail_mapping_matches_spec() {
        val expect = mapOf(
            TransferFailReason.CANCELLED to R.string.download_cancelled,
            TransferFailReason.SOURCE_OFFLINE to R.string.download_source_offline,
            TransferFailReason.VERIFY_FAILED to R.string.download_verify_failed,
            TransferFailReason.IO to R.string.download_io_failed,
            TransferFailReason.UNKNOWN to R.string.download_failed,
        )
        assertEquals(TransferFailReason.entries.toSet(), expect.keys)
        expect.forEach { (r, res) -> assertEquals("$r", res, transferFailRes(r)) }
    }

    @Test fun pair_fail_mapping_matches_spec() {
        val expect = mapOf(
            PairFailReason.TIMEOUT to R.string.pair_fail_timeout,
            PairFailReason.CANCELLED to R.string.pair_fail_cancelled,
            PairFailReason.NOT_OPEN to R.string.pair_fail_not_open,
            PairFailReason.WRONG_SECRET to R.string.pair_fail_wrong_secret,
            PairFailReason.BUSY to R.string.pair_fail_busy,
            PairFailReason.CERT_MISMATCH to R.string.pair_fail_cert_mismatch,
            PairFailReason.BAD_SIGNATURE to R.string.pair_fail_bad_signature,
            PairFailReason.IDENTITY_MISMATCH to R.string.pair_fail_identity_mismatch,
            PairFailReason.INVALID_NAME to R.string.pair_fail_invalid_name,
            PairFailReason.SAS_MISMATCH to R.string.pair_fail_sas_mismatch,
            PairFailReason.UNKNOWN to R.string.pair_fail_unknown,
        )
        assertEquals(PairFailReason.entries.toSet(), expect.keys)
        expect.forEach { (r, res) -> assertEquals("$r", res, pairFailRes(r)) }
    }
}
