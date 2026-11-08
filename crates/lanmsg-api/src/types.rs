// P1 核心骨架：客户端 API 类型（C5）。
// IPC 与 FFI 共用。ID 字段在 API 层一律为 String（C2 的 hex / MsgId 字符串形式），以便 FFI。
// 所有类型 #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]，
// 对外类型在 uniffi feature 下加 #[cfg_attr(feature = "uniffi", derive(...))]（结构体用 uniffi::Record，枚举用 uniffi::Enum）。
// 注释中文，标识符英文。

use serde::{Deserialize, Serialize};

// ===== 顶层 IPC 类型 =====

/// 客户端 API 请求包装：id 为请求序号，body 为具体请求。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiRequest {
    pub id: u64,
    pub body: Request,
}

/// 客户端 API 响应包装：id 对应请求序号，result 为响应结果或错误。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiResponse {
    pub id: u64,
    pub result: Result<Response, ApiError>,
}

/// IPC 帧（仅 IPC 使用，不导出到 UniFFI）：客户端请求、服务端响应或服务端主动推送的事件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IpcFrame {
    Request(ApiRequest),
    Response(ApiResponse),
    Event(Event),
}

/// API 调用错误。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

/// API 错误码。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ErrorCode {
    /// 服务尚未初始化。
    NotInitialized,
    /// 服务已初始化，不允许重复初始化。
    AlreadyInitialized,
    /// 请求的资源不存在。
    NotFound,
    /// 参数非法。
    InvalidArgument,
    /// 对端设备不在线。
    PeerOffline,
    /// 请求被拒绝。
    Rejected,
    /// IO 错误。
    Io,
    /// 内部错误。
    Internal,
    /// 请求对应的功能尚未实现。
    Unimplemented,
}

/// 传输失败原因（界面据此本地化文案）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TransferFailReason {
    /// 用户取消。
    Cancelled,
    /// 持有文件的设备不在线。
    SourceOffline,
    /// 整文件校验不通过。
    VerifyFailed,
    /// 本地读写失败。
    Io,
    /// 未知原因。
    Unknown,
}

// ===== Request =====

/// 客户端请求。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Request {
    GetSelf,
    Init {
        display_name: String,
        device_name: String,
        mode: InitMode,
    },
    SetStatus {
        status: StatusDto,
    },
    SetProfile {
        display_name: String,
    },
    ListPeers,
    ListConversations,
    ListMessages {
        conv: String,
        before: Option<String>,
        limit: u32,
    },
    OpenDm {
        user: String,
    },
    SendText {
        conv: String,
        text: String,
    },
    SendFiles {
        conv: String,
        paths: Vec<String>,
    },
    SendFolder {
        conv: String,
        path: String,
    },
    Recall {
        msg: String,
    },
    MarkRead {
        conv: String,
        upto: String,
    },
    DeleteLocal {
        msgs: Vec<String>,
        also_my_devices: bool,
    },
    DownloadBlob {
        hash: String,
    },
    CancelTransfer {
        transfer: String,
    },
    ListContacts,
    RequestContact {
        user: String,
    },
    ListRequests,
    RespondRequest {
        request: String,
        action: RequestAction,
    },
    SetContactPolicy {
        user: String,
        auto_accept: bool,
        ask_above_bytes: Option<u64>,
    },
    ListMyDevices,
    RemoveDevice {
        device: String,
    },
    PairStart {
        method: PairMethod,
    },
    PairJoin {
        target: PairTarget,
    },
    PairConfirm {
        session: String,
        matches: bool,
        /// 用户确认时界面上显示的数字；确认“一致”时必须与当前会话的数字相同，
        /// 否则拒绝（会话可能已被新连接覆盖）。拒绝（matches=false）时不检查。
        code: String,
    },
    PairCancel {
        session: String,
    },
    QueryFiles {
        filter: FileFilter,
    },
    /// 本机 `blob_local` 中路径位于 `under` 之下、且已被会话里每个其他成员的
    /// 每台未吊销设备持有（即已全部送达）的本地路径。
    DeliveredLocalPaths {
        under: String,
    },
    SearchMessages {
        query: String,
        conv: Option<String>,
        limit: u32,
    },
    ForwardFiles {
        conv: String,
        hashes: Vec<String>,
    },
    GetSettings,
    SetSettings {
        settings: SettingsDto,
    },
    AddManualPeer {
        addr: String,
    },
    /// 删除手动添加的对端（P5 追加）：停止对该地址的重连，并取消其“手动设备”放行。
    RemoveManualPeer {
        addr: String,
    },
    /// 列出手动添加的对端（P5 追加）。
    ListManualPeers,
    ScanSubnet {
        cidr: String,
    },
    ListInterfaces,
    /// 平台层通知：网络已变化（Wi-Fi 切换 / 恢复）。core 清零重连退避并立即重拨、重新通告。
    NetworkChanged,
    StationCreate {
        name: String,
    },
    StationList,
    StationPut {
        station: String,
        item: StationPutItem,
        mode: StoreMode,
        expires_at: Option<i64>,
    },
    StationSetPinned {
        station: String,
        item: String,
        pinned: bool,
    },
    StationItems {
        station: String,
    },
    StationRemove {
        station: String,
        item: String,
    },
    WebStart,
    WebStop,
    WebRespond {
        request: String,
        allow: bool,
    },
    SetUiFocus {
        conv: Option<String>,
    },
    Shutdown,
}

