package name.apk.lanmsg.ui.common

import androidx.annotation.StringRes
import name.apk.lanmsg.R
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.PairFailReason
import uniffi.lanmsg_api.TransferFailReason

/** 错误码对应的界面文案。core 的 message 只是英文排障信息，界面不显示。 */
@StringRes
fun errorTextRes(code: ErrorCode): Int = when (code) {
    ErrorCode.NOT_INITIALIZED -> R.string.err_not_initialized
    ErrorCode.ALREADY_INITIALIZED -> R.string.err_already_initialized
    ErrorCode.NOT_FOUND -> R.string.err_not_found
    ErrorCode.INVALID_ARGUMENT -> R.string.err_invalid_argument
    ErrorCode.PEER_OFFLINE -> R.string.err_peer_offline
    ErrorCode.REJECTED -> R.string.err_rejected
    ErrorCode.IO -> R.string.err_io
    ErrorCode.INTERNAL -> R.string.err_internal
    ErrorCode.UNIMPLEMENTED -> R.string.err_unimplemented
}

/** 传输失败原因对应的界面文案。 */
@StringRes
fun transferFailRes(reason: TransferFailReason): Int = when (reason) {
    TransferFailReason.CANCELLED -> R.string.download_cancelled
    TransferFailReason.SOURCE_OFFLINE -> R.string.download_source_offline
    TransferFailReason.VERIFY_FAILED -> R.string.download_verify_failed
    TransferFailReason.IO -> R.string.download_io_failed
    TransferFailReason.UNKNOWN -> R.string.download_failed
}

/** 配对失败原因对应的界面文案。 */
@StringRes
fun pairFailRes(reason: PairFailReason): Int = when (reason) {
    PairFailReason.TIMEOUT -> R.string.pair_fail_timeout
    PairFailReason.CANCELLED -> R.string.pair_fail_cancelled
    PairFailReason.NOT_OPEN -> R.string.pair_fail_not_open
    PairFailReason.WRONG_SECRET -> R.string.pair_fail_wrong_secret
    PairFailReason.BUSY -> R.string.pair_fail_busy
    PairFailReason.CERT_MISMATCH -> R.string.pair_fail_cert_mismatch
    PairFailReason.BAD_SIGNATURE -> R.string.pair_fail_bad_signature
    PairFailReason.IDENTITY_MISMATCH -> R.string.pair_fail_identity_mismatch
    PairFailReason.INVALID_NAME -> R.string.pair_fail_invalid_name
    PairFailReason.SAS_MISMATCH -> R.string.pair_fail_sas_mismatch
    PairFailReason.UNKNOWN -> R.string.pair_fail_unknown
}
