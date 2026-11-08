//! Web 临时会话：应用层加密（Task 12）与 HTTP / WebSocket 服务（Task 14）。

pub mod crypto;
mod server;
pub mod session;
pub mod transfer;

pub use session::{WebDeps, WebHost};