// ===== Response =====

/// 客户端响应：除 Ok 外，每个变体均为具名单字段 { value: T }。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Response {
    Ok,
    SelfInfo {
        value: SelfInfo,
    },
    MsgId {
        value: String,
    },
    Peers {
        value: Vec<PeerView>,
    },
    Conversations {
        value: Vec<ConversationSummary>,
    },
    Messages {
        value: Vec<MessageView>,
    },
    ConvId {
        value: String,
    },
    TransferId {
        value: String,
    },
    Contacts {
        value: Vec<ContactView>,
    },
    Requests {
        value: Vec<RequestView>,
    },
    Devices {
        value: Vec<DeviceView>,
    },
    PairSession {
        value: PairSessionInfo,
    },
    Files {
        value: Vec<FileView>,
    },
    /// 路径列表（如 DeliveredLocalPaths 的结果）。
    Paths {
        value: Vec<String>,
    },
    Settings {
        value: SettingsDto,
    },
    Interfaces {
        value: Vec<InterfaceView>,
    },
    ManualPeers {
        value: Vec<ManualPeerView>,
    },
    Stations {
        value: Vec<StationView>,
    },
    StationItems {
        value: Vec<StationItemView>,
    },
    WebSession {
        value: WebSessionInfo,
    },
}

// ===== Event =====

/// 服务端主动推送的事件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Event {
    PeerOnline {
        peer: PeerView,
    },
    PeerOffline {
        device: String,
    },
    MessageAdded {
        msg: MessageView,
    },
    MessageUpdated {
        msg: MessageView,
    },
    ConversationUpdated {
        summary: ConversationSummary,
    },
    TransferProgress {
        transfer: String,
        hash: String,
        done: u64,
        total: u64,
        bytes_per_sec: u64,
    },
    TransferFinished {
        transfer: String,
        hash: String,
        ok: bool,
        /// 英文排障信息，不用于界面显示。
        error: Option<String>,
        /// 失败原因（界面据此取文案）；成功时为 None。
        reason: Option<TransferFailReason>,
    },
    IncomingRequest {
        request: RequestView,
    },
    PairStatus {
        session: String,
        state: PairStateDto,
    },
    DevicesChanged,
    ContactsChanged,
    WebRequest {
        request: String,
        user_agent: String,
        ip: String,
    },
    StationUpdated {
        station: String,
    },
    FirewallMissing,
    ScanFinished {
        cidr: String,
        probed: u32,
        found: u32,
    },
    WebSessionChanged {
        info: Option<WebSessionInfo>,
    },
}

// ===== DTO =====

/// 初始化模式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum InitMode {
    /// 首台设备，自动创建自设备会话。
    FirstDevice,
    /// 加入现有用户，仅记录设备名。
    JoinExisting,
}

/// 状态 DTO。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum StatusDto {
    Online,
    Invisible,
    Busy,
}

/// 自身信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SelfInfo {
    pub initialized: bool,
    pub user_id: Option<String>,
    pub user_short: Option<String>,
    pub display_name: Option<String>,
    pub device_id: String,
    pub device_name: Option<String>,
    pub status: StatusDto,
}

