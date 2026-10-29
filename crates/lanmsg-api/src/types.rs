// P1 核心骨架：客户端 API 类型（C5）。
// IPC 与 FFI 共用。ID 字段在 API 层一律为 String（C2 的 hex / MsgId 字符串形式），以便 FFI。
// 所有类型 #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]，
// uniffi feature 下加 #[cfg_attr(feature = "uniffi", derive(...))]（结构体用 uniffi::Record，枚举用 uniffi::Enum）。
// 注释中文，标识符英文。

use serde::{Deserialize, Serialize};

// ===== 顶层 IPC 类型 =====

/// 客户端 API 请求包装：id 为请求序号，body 为具体请求。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ApiRequest {
    pub id: u64,
    pub body: Request,
}

/// 客户端 API 响应包装：id 对应请求序号，result 为响应结果或错误。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ApiResponse {
    pub id: u64,
    pub result: Result<Response, ApiError>,
}

/// IPC 帧：客户端请求、服务端响应或服务端主动推送的事件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum IpcFrame {
    Request(ApiRequest),
    Response(ApiResponse),
    Event(Event),
}

/// API 调用错误。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
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
    },
    PairCancel {
        session: String,
    },
    QueryFiles {
        filter: FileFilter,
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
    ScanSubnet {
        cidr: String,
    },
    ListInterfaces,
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
    SelfInfo { value: SelfInfo },
    MsgId { value: String },
    Peers { value: Vec<PeerView> },
    Conversations { value: Vec<ConversationSummary> },
    Messages { value: Vec<MessageView> },
    ConvId { value: String },
    TransferId { value: String },
    Contacts { value: Vec<ContactView> },
    Requests { value: Vec<RequestView> },
    Devices { value: Vec<DeviceView> },
    PairSession { value: PairSessionInfo },
    Files { value: Vec<FileView> },
    Settings { value: SettingsDto },
    Interfaces { value: Vec<InterfaceView> },
    Stations { value: Vec<StationView> },
    StationItems { value: Vec<StationItemView> },
    WebSession { value: WebSessionInfo },
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
        error: Option<String>,
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

/// 会话摘要。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConversationSummary {
    pub conv: String,
    pub kind: ConvKind,
    pub title: String,
    pub last_text: Option<String>,
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
        text: String,
    },
    Unsupported,
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
    pub source_label: String,
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
    pub preview: String,
    pub created_at: i64,
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
    ShowSas { code: String },
    Verifying,
    Done,
    Failed { reason: String },
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
    pub conv_title: String,
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
}

/// 后台模式（仅 Android 使用）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum BackgroundMode {
    Realtime,
    PowerSaving,
    Off,
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
