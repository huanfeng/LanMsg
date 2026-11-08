// P5 地址簿帧：设备之间交换各自已知的对端地址（跨网段连接用）。
// 注释中文，标识符英文。

use serde::{Deserialize, Serialize};

use crate::ids::DeviceId;
use crate::types::PathClass;

/// 地址簿帧负载。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressBook {
    pub entries: Vec<AddressEntry>,
}

/// 某台设备的已知地址列表。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressEntry {
    pub device: DeviceId,
    pub addrs: Vec<KnownAddr>,
}

/// 单个已知地址。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownAddr {
    /// "ip:port"，IPv6 为 "[ip]:port"。
    pub addr: String,
    pub path: PathClass,
    /// 最近一次连通的 Unix 毫秒时间。
    pub last_ok: i64,
}