/// 网络路径类型 DTO。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PathClassDto {
    Lan,
    Vpn,
    Internet,
    Relay,
}

/// 对端设备视图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PeerView {
    pub device_id: String,
    pub device_short: String,
    pub user_id: String,
    pub display_name: String,
    pub device_name: String,
    pub device_type: String,
    pub addr: String,
    pub path: PathClassDto,
    pub is_contact: bool,
    pub is_mine: bool,
}

/// 会话类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ConvKind {
    SelfDevices,
    Dm,
    Guest,
    LocalSend,
    Station,
}

/// 会话最后一条消息的结构化类型（供各端自行拼预览文案，core 不生成展示文本）。
///
/// 图片/视频判定与 mime 前缀一致：整条消息的文件 mime 全以 `image/` 开头 → `Image`，
/// 全以 `video/` 开头 → `Video`，否则 `File`（`name` 取第一个文件名）。
/// 注意：文档注释里不要写斜杠加星号，uniffi 会原样抄进 Kotlin KDoc，Kotlin 块注释可嵌套。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum LastMessageKind {
    Text,
    Image { count: u32 },
    Video { count: u32 },
    File { name: String, count: u32 },
    Folder { name: String },
    System,
    Recalled,
}

/// 会话摘要。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConversationSummary {
    pub conv: String,
    pub kind: ConvKind,
    pub title: String,
    pub last_text: Option<String>,
    /// 最后一条可见消息的类型；会话无消息时为 None。
    pub last_kind: Option<LastMessageKind>,
    pub last_hlc_millis: Option<i64>,
    pub unread: u32,
    pub peer_online: bool,
}

/// 消息状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum MessageState {
    Sending,
    Sent,
    Delivered,
    Read,
    Failed,
    Recalled,
}

/// 消息体视图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum MessageBodyView {
    Text {
        text: String,
    },
    Files {
        items: Vec<FileItemView>,
    },
    Folder {
        name: String,
        items: Vec<FileItemView>,
    },
    System {
        event: SystemEventView,
    },
    Unsupported,
}

/// 系统消息的结构化事件（界面据此按本地化资源拼文案；name 一律非空，
/// 缺失时由 core 用设备或用户的短 ID 兜底）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SystemEventView {
    DeviceAdded {
        name: String,
    },
    DeviceRemoved {
        name: String,
    },
    ContactAdded {
        name: String,
    },
    ContactUpdated {
        name: String,
    },
    NicknameUpdated {
        name: String,
    },
    MessagesDeleted {
        count: u32,
    },
    Recalled {
        author_name: String,
        by_me: bool,
    },
    /// LocalSend 对端开启了 PIN，暂不支持（P5 追加）。
    LocalSendPin {
        name: String,
    },
    /// LocalSend 对端正忙，请稍后重试（P5 追加）。
    LocalSendBusy {
        name: String,
    },
}

/// Blob 状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum BlobState {
    Remote,
    Downloading,
    Local,
    SourceOffline,
    Preparing,
}

/// 文件项视图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FileItemView {
    pub hash: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub thumb_path: Option<String>,
    pub local_path: Option<String>,
    pub state: BlobState,
    /// 文件作者的显示名（名字缺失时为设备短 ID，非空）。
    pub source_name: String,
    /// 作者是否为本机用户自己（界面据此显示“我”）。
    pub source_is_me: bool,
    /// 未下载时持有该文件的设备名（在线持有者 → 首个持有者 → 作者设备）；
    /// 已在本机时为 None。“已下载”由 `state == Local` 推出。
    pub holder_name: Option<String>,
}

/// 消息视图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MessageView {
    pub id: String,
    pub conv: String,
    pub author_user: String,
    pub author_device: String,
    pub author_device_name: String,
    pub is_mine: bool,
    pub hlc_millis: i64,
    pub body: MessageBodyView,
    pub state: MessageState,
}

/// 联系人视图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ContactView {
    pub user_id: String,
    pub user_short: String,
    pub display_name: String,
    pub online: bool,
    pub devices: Vec<DeviceView>,
    pub auto_accept: bool,
    pub ask_above_bytes: Option<u64>,
}

/// 设备视图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeviceView {
    pub device_id: String,
    pub device_short: String,
    pub name: String,
    pub device_type: String,
    pub online: bool,
    pub path: Option<PathClassDto>,
    pub is_self: bool,
}

