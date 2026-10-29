// P1 核心骨架：网络层（Task 8）。TLS 配置与路径等级判定。
// 注释中文，标识符英文。

pub mod path;
pub mod tls;

pub use path::{classify, local_ifaces, LocalIface};
pub use tls::{make_tls_configs, peer_device_id, TLS_SNI};
