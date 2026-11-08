// P5 文件中转站：主机端（托管站点、条目登记、过期与可用性巡检）。
// 注释中文，标识符英文。

pub mod client;
pub mod host;

pub use host::StationHost;