/// 请求类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum RequestKind {
    Contact,
    Message,
    File,
}

/// 请求视图（用于 IncomingRequest 事件）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RequestView {
    pub request: String,
    pub kind: RequestKind,
    pub from_user: String,
    pub from_name: String,
    pub from_short: String,
    pub preview: RequestPreviewView,
    pub created_at: i64,
}

/// 请求内容预览（结构化；界面按 kind 与变体自行拼本地化文案）。
/// `Text` 的内容是用户消息原文，原样保留。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum RequestPreviewView {
    /// 无内容可预览（联系人请求，或无法归类的消息）。
    None,
    /// 文字消息的前若干字符。
    Text { text: String },
    /// 文件消息：首个文件名、文件个数、总字节数。
    Files {
        first_name: String,
        count: u32,
        total_size: u64,
    },
    /// 文件夹消息：文件夹名、文件个数。
    Folder { name: String, count: u32 },
}

/// 请求动作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum RequestAction {
    AcceptOnce,
    AddContact,
    Reject,
    Block,
}

/// 配对方式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PairMethod {
    Qr,
    Sas,
}

/// 配对目标。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PairTarget {
    QrUri { uri: String },
    Device { device: String },
}

/// 配对会话信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PairSessionInfo {
    pub session: String,
    pub qr_uri: Option<String>,
}

/// 配对状态 DTO。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PairStateDto {
    WaitingPeer,
    /// 数字比对：`peer_*` 是对端设备的名称、类型（小写，如 phone）与指纹（device_short），
    /// 供界面提示“请到该设备上核对”。
    ShowSas {
        code: String,
        peer_name: String,
        peer_type: String,
        peer_short: String,
    },
    Verifying,
    Done,
    Failed {
        reason: PairFailReason,
    },
}

/// 配对失败原因（界面据此本地化文案）。线上 `PairMsg::Abort.reason` 使用 `wire_key()`
/// 的稳定键名；收到未知键名（含旧版本发来的文本）一律解析为 `Unknown`。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PairFailReason {
    /// 配对窗口超时。
    Timeout,
    /// 用户取消。
    Cancelled,
    /// 对端未开启配对。
    NotOpen,
    /// 口令错误。
    WrongSecret,
    /// 对端正在与其他设备配对。
    Busy,
    /// 证书不是发给本机的。
    CertMismatch,
    /// 证书验签失败。
    BadSignature,
    /// 用户身份与证书不一致。
    IdentityMismatch,
    /// 显示名不合法。
    InvalidName,
    /// 双方确认的数字不一致。
    SasMismatch,
    /// 未知原因。
    Unknown,
}

impl PairFailReason {
    /// 线上使用的稳定键名（Unknown 也有键名，但解析时不会还原为别的变体）。
    pub fn wire_key(&self) -> &'static str {
        match self {
            PairFailReason::Timeout => "timeout",
            PairFailReason::Cancelled => "cancelled",
            PairFailReason::NotOpen => "not_open",
            PairFailReason::WrongSecret => "wrong_secret",
            PairFailReason::Busy => "busy",
            PairFailReason::CertMismatch => "cert_mismatch",
            PairFailReason::BadSignature => "bad_signature",
            PairFailReason::IdentityMismatch => "identity_mismatch",
            PairFailReason::InvalidName => "invalid_name",
            PairFailReason::SasMismatch => "sas_mismatch",
            PairFailReason::Unknown => "unknown",
        }
    }

    /// 由线上键名还原；未知键名映射为 `Unknown`。
    pub fn from_wire(key: &str) -> Self {
        match key {
            "timeout" => PairFailReason::Timeout,
            "cancelled" => PairFailReason::Cancelled,
            "not_open" => PairFailReason::NotOpen,
            "wrong_secret" => PairFailReason::WrongSecret,
            "busy" => PairFailReason::Busy,
            "cert_mismatch" => PairFailReason::CertMismatch,
            "bad_signature" => PairFailReason::BadSignature,
            "identity_mismatch" => PairFailReason::IdentityMismatch,
            "invalid_name" => PairFailReason::InvalidName,
            "sas_mismatch" => PairFailReason::SasMismatch,
            _ => PairFailReason::Unknown,
        }
    }
}

