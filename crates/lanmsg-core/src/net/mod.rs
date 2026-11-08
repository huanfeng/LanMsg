// P1 核心骨架：网络层（Task 8）。TLS 配置、路径等级判定、控制连接与连接管理器。
// 注释中文，标识符英文。

pub mod addrbook;
pub mod conn;
pub mod manager;
pub mod path;
pub mod timing;
pub mod tls;

pub use conn::{ConnEvent, DataHandler, DataStream, HelloInfo, PeerInfo};
pub use manager::ConnManager;
pub use path::{classify, detect_tunnel, local_ifaces, LocalIface, TunnelKind};
pub use tls::{make_tls_configs, make_tls_configs_expecting, peer_device_id, TLS_SNI};
