//! LocalSend v2 协议实现（类型、证书、局域网发现）。不依赖 lanmsg-core。

pub mod cert;
mod client;
pub mod discovery;
mod server;
pub mod types;

pub use cert::{fingerprint, load_or_create, LsCert};
pub use discovery::{lenient_client, register_reply, LsDiscovery, LsPeerEvent};
pub use server::{LocalSend, LsConfig, LsDecision, LsEvent};
pub use types::{
    LsAnnounce, LsFileMeta, LsFileTimes, LsInfo, LsPeer, LsProtocol, PrepareUploadRequest,
    PrepareUploadResponse,
};

/// 官方协议固定的组播地址。
pub const LOCALSEND_MCAST: std::net::Ipv4Addr = std::net::Ipv4Addr::new(224, 0, 0, 167);
/// 官方协议固定的默认端口（UDP 发现与 HTTP(S) 共用）。
pub const LOCALSEND_PORT: u16 = 53317;

/// LocalSend 模块错误。
#[derive(Debug, thiserror::Error)]
pub enum LsError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(String),
    #[error("unexpected http status {0}")]
    Status(u16),
    #[error("json error: {0}")]
    Json(String),
    #[error("tls error: {0}")]
    Tls(String),
    #[error("localsend port is already in use")]
    PortBusy,
}