/// 文件种类过滤。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum FileKindFilter {
    All,
    Image,
    Video,
    Document,
    Archive,
    Other,
}

/// 文件查询过滤条件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FileFilter {
    pub kind: FileKindFilter,
    pub conv: Option<String>,
    pub source_device: Option<String>,
    pub state: Option<BlobState>,
    pub name_query: Option<String>,
    pub before_millis: Option<i64>,
    pub limit: u32,
}

/// 文件视图（查询结果行）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FileView {
    pub item: FileItemView,
    pub conv: String,
    /// 会话标题；自设备会话为空串，界面按 `conv_kind` 本地化。
    pub conv_title: String,
    pub conv_kind: ConvKind,
    pub msg: String,
    pub hlc_millis: i64,
}

/// 设置 DTO。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettingsDto {
    pub device_name: String,
    pub download_dir: String,
    pub auto_image_max: u64,
    pub auto_file_max: u64,
    pub auto_file_wifi_only: bool,
    pub localsend_enabled: bool,
    pub unicast_targets: Vec<String>,
    pub background_mode: BackgroundMode,
    /// 只读：P5 填充，SetSettings 时忽略。
    pub localsend_listening: Option<bool>,
    /// LocalSend 监听端口（P5 追加，默认 53317；SetSettings 校验 1024–65535）。
    #[serde(default = "default_localsend_port")]
    pub localsend_port: u16,
}

/// `SettingsDto.localsend_port` 的默认值（旧版本保存的设置缺该字段时使用）。
fn default_localsend_port() -> u16 {
    53317
}

/// 后台模式（仅 Android 使用）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum BackgroundMode {
    Realtime,
    PowerSaving,
    Off,
}

/// 手动添加的对端视图（P5 追加）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ManualPeerView {
    /// 添加时的地址（IP 或主机名，已规范化为 host:port）。
    pub addr: String,
    /// 已连上过的设备（hex）；尚未连上过为 None。
    pub device_id: Option<String>,
}

/// 网络接口视图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InterfaceView {
    pub name: String,
    pub addr: String,
    pub prefix: u8,
    pub tunnel_kind: Option<String>,
}

/// 中转站视图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StationView {
    pub station: String,
    pub name: String,
    pub host_device: String,
    pub host_online: bool,
    pub item_count: u32,
}

/// 中转站放入项。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum StationPutItem {
    Paths { paths: Vec<String> },
    Text { text: String },
    Blobs { hashes: Vec<String> },
}

/// 存储模式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum StoreMode {
    Reference,
    Copy,
}

/// 中转站条目视图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StationItemView {
    pub item: String,
    pub added_by: String,
    pub added_at: i64,
    pub expires_at: Option<i64>,
    pub text: Option<String>,
    pub file: Option<FileItemView>,
    pub pinned: bool,
    pub unavailable: Option<bool>,
}

/// Web 临时会话信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WebSessionInfo {
    pub url: String,
    pub code: String,
    pub qr_uri: String,
    pub urls: Option<Vec<String>>,
}

#[cfg(test)]
mod pair_fail_reason_tests {
    use super::PairFailReason;

    const ALL: [PairFailReason; 11] = [
        PairFailReason::Timeout,
        PairFailReason::Cancelled,
        PairFailReason::NotOpen,
        PairFailReason::WrongSecret,
        PairFailReason::Busy,
        PairFailReason::CertMismatch,
        PairFailReason::BadSignature,
        PairFailReason::IdentityMismatch,
        PairFailReason::InvalidName,
        PairFailReason::SasMismatch,
        PairFailReason::Unknown,
    ];

    #[test]
    fn wire_keys_round_trip() {
        for r in ALL {
            assert_eq!(PairFailReason::from_wire(r.wire_key()), r, "{r:?}");
        }
    }

    #[test]
    fn unknown_wire_key_maps_to_unknown() {
        assert_eq!(
            PairFailReason::from_wire("未开启配对"),
            PairFailReason::Unknown
        );
        assert_eq!(PairFailReason::from_wire(""), PairFailReason::Unknown);
        assert_eq!(
            PairFailReason::from_wire("no_such_key"),
            PairFailReason::Unknown
        );
    }

    #[test]
    fn wire_keys_are_distinct() {
        let mut keys: Vec<_> = ALL.iter().map(|r| r.wire_key()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), ALL.len());
    }
}
