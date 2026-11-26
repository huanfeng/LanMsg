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

/// LocalSend 端口可配置范围的下界（含）。
pub const LOCALSEND_PORT_MIN: u16 = 1024;

/// LocalSend 端口可配置范围的上界（含）。
pub const LOCALSEND_PORT_MAX: u16 = 65535;

/// LocalSend 组播地址。
pub const LOCALSEND_MCAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);

/// 设备通告发送间隔。
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30);

/// 心跳 Ping 发送间隔。
pub const PING_INTERVAL: Duration = Duration::from_secs(15);

/// 心跳超时，超过即断开连接。
pub const PING_TIMEOUT: Duration = Duration::from_secs(45);

/// 同设备新连接到达时，旧连接若在此时长内没有收到任何包，视为已死并被新连接替换
/// （对端无 FIN 消失后重启/换网重连的场景）。须大于 PING_INTERVAL 以免误伤存活连接。
pub const RECONNECT_REPLACE_IDLE: Duration = Duration::from_secs(20);

/// 出站拨号总时限（TCP 连接 + TLS 握手 + Hello 交换）：对端 TCP 已接受但应用层沉默时
/// 不能让拨号方永久挂起。
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// 入站握手总时限（TLS 握手 + 通道字节 + Hello / DataHello）：只连接不发数据的对端
/// 在此时限后被关闭，释放 fd。
pub const ACCEPT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

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

/// 每个对端后台同步任务的作业队列容量（满则丢弃，防远端洪泛放大）。
pub const SYNC_JOB_QUEUE: usize = 64;

/// 同时拨号宽限窗口（毫秒）：出站连接建立后若本端 DeviceId 大于对端
/// （按重复连接裁决该出站连接注定被对端关闭），先等待该时长看对端拨入
/// 的连接是否到达，避免对必被淘汰的连接先发 Up 再发 Down。
pub const DUP_DIAL_GRACE_MS: u64 = 300;

/// TLS 握手后拨号方写的首字节：控制通道。
pub const CHANNEL_CONTROL: u8 = 0x01;

/// TLS 握手后拨号方写的首字节：数据通道。
pub const CHANNEL_DATA: u8 = 0x02;

/// TLS 握手后拨号方写的首字节：反向建立的数据通道（NAT 下由被请求方拨回，
/// 首帧 DataHello 之后紧跟 DialBack 帧）。
pub const CHANNEL_DATA_DIAL_BACK: u8 = 0x03;

/// 数据通道空闲超时。
pub const DATA_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Blob 查询等待时长。
pub const BLOB_QUERY_WAIT: Duration = Duration::from_secs(2);

/// 进度上报间隔。
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// 密钥库后端选择标记文件名（位于数据目录下），内容为 `os` 或 `file`。
pub const KEYSTORE_BACKEND_MARKER: &str = "keystore.backend";

/// 密钥库后端标记取值：使用系统密钥库。
pub const KEYSTORE_BACKEND_OS: &str = "os";

/// 密钥库后端标记取值：使用文件密钥库。
pub const KEYSTORE_BACKEND_FILE: &str = "file";

/// 探测系统密钥库可用性时读写的临时条目名。
pub const KEYSTORE_PROBE_NAME: &str = "lanmsg-probe";

/// 首次选定密钥库后端时，等待系统密钥库响应的最长时间。
///
/// 钥匙环被锁定（如桌面自动登录）时，Secret Service 会等用户在桌面上解锁，调用方可能永远
/// 收不到返回；此时没有可丢失的身份，超时后本次回退到文件库且不写标记，下次启动再试。
/// 取值偏短是因为超时只是“本次回退”，不是永久决定。已选定 `os` 后端后的读写不受此限制：
/// 那时回退文件库会重建身份，必须等用户解锁。
pub const KEYSTORE_PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// 用户显示名最大字符数（trim 后，按 Unicode 标量值计）。
pub const DISPLAY_NAME_MAX_CHARS: usize = 32;

/// 设备名最大字符数（trim 后，按 Unicode 标量值计）。
pub const DEVICE_NAME_MAX_CHARS: usize = 32;

/// 数据目录下的“后台已被用户主动退出”标记文件名。
/// lanmsgd 因托盘“退出”或 IPC Shutdown 退出前写入、启动时删除；
/// 界面进程连不上后台时若看到它，就不再自动拉起 lanmsgd。
pub const DAEMON_QUIT_MARKER: &str = "lanmsgd.quit";
