// P1 核心骨架：核心错误类型。
// KeyStoreError 与 CoreError，以及 CoreError 到 lanmsg_api::ApiError 的映射。
// 注释中文，标识符英文。

use lanmsg_api::types::ErrorCode;

use crate::store::StoreError;

/// 密钥库错误。
#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    /// IO 错误。
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// 数据损坏。
    #[error("corrupt data: {0}")]
    Corrupt(String),
}

/// 核心库错误。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// service not initialized。
    #[error("service not initialized")]
    NotInitialized,
    /// service already initialized，不允许重复初始化。
    #[error("service already initialized")]
    AlreadyInitialized,
    /// 请求的资源不存在。
    #[error("not found: {0}")]
    NotFound(String),
    /// 参数非法。
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// peer device offline。
    #[error("peer device offline")]
    PeerOffline,
    /// 请求被拒绝。
    #[error("request rejected: {0}")]
    Rejected(String),
    /// IO 错误。
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// 存储层错误。
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    /// 密钥库错误。
    #[error("keystore error: {0}")]
    KeyStore(#[from] KeyStoreError),
    /// 协议序列化错误。
    #[error("protocol error: {0}")]
    Proto(String),
    /// TLS 错误。
    #[error("TLS error: {0}")]
    Tls(String),
}

impl From<CoreError> for lanmsg_api::types::ApiError {
    fn from(err: CoreError) -> Self {
        let (code, message) = match err {
            CoreError::NotInitialized => (ErrorCode::NotInitialized, err.to_string()),
            CoreError::AlreadyInitialized => (ErrorCode::AlreadyInitialized, err.to_string()),
            CoreError::NotFound(s) => (ErrorCode::NotFound, s),
            CoreError::InvalidArgument(s) => (ErrorCode::InvalidArgument, s),
            CoreError::PeerOffline => (ErrorCode::PeerOffline, err.to_string()),
            CoreError::Rejected(s) => (ErrorCode::Rejected, s),
            CoreError::Io(_) => (ErrorCode::Io, err.to_string()),
            CoreError::Store(_) => (ErrorCode::Internal, err.to_string()),
            CoreError::KeyStore(_) => (ErrorCode::Internal, err.to_string()),
            CoreError::Proto(_) => (ErrorCode::Internal, err.to_string()),
            CoreError::Tls(_) => (ErrorCode::Internal, err.to_string()),
        };
        lanmsg_api::types::ApiError { code, message }
    }
}
