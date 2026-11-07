// 错误码与传输失败原因到界面文案的映射。
// core 给出的 message 是英文排障信息，界面一律不显示，只按结构化原因取文案。

use lanmsg_api::types::{ErrorCode, PairFailReason, TransferFailReason};

use crate::strings;

/// 错误码对应的界面文案。
pub fn error_text(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::NotInitialized => strings::ERR_NOT_INITIALIZED,
        ErrorCode::AlreadyInitialized => strings::ERR_ALREADY_INITIALIZED,
        ErrorCode::NotFound => strings::ERR_NOT_FOUND,
        ErrorCode::InvalidArgument => strings::ERR_INVALID_ARGUMENT,
        ErrorCode::PeerOffline => strings::ERR_PEER_OFFLINE,
        ErrorCode::Rejected => strings::ERR_REJECTED,
        ErrorCode::Io => strings::ERR_IO,
        ErrorCode::Internal => strings::ERR_INTERNAL,
        ErrorCode::Unimplemented => strings::ERR_UNIMPLEMENTED,
    }
}

/// 传输失败原因对应的界面文案。
pub fn transfer_fail_text(reason: &TransferFailReason) -> &'static str {
    match reason {
        TransferFailReason::Cancelled => strings::DOWNLOAD_CANCELLED,
        TransferFailReason::SourceOffline => strings::DOWNLOAD_SOURCE_OFFLINE,
        TransferFailReason::VerifyFailed => strings::DOWNLOAD_VERIFY_FAILED,
        TransferFailReason::Io => strings::DOWNLOAD_IO_FAILED,
        TransferFailReason::Unknown => strings::DOWNLOAD_FAILED,
    }
}

/// 配对失败原因对应的界面文案。
pub fn pair_fail_text(reason: &PairFailReason) -> &'static str {
    match reason {
        PairFailReason::Timeout => strings::PAIR_FAIL_TIMEOUT,
        PairFailReason::Cancelled => strings::PAIR_FAIL_CANCELLED,
        PairFailReason::NotOpen => strings::PAIR_FAIL_NOT_OPEN,
        PairFailReason::WrongSecret => strings::PAIR_FAIL_WRONG_SECRET,
        PairFailReason::Busy => strings::PAIR_FAIL_BUSY,
        PairFailReason::CertMismatch => strings::PAIR_FAIL_CERT_MISMATCH,
        PairFailReason::BadSignature => strings::PAIR_FAIL_BAD_SIGNATURE,
        PairFailReason::IdentityMismatch => strings::PAIR_FAIL_IDENTITY_MISMATCH,
        PairFailReason::InvalidName => strings::PAIR_FAIL_INVALID_NAME,
        PairFailReason::SasMismatch => strings::PAIR_FAIL_SAS_MISMATCH,
        PairFailReason::Unknown => strings::PAIR_FAIL_UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CODES: [ErrorCode; 9] = [
        ErrorCode::NotInitialized,
        ErrorCode::AlreadyInitialized,
        ErrorCode::NotFound,
        ErrorCode::InvalidArgument,
        ErrorCode::PeerOffline,
        ErrorCode::Rejected,
        ErrorCode::Io,
        ErrorCode::Internal,
        ErrorCode::Unimplemented,
    ];

    const ALL_REASONS: [TransferFailReason; 5] = [
        TransferFailReason::Cancelled,
        TransferFailReason::SourceOffline,
        TransferFailReason::VerifyFailed,
        TransferFailReason::Io,
        TransferFailReason::Unknown,
    ];

    /// 新增 ErrorCode 变体时这里不编译，提醒补文案与测试。
    #[test]
    fn all_codes_list_is_exhaustive() {
        for c in &ALL_CODES {
            match c {
                ErrorCode::NotInitialized
                | ErrorCode::AlreadyInitialized
                | ErrorCode::NotFound
                | ErrorCode::InvalidArgument
                | ErrorCode::PeerOffline
                | ErrorCode::Rejected
                | ErrorCode::Io
                | ErrorCode::Internal
                | ErrorCode::Unimplemented => {}
            }
        }
    }

    #[test]
    fn every_error_code_has_text() {
        for c in &ALL_CODES {
            let t = error_text(c);
            assert!(!t.is_empty(), "{c:?}");
            assert_ne!(t, format!("{c:?}"));
        }
    }

    const ALL_PAIR_REASONS: [PairFailReason; 11] = [
        PairFailReason::Timeout,
        PairFailReason::Cancelled,
        PairFailReason::NotOpen,
        PairFailReason::WrongSecret,
        PairFailReason::Busy,
        PairFailReason::CertMismatch,
        PairFailReason::BadSignature,
        PairFailReason::IdentityMismatch,
        PairFailReason::InvalidName,
        PairFailReason::SasMismatch,
        PairFailReason::Unknown,
    ];

    #[test]
    fn every_pair_fail_reason_has_text() {
        for r in &ALL_PAIR_REASONS {
            let t = pair_fail_text(r);
            assert!(!t.is_empty(), "{r:?}");
            assert_ne!(t, format!("{r:?}"));
        }
    }

    #[test]
    fn unknown_pair_reason_text_is_generic() {
        assert_eq!(pair_fail_text(&PairFailReason::Unknown), "配对失败，请重试");
    }

    #[test]
    fn every_transfer_fail_reason_has_text() {
        for r in &ALL_REASONS {
            let t = transfer_fail_text(r);
            assert!(!t.is_empty(), "{r:?}");
            assert_ne!(t, format!("{r:?}"));
        }
    }
}
