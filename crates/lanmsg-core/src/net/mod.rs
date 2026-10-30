// P1 核心骨架：网络层（Task 8）。TLS 配置、路径等级判定、控制连接与连接管理器。
// 注释中文，标识符英文。

pub mod conn;
pub mod manager;
pub mod path;
pub mod tls;

pub use conn::{ConnEvent, HelloInfo, PeerInfo};
pub use manager::ConnManager;
pub use path::{classify, local_ifaces, LocalIface};
pub use tls::{make_tls_configs, peer_device_id, TLS_SNI};
