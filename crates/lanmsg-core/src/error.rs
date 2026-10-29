// P1 核心骨架：核心错误类型。
// KeyStoreError 与 CoreError，以及 CoreError 到 lanmsg_api::ApiError 的映射。
// 注释中文，标识符英文。

use lanmsg_api::types::ErrorCode;

use crate::store::StoreError;

/// 密钥库错误。
#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    /// IO 错误。
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    /// 数据损坏。
    #[error("数据损坏: {0}")]
    Corrupt(String),
}

/// 核心库错误。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// 服务尚未初始化。
    #[error("服务尚未初始化")]
    NotInitialized,
    /// 服务已初始化，不允许重复初始化。
    #[error("服务已初始化")]
    AlreadyInitialized,
    /// 请求的资源不存在。
    #[error("资源不存在: {0}")]
    NotFound(String),
    /// 参数非法。
    #[error("参数非法: {0}")]
    InvalidArgument(String),
    /// 对端设备不在线。
    #[error("对端设备不在线")]
    PeerOffline,
    /// 请求被拒绝。
    #[error("请求被拒绝: {0}")]
    Rejected(String),
    /// IO 错误。
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    /// 存储层错误。
    #[error("存储错误: {0}")]
    Store(#[from] StoreError),
    /// 密钥库错误。
    #[error("密钥库错误: {0}")]
    KeyStore(#[from] KeyStoreError),
    /// 协议序列化错误。
    #[error("协议错误: {0}")]
    Proto(String),
    /// TLS 错误。
    #[error("TLS 错误: {0}")]
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
