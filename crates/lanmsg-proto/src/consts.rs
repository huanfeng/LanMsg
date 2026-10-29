// P1 核心骨架：协议常量（C1）。
// 所有端口、组播地址、阈值、超时等常量仅在此处定义一次，其他模块引用，不写字面量。
// 注释中文，标识符英文。

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

/// 协议版本号，写入帧与 Announce，版本不一致的包被丢弃。
pub const PROTO_VERSION: u16 = 1;

/// TCP 主服务端口（被占用则随机分配）。
pub const SERVICE_PORT: u16 = 53320;

/// UDP 组播发现端口。
pub const DISCOVERY_PORT: u16 = 53320;

/// IPv4 组播地址。
pub const DISCOVERY_V4: Ipv4Addr = Ipv4Addr::new(239, 255, 53, 20);

/// IPv6 组播地址。
pub const DISCOVERY_V6: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x5320);

/// Web 临时会话端口。
pub const WEB_PORT: u16 = 53321;

/// LocalSend 适配端口。
pub const LOCALSEND_PORT: u16 = 53317;

/// LocalSend 组播地址。
pub const LOCALSEND_MCAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);

/// 设备通告发送间隔。
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30);

/// 心跳 Ping 发送间隔。
pub const PING_INTERVAL: Duration = Duration::from_secs(15);

/// 心跳超时，超过即断开连接。
pub const PING_TIMEOUT: Duration = Duration::from_secs(45);

/// 网卡变化轮询间隔。
pub const IFACE_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// 设备离线判定时长。
pub const OFFLINE_AFTER: Duration = Duration::from_secs(90);

/// 单帧最大长度（字节）。
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// 配对令牌生存时间。
pub const PAIR_TOKEN_TTL: Duration = Duration::from_secs(5 * 60);

/// Web 空闲超时。
pub const WEB_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Blob 分段长度（字节）。
pub const BLOB_SEGMENT_LEN: u64 = 8 * 1024 * 1024;

/// Blob 并发连接数。
pub const BLOB_PARALLEL_CONNS: usize = 4;

/// 缩略图最大字节数。
pub const THUMB_MAX_BYTES: usize = 32 * 1024;

/// 缩略图边长像素。
pub const THUMB_EDGE_PX: u32 = 256;

/// 自动下载图片最大字节数。
pub const AUTO_DOWNLOAD_IMAGE_MAX: u64 = 10 * 1024 * 1024;

/// 自动下载文件最大字节数。
pub const AUTO_DOWNLOAD_FILE_MAX: u64 = 50 * 1024 * 1024;

/// VPN 路径阈值除数（lan 阈值 / 2）。
pub const VPN_THRESHOLD_DIVISOR: u64 = 2;

/// 消息撤回窗口。
pub const RECALL_WINDOW: Duration = Duration::from_secs(2 * 60);

/// 指数退避最小间隔。
pub const RETRY_MIN: Duration = Duration::from_secs(1);

/// 指数退避最大间隔。
pub const RETRY_MAX: Duration = Duration::from_secs(5 * 60);

/// 网段扫描最大前缀长度（/22）。
pub const SCAN_MIN_PREFIX: u8 = 22;

/// 缩略图缓存上限（字节）。
pub const THUMB_CACHE_LIMIT: u64 = 1024 * 1024 * 1024;

// —— 以下由 P2 使用（仍在 P1 Task 2 中定义）——

/// 配对 TLS 导出器标签。
pub const PAIR_TLS_EXPORTER_LABEL: &[u8] = b"EXPORTER-lanmsg-pair";

/// 陌生人门控窗口。
pub const STRANGER_GATE_WINDOW: Duration = Duration::from_secs(30 * 60);

/// 文件夹条目最大数量。
pub const MAX_FOLDER_ENTRIES: usize = 10_000;

/// 同步批次大小。
pub const SYNC_BATCH: usize = 256;

/// TLS 握手后拨号方写的首字节：控制通道。
pub const CHANNEL_CONTROL: u8 = 0x01;

/// TLS 握手后拨号方写的首字节：数据通道。
pub const CHANNEL_DATA: u8 = 0x02;

/// 数据通道空闲超时。
pub const DATA_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Blob 查询等待时长。
pub const BLOB_QUERY_WAIT: Duration = Duration::from_secs(2);

/// 进度上报间隔。
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
