// 界面状态模型：纯数据，不依赖 windui，所有状态迁移逻辑在此单测。
// 注释中文，标识符英文。

use std::collections::{BTreeMap, BTreeSet};

use lanmsg_api::types::*;

use crate::bridge::{ReplyTag, UiMsg};
use crate::strings;

/// 一次拉取的消息页大小。
pub const MESSAGE_PAGE: u32 = 50;
/// 内存中保留的消息窗口上限。
pub const MESSAGE_WINDOW_MAX: usize = 300;
/// 文件视图一页的大小。
pub const FILE_PAGE: u32 = 100;
/// 搜索结果上限。
const SEARCH_LIMIT: u32 = 50;

/// 主页面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Wizard,
    Chats,
    Contacts,
    Files,
    Settings,
}

/// 与后台服务的连接状态。
#[derive(Debug, Clone, PartialEq)]
pub enum ConnState {
    Connecting,
    Connected,
    Disconnected(String),
}

/// 单个文件的传输进度。
#[derive(Debug, Clone, PartialEq)]
pub struct TransferInfo {
    pub hash: String,
    pub done: u64,
    pub total: u64,
    pub bytes_per_sec: u64,
}

/// 配对界面状态。
#[derive(Debug, Clone, PartialEq)]
pub enum PairUi {
    /// 加入方：选择可见设备或粘贴配对链接。
    Picking,
    /// `PairStart` / `PairJoin` 已发出，等待会话信息。
    Starting,
    Showing {
        session: String,
        qr_uri: Option<String>,
        state: PairStateDto,
    },
}

/// 配对链接是否合法：trim 后以 `lanmsg://` 开头且后缀非空。
pub fn pair_link_ok(link: &str) -> bool {
    link.trim()
        .strip_prefix("lanmsg://")
        .is_some_and(|rest| !rest.is_empty())
}

fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn port_ok(port: &str) -> bool {
    !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|p| p != 0)
}

/// 规范化手动添加的对端地址，缺端口时补默认服务端口。
/// 接受 `host`、`host:port`、`IPv4`、`IPv4:port`、`[v6]:port`、`[v6]`（补默认端口）、裸 IPv6；
/// 端口须在 1–65535。
pub fn normalize_peer_addr(input: &str) -> Result<String, &'static str> {
    let s = input.trim();
    let port = lanmsg_proto::consts::SERVICE_PORT;
    if s.is_empty() {
        return Err(strings::MANUAL_PEER_EMPTY);
    }
    if let Some(rest) = s.strip_prefix('[') {
        let (v6, tail) = rest.split_once(']').ok_or(strings::MANUAL_PEER_INVALID)?;
        if v6.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(strings::MANUAL_PEER_INVALID);
        }
        return match tail {
            "" => Ok(format!("[{v6}]:{port}")),
            t => match t.strip_prefix(':') {
                Some(p) if port_ok(p) => Ok(format!("[{v6}]:{p}")),
                _ => Err(strings::MANUAL_PEER_PORT_INVALID),
            },
        };
    }
    match s.matches(':').count() {
        0 if host_ok(s) => Ok(format!("{s}:{port}")),
        1 => {
            let (host, p) = s.split_once(':').unwrap_or_default();
            if !host_ok(host) {
                Err(strings::MANUAL_PEER_INVALID)
            } else if !port_ok(p) {
                Err(strings::MANUAL_PEER_PORT_INVALID)
            } else {
                Ok(s.to_string())
            }
        }
        n if n >= 2 && s.parse::<std::net::Ipv6Addr>().is_ok() => Ok(format!("[{s}]:{port}")),
        _ => Err(strings::MANUAL_PEER_INVALID),
    }
}

/// 最近一次配对的发起意图：失败后“重试”据此重来。
#[derive(Debug, Clone, PartialEq)]
pub enum PairIntent {
    Start(PairMethod),
    Join(PairTarget),
}

/// 联系人页分类数量（联系人 / 新的请求 / 附近的人 / 我的设备）。
pub const CONTACTS_TABS: usize = 4;

/// Model 迁移产生的副作用：交由 UiState 经 Bridge 执行。
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Call(Request, ReplyTag),
    /// 首台设备初始化成功后登记 lanmsgd 开机自启（失败只提示）。
    EnableAutostart,
}

/// 文件项的有效状态：核心报告待下载，但本地已发出请求或正在传输时按“下载中”显示。
/// 消息行与文件页共用。
pub fn effective_state(item: &FileItemView, model: &Model) -> BlobState {
    if item.state == BlobState::Remote
        && (model.pending_downloads.contains(&item.hash)
            || model.transfers.contains_key(&item.hash))
    {
        BlobState::Downloading
    } else {
        item.state.clone()
    }
}

/// 界面全部可观察状态。
#[derive(Debug, Clone)]
pub struct Model {
    pub conn: ConnState,
    pub me: Option<SelfInfo>,
    pub page: Page,
    pub convs: Vec<ConversationSummary>,
    pub current_conv: Option<String>,
    pub messages: Vec<MessageView>,
    pub has_older: bool,
    pub peers: Vec<PeerView>,
    pub contacts: Vec<ContactView>,
    pub requests: Vec<RequestView>,
    pub devices: Vec<DeviceView>,
    pub files: Vec<FileView>,
    pub file_filter: FileFilter,
    pub settings: Option<SettingsDto>,
    pub transfers: BTreeMap<String, TransferInfo>,
    /// 哈希 -> 传输 id：取消传输要用；下载回复与进度事件都会登记，结束时清除。
    pub transfer_ids: BTreeMap<String, String>,
    /// 加载更早消息的请求已发出、尚未得到回复（防止到顶回调连发造成重复合并）。
    pub older_in_flight: bool,
    /// 窗口之外还有更新的消息（前插导致从末尾截断时置真）；为真时新消息不追加，
    /// 滚到底部再重新拉取最新一页。
    pub has_newer: bool,
    /// 重新拉取最新一页的请求已发出、尚未得到回复（防重；失败时 has_newer 保持为真）。
    pub reload_in_flight: bool,
    /// 已发出下载请求、尚未结束的文件哈希：点击后立刻显示“下载中”，并防止重复请求。
    pub pending_downloads: BTreeSet<String>,
    /// 用户已取消的哈希：对应的失败结束事件不再弹“下载失败”。
    pub cancelled_downloads: BTreeSet<String>,
    pub pair: Option<PairUi>,
    /// 最近一次配对意图（重试用）。
    pub pair_intent: Option<PairIntent>,
    /// 先于 `PairSession` 回复到达的配对状态事件 `(会话, 状态)`：回复到达后应用。
    pub pair_early_status: Option<(String, PairStateDto)>,
    /// 每次发起/加入配对自增；回复带回，用于识别取消后迟到的旧回复。
    pub pair_nonce: u64,
    /// 联系人页当前分类（0 联系人、1 新的请求、2 附近的人、3 我的设备）。
    pub contacts_tab: usize,
    /// 已发出处理请求、尚未回复的请求 id（防重复点击）。
    pub pending_requests: BTreeSet<String>,
    /// 已发出联系人请求、尚未回复的用户 id（防重复点击）。
    pub pending_contact_requests: BTreeSet<String>,
    pub toast: Option<String>,
    /// 发送失败待回填输入框的草稿 `(会话, 文字)`；由 UiState 在输入框存在时取走。
    pub draft_restore: Option<(String, String)>,
    pub window_focused: bool,
    pub search_query: String,
    pub search_results: Vec<MessageView>,
    /// 已提交、尚未得到回复的向导初始化模式。
    pub init_mode: Option<InitMode>,
    /// 以“加入已有设备”方式完成了 Init，等待配对；配对完成或取消后复位。
    pub wizard_joining: bool,
    /// Init 已发出、尚未得到回复（期间禁用向导按钮，防重复提交）。
    pub init_in_flight: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}

impl Model {
    pub fn new() -> Model {
        Model {
            conn: ConnState::Connecting,
            me: None,
            page: Page::Chats,
            convs: Vec::new(),
            current_conv: None,
            messages: Vec::new(),
            has_older: false,
            peers: Vec::new(),
            contacts: Vec::new(),
            requests: Vec::new(),
            devices: Vec::new(),
            files: Vec::new(),
            file_filter: FileFilter {
                kind: FileKindFilter::All,
                conv: None,
                source_device: None,
                state: None,
                name_query: None,
                before_millis: None,
                limit: FILE_PAGE,
            },
            settings: None,
            transfers: BTreeMap::new(),
            transfer_ids: BTreeMap::new(),
            older_in_flight: false,
            has_newer: false,
            reload_in_flight: false,
            pending_downloads: BTreeSet::new(),
            cancelled_downloads: BTreeSet::new(),
            pair: None,
            pair_intent: None,
            pair_early_status: None,
            pair_nonce: 0,
            contacts_tab: 0,
            pending_requests: BTreeSet::new(),
            pending_contact_requests: BTreeSet::new(),
            toast: None,
            draft_restore: None,
            window_focused: true,
            search_query: String::new(),
            search_results: Vec::new(),
            init_mode: None,
            wizard_joining: false,
            init_in_flight: false,
        }
    }

    /// 处理一条来自桥的消息，返回需要执行的副作用。
    pub fn apply(&mut self, msg: UiMsg) -> Vec<Effect> {
        match msg {
            UiMsg::Connected | UiMsg::Resync => self.refresh_all(),
            UiMsg::Disconnected(reason) => {
                // 断开期间保留已有数据，只更新连接状态
                self.conn = ConnState::Disconnected(reason);
                Vec::new()
            }
            UiMsg::Reply { tag, result } => match result {
                Ok(resp) => self.apply_reply(tag, resp),
                Err(e) => {
                    // 断线期间的自动请求会得到“未连接”，顶部提示条已说明，不再弹 toast
                    let offline_io =
                        e.code == ErrorCode::Io && matches!(self.conn, ConnState::Disconnected(_));
                    if !offline_io {
                        self.toast = Some(e.message);
                    }
                    match &tag {
                        // 文字发送失败：草稿交还输入框，避免用户重打
                        ReplyTag::Sent {
                            conv,
                            draft: Some(d),
                        } => self.draft_restore = Some((conv.clone(), d.clone())),
                        ReplyTag::Messages { older: true, .. } => self.older_in_flight = false,
                        ReplyTag::Messages { older: false, .. } => self.reload_in_flight = false,
                        _ => {}
                    }
                    if let ReplyTag::Transfer { hash } = &tag {
                        // 下载请求失败：恢复为可再次下载
                        self.pending_downloads.remove(hash);
                    }
                    if tag == ReplyTag::Init {
                        // Init 失败：回到可重新提交的状态
                        self.init_mode = None;
                        self.init_in_flight = false;
                    }
                    let mut fx = Vec::new();
                    match &tag {
                        ReplyTag::Responded { request } => {
                            self.pending_requests.remove(request);
                        }
                        ReplyTag::Requested { user } => {
                            self.pending_contact_requests.remove(user);
                        }
                        // 发起方启动失败：关闭对话框；加入方失败：回到选择界面可改选
                        // 仅 nonce 匹配（仍是最近一次发起）时才动界面，过期回复的错误不影响新会话
                        ReplyTag::Pair { nonce } if *nonce == self.pair_nonce => self.pair = None,
                        ReplyTag::PairJoin { nonce } if *nonce == self.pair_nonce => {
                            self.pair = Some(PairUi::Picking)
                        }
                        // 乐观更新失败：以服务端为准
                        ReplyTag::Policy => {
                            fx.push(call(Request::ListContacts, ReplyTag::Contacts));
                        }
                        _ => {}
                    }
                    fx
                }
            },
            UiMsg::Event(ev) => self.apply_event(ev),
        }
    }

    /// 连接（重连）后的全量刷新。
    fn refresh_all(&mut self) -> Vec<Effect> {
        self.conn = ConnState::Connected;
        // 后台服务可能重启过，旧的下载请求不会再有结束事件
        self.pending_downloads.clear();
        self.cancelled_downloads.clear();
        let mut fx = vec![
            call(Request::GetSelf, ReplyTag::SelfInfo),
            call(Request::ListConversations, ReplyTag::Conversations),
            call(Request::ListRequests, ReplyTag::Requests),
            call(Request::GetSettings, ReplyTag::Settings),
            call(Request::ListPeers, ReplyTag::Peers),
        ];
        if let Some(c) = self.current_conv.clone() {
            // 后台服务可能重启过，界面焦点需重新告知
            if self.window_focused {
                fx.push(call(
                    Request::SetUiFocus {
                        conv: Some(c.clone()),
                    },
                    ReplyTag::Generic,
                ));
            }
            fx.push(self.list_messages_call(c, None, false));
        }
        fx
    }

    fn list_messages_call(&self, conv: String, before: Option<String>, older: bool) -> Effect {
        call(
            Request::ListMessages {
                conv: conv.clone(),
                before,
                limit: MESSAGE_PAGE,
            },
            ReplyTag::Messages { conv, older },
        )
    }

    fn mark_read_last(&self) -> Option<Effect> {
        let conv = self.current_conv.clone()?;
        let last = self.messages.last()?;
        Some(call(
            Request::MarkRead {
                conv,
                upto: last.id.clone(),
            },
            ReplyTag::Generic,
        ))
    }

    fn apply_reply(&mut self, tag: ReplyTag, resp: Response) -> Vec<Effect> {
        match (tag, resp) {
            (ReplyTag::SelfInfo, Response::SelfInfo { value }) => {
                if !value.initialized {
                    self.page = Page::Wizard;
                    self.me = Some(value);
                    return Vec::new();
                }
                let mut fx = Vec::new();
                if self.page == Page::Wizard {
                    // 向导完成：补拉会话列表（初始化前的拉取可能为空）
                    self.page = Page::Chats;
                    fx.push(call(Request::ListConversations, ReplyTag::Conversations));
                }
                self.me = Some(value);
                fx
            }
            (ReplyTag::Conversations, Response::Conversations { value }) => {
                self.convs = value;
                sort_convs(&mut self.convs);
                Vec::new()
            }
            (ReplyTag::Messages { conv, older }, Response::Messages { value }) => {
                if self.current_conv.as_deref() != Some(conv.as_str()) {
                    // 已切换会话，丢弃过期回复
                    return Vec::new();
                }
                self.has_older = value.len() == MESSAGE_PAGE as usize;
                if older {
                    self.older_in_flight = false;
                    let mut merged = value;
                    merged.append(&mut self.messages);
                    if merged.len() > MESSAGE_WINDOW_MAX {
                        // 截掉的是最新的消息：窗口与最新消息之间出现缺口
                        merged.truncate(MESSAGE_WINDOW_MAX);
                        self.has_newer = true;
                    }
                    self.messages = merged;
                    Vec::new()
                } else {
                    self.messages = value;
                    self.has_newer = false;
                    self.reload_in_flight = false;
                    if self.window_focused {
                        self.mark_read_last().into_iter().collect()
                    } else {
                        Vec::new()
                    }
                }
            }
            (ReplyTag::Peers, Response::Peers { value }) => {
                self.peers = value;
                Vec::new()
            }
            (ReplyTag::Contacts, Response::Contacts { value }) => {
                self.contacts = value;
                Vec::new()
            }
            (ReplyTag::Requests, Response::Requests { value }) => {
                self.requests = value;
                Vec::new()
            }
            (ReplyTag::Devices, Response::Devices { value }) => {
                self.devices = value;
                Vec::new()
            }
            (ReplyTag::Files { append }, Response::Files { value }) => {
                if append {
                    self.files.extend(value);
                } else {
                    self.files = value;
                }
                Vec::new()
            }
            (ReplyTag::Settings, Response::Settings { value }) => {
                self.settings = Some(value);
                Vec::new()
            }
            (
                ReplyTag::Pair { nonce } | ReplyTag::PairJoin { nonce },
                Response::PairSession { value },
            ) => {
                if nonce != self.pair_nonce || self.pair != Some(PairUi::Starting) {
                    // 等待期间用户已取消（含取消后又重新发起）：迟到的会话不再弹框，
                    // 并通知服务端收掉孤儿会话。只在 nonce 匹配时才清 early 缓存，
                    // 避免清掉属于新会话的先到状态。
                    if nonce == self.pair_nonce {
                        self.pair_early_status = None;
                    }
                    return vec![call(
                        Request::PairCancel {
                            session: value.session,
                        },
                        ReplyTag::Generic,
                    )];
                }
                // 状态事件可能先于回复到达：属于本会话的就地应用
                let state = match self.pair_early_status.take() {
                    Some((s, st)) if s == value.session => st,
                    _ => PairStateDto::WaitingPeer,
                };
                let mut fx = Vec::new();
                if state == PairStateDto::Done {
                    self.wizard_joining = false;
                    fx.push(call(Request::ListMyDevices, ReplyTag::Devices));
                    fx.push(call(Request::GetSelf, ReplyTag::SelfInfo));
                }
                self.pair = Some(PairUi::Showing {
                    session: value.session,
                    qr_uri: value.qr_uri,
                    state,
                });
                fx
            }
            (ReplyTag::Search { query }, Response::Messages { value }) => {
                // 只接受与当前查询词一致的回复，丢弃过期回复
                if query == self.search_query.trim() {
                    self.search_results = value;
                }
                Vec::new()
            }
            (ReplyTag::Init, Response::Ok) => {
                self.init_in_flight = false;
                let mut fx = Vec::new();
                match self.init_mode.take() {
                    Some(InitMode::FirstDevice) => fx.push(Effect::EnableAutostart),
                    Some(InitMode::JoinExisting) => {
                        self.wizard_joining = true;
                        // 直接进入加入流程：选择已有设备或粘贴配对链接
                        fx.extend(self.pair_open_join());
                    }
                    None => {}
                }
                fx.push(call(Request::GetSelf, ReplyTag::SelfInfo));
                fx
            }
            (ReplyTag::Transfer { hash }, Response::TransferId { value }) => {
                self.transfer_ids.insert(hash, value);
                Vec::new()
            }
            (ReplyTag::Deleted { msg }, Response::Ok) => {
                self.messages.retain(|m| m.id != msg);
                Vec::new()
            }
            (ReplyTag::Responded { request }, Response::Ok) => {
                self.pending_requests.remove(&request);
                self.requests.retain(|r| r.request != request);
                Vec::new()
            }
            (ReplyTag::Requested { user }, Response::Ok) => {
                self.pending_contact_requests.remove(&user);
                self.toast = Some(strings::CONTACT_REQUEST_SENT.to_string());
                Vec::new()
            }
            (ReplyTag::Dm, Response::ConvId { value }) => {
                self.page = Page::Chats;
                let mut fx = self.open_conv(value);
                fx.push(call(Request::ListConversations, ReplyTag::Conversations));
                fx
            }
            (ReplyTag::DeviceRemoved, Response::Ok) => {
                vec![call(Request::ListMyDevices, ReplyTag::Devices)]
            }
            (ReplyTag::ManualPeer, Response::Ok) => {
                self.toast = Some(strings::MANUAL_PEER_SENT.to_string());
                vec![call(Request::ListPeers, ReplyTag::Peers)]
            }
            (ReplyTag::Forwarded, _) => {
                self.toast = Some(strings::FORWARDED.to_string());
                Vec::new()
            }
            // Sent（成功：草稿随之丢弃）/ Transfer / Generic 等无需更新模型：结果经事件到达
            _ => Vec::new(),
        }
    }

    fn apply_event(&mut self, ev: Event) -> Vec<Effect> {
        match ev {
            Event::MessageAdded { msg } => {
                if self.current_conv.as_deref() != Some(msg.conv.as_str())
                    || self.messages.iter().any(|m| m.id == msg.id)
                {
                    return Vec::new();
                }
                if self.has_newer {
                    // 窗口末尾不是最新消息：不追加以免造成缺口，滚到底部时重新拉取
                    return Vec::new();
                }
                let id = msg.id.clone();
                let conv = msg.conv.clone();
                self.messages.push(msg);
                if self.messages.len() > MESSAGE_WINDOW_MAX {
                    let excess = self.messages.len() - MESSAGE_WINDOW_MAX;
                    self.messages.drain(..excess);
                    self.has_older = true;
                }
                if self.window_focused {
                    vec![call(
                        Request::MarkRead { conv, upto: id },
                        ReplyTag::Generic,
                    )]
                } else {
                    Vec::new()
                }
            }
            Event::MessageUpdated { msg } => {
                if let Some(slot) = self.messages.iter_mut().find(|m| m.id == msg.id) {
                    *slot = msg;
                }
                Vec::new()
            }
            Event::ConversationUpdated { summary } => {
                match self.convs.iter_mut().find(|c| c.conv == summary.conv) {
                    Some(slot) => *slot = summary,
                    None => self.convs.push(summary),
                }
                sort_convs(&mut self.convs);
                Vec::new()
            }
            Event::TransferProgress {
                transfer,
                hash,
                done,
                total,
                bytes_per_sec,
            } => {
                self.transfer_ids.insert(hash.clone(), transfer);
                self.transfers.insert(
                    hash.clone(),
                    TransferInfo {
                        hash,
                        done,
                        total,
                        bytes_per_sec,
                    },
                );
                Vec::new()
            }
            Event::TransferFinished {
                hash, ok, error, ..
            } => {
                self.transfers.remove(&hash);
                self.transfer_ids.remove(&hash);
                self.pending_downloads.remove(&hash);
                let cancelled = self.cancelled_downloads.remove(&hash);
                if !ok && !cancelled {
                    let why = error.unwrap_or_else(|| strings::UNKNOWN_ERROR.to_string());
                    self.toast = Some(format!("{}{}", strings::DOWNLOAD_FAILED_PREFIX, why));
                }
                Vec::new()
            }
            Event::IncomingRequest { request } => {
                if !self.requests.iter().any(|r| r.request == request.request) {
                    self.requests.push(request);
                }
                Vec::new()
            }
            Event::PeerOnline { peer } => {
                match self
                    .peers
                    .iter_mut()
                    .find(|p| p.device_id == peer.device_id)
                {
                    Some(slot) => *slot = peer,
                    None => self.peers.push(peer),
                }
                Vec::new()
            }
            Event::PeerOffline { device } => {
                self.peers.retain(|p| p.device_id != device);
                Vec::new()
            }
            Event::ContactsChanged => vec![call(Request::ListContacts, ReplyTag::Contacts)],
            Event::DevicesChanged => vec![call(Request::ListMyDevices, ReplyTag::Devices)],
            Event::PairStatus { session, state } => {
                let mut fx = Vec::new();
                if self.pair == Some(PairUi::Starting) {
                    // 回复尚未到达：只缓存最后一个先到状态（取舍：同一会话连发多个状态时
                    // 只保留最新，终态必为最后一个；会话 id 在回复到达时比对，
                    // 不属于本会话的缓存直接丢弃）
                    self.pair_early_status = Some((session, state));
                    return fx;
                }
                if let Some(PairUi::Showing {
                    session: cur,
                    state: cur_state,
                    ..
                }) = self.pair.as_mut()
                {
                    if *cur == session {
                        if state == PairStateDto::Done {
                            // 配对完成：设备列表与本机身份都变了；向导里的加入流程到此结束
                            // （GetSelf 回来后 initialized 为真，页面自动切到会话页）
                            self.wizard_joining = false;
                            fx.push(call(Request::ListMyDevices, ReplyTag::Devices));
                            fx.push(call(Request::GetSelf, ReplyTag::SelfInfo));
                        }
                        *cur_state = state;
                    }
                }
                fx
            }
            _ => Vec::new(),
        }
    }

    /// 打开会话：清空消息窗口并拉取最新一页。
    pub fn open_conv(&mut self, conv: String) -> Vec<Effect> {
        self.current_conv = Some(conv.clone());
        self.messages.clear();
        self.has_older = false;
        self.has_newer = false;
        self.reload_in_flight = false;
        self.older_in_flight = false;
        let mut fx = Vec::new();
        if self.window_focused {
            fx.push(call(
                Request::SetUiFocus {
                    conv: Some(conv.clone()),
                },
                ReplyTag::Generic,
            ));
        }
        fx.push(self.list_messages_call(conv, None, false));
        fx
    }

    /// 加载更早的一页消息（以当前窗口第一条为游标）。
    pub fn load_older(&mut self) -> Vec<Effect> {
        let (Some(conv), Some(first)) = (self.current_conv.clone(), self.messages.first()) else {
            return Vec::new();
        };
        if !self.has_older || self.older_in_flight {
            return Vec::new();
        }
        self.older_in_flight = true;
        vec![self.list_messages_call(conv, Some(first.id.clone()), true)]
    }

    /// 滚到底部时：窗口缺最新消息则重新拉取最新一页（回复会替换窗口）。
    pub fn reload_latest(&mut self) -> Vec<Effect> {
        let Some(conv) = self.current_conv.clone() else {
            return Vec::new();
        };
        if !self.has_newer || self.reload_in_flight {
            return Vec::new();
        }
        // has_newer 在成功回复后才清除：请求失败时提示条仍在，可再次点击
        self.reload_in_flight = true;
        vec![self.list_messages_call(conv, None, false)]
    }

    /// 切换主页面并拉取该页所需数据。
    pub fn set_page(&mut self, p: Page) -> Vec<Effect> {
        self.page = p;
        match p {
            Page::Wizard => Vec::new(),
            Page::Chats => vec![call(Request::ListConversations, ReplyTag::Conversations)],
            Page::Contacts => vec![
                call(Request::ListContacts, ReplyTag::Contacts),
                call(Request::ListRequests, ReplyTag::Requests),
                call(Request::ListPeers, ReplyTag::Peers),
                call(Request::ListMyDevices, ReplyTag::Devices),
            ],
            Page::Files => vec![call(
                Request::QueryFiles {
                    filter: self.file_filter.clone(),
                },
                ReplyTag::Files { append: false },
            )],
            Page::Settings => vec![call(Request::GetSettings, ReplyTag::Settings)],
        }
    }

    /// 窗口焦点变化：失焦时告诉 core 不再关注任何会话，回到前台时恢复并标已读。
    pub fn set_window_focused(&mut self, on: bool) -> Vec<Effect> {
        self.window_focused = on;
        if !on {
            return vec![call(Request::SetUiFocus { conv: None }, ReplyTag::Generic)];
        }
        match self.current_conv.clone() {
            Some(c) => {
                let mut fx = vec![call(
                    Request::SetUiFocus { conv: Some(c) },
                    ReplyTag::Generic,
                )];
                fx.extend(self.mark_read_last());
                fx
            }
            None => Vec::new(),
        }
    }

    /// 搜索消息；空白查询清空结果且不发请求。
    pub fn search(&mut self, query: String) -> Vec<Effect> {
        self.search_query = query;
        let q = self.search_query.trim().to_string();
        if q.is_empty() {
            self.search_results.clear();
            return Vec::new();
        }
        vec![call(
            Request::SearchMessages {
                query: q.clone(),
                conv: None,
                limit: SEARCH_LIMIT,
            },
            ReplyTag::Search { query: q },
        )]
    }

    /// 向导提交：校验显示名后发 `Init`。设备名为空时用默认值。
    pub fn submit_init(
        &mut self,
        display_name: &str,
        device_name: &str,
        mode: InitMode,
    ) -> Vec<Effect> {
        if self.init_in_flight {
            return Vec::new();
        }
        if !wizard_name_ok(display_name) {
            self.toast = Some(strings::WIZARD_NAME_INVALID.to_string());
            return Vec::new();
        }
        // 设备名留空用默认值；非空则同样限 1–32 个字符
        if !device_name.trim().is_empty() && !wizard_name_ok(device_name) {
            self.toast = Some(strings::WIZARD_DEVICE_INVALID.to_string());
            return Vec::new();
        }
        let device = device_name.trim();
        let device = if device.is_empty() {
            strings::DEFAULT_DEVICE_NAME
        } else {
            device
        };
        self.init_mode = Some(mode.clone());
        self.init_in_flight = true;
        vec![call(
            Request::Init {
                display_name: display_name.trim().to_string(),
                device_name: device.to_string(),
                mode,
            },
            ReplyTag::Init,
        )]
    }

    /// 处理陌生人请求（接收一次 / 加为联系人 / 拒绝 / 屏蔽）；成功回复后才从列表移除。
    pub fn respond_request(&mut self, request: String, action: RequestAction) -> Vec<Effect> {
        if !self.pending_requests.insert(request.clone()) {
            return Vec::new();
        }
        vec![call(
            Request::RespondRequest {
                request: request.clone(),
                action,
            },
            ReplyTag::Responded { request },
        )]
    }

    /// 向附近的人发出联系人请求。
    pub fn request_contact(&mut self, user: String) -> Vec<Effect> {
        if !self.pending_contact_requests.insert(user.clone()) {
            return Vec::new();
        }
        vec![call(
            Request::RequestContact { user: user.clone() },
            ReplyTag::Requested { user },
        )]
    }

    /// 保存联系人策略：先本地更新，失败时按服务端重新拉取。
    pub fn set_contact_policy(
        &mut self,
        user: String,
        auto_accept: bool,
        ask_above_bytes: Option<u64>,
    ) -> Vec<Effect> {
        if let Some(c) = self.contacts.iter_mut().find(|c| c.user_id == user) {
            c.auto_accept = auto_accept;
            c.ask_above_bytes = ask_above_bytes;
        }
        vec![call(
            Request::SetContactPolicy {
                user,
                auto_accept,
                ask_above_bytes,
            },
            ReplyTag::Policy,
        )]
    }

    /// 与某联系人发消息：打开（或创建）私聊，回复后切到会话页。
    pub fn open_dm(&mut self, user: String) -> Vec<Effect> {
        vec![call(Request::OpenDm { user }, ReplyTag::Dm)]
    }

    /// 移除本人的另一台设备；本机不可移除。
    pub fn remove_device(&mut self, device: String) -> Vec<Effect> {
        let is_self = self.me.as_ref().is_some_and(|me| me.device_id == device)
            || self
                .devices
                .iter()
                .any(|d| d.device_id == device && d.is_self);
        if is_self {
            self.toast = Some(strings::CANNOT_REMOVE_SELF.to_string());
            return Vec::new();
        }
        vec![call(
            Request::RemoveDevice { device },
            ReplyTag::DeviceRemoved,
        )]
    }

    /// 手动添加对端地址（跨网段 / VPN）。
    pub fn add_manual_peer(&mut self, addr: String) -> Vec<Effect> {
        let addr = match normalize_peer_addr(&addr) {
            Ok(a) => a,
            Err(msg) => {
                self.toast = Some(msg.to_string());
                return Vec::new();
            }
        };
        vec![call(Request::AddManualPeer { addr }, ReplyTag::ManualPeer)]
    }

    /// 切换联系人页分类并刷新该类所需数据。
    pub fn set_contacts_tab(&mut self, tab: usize) -> Vec<Effect> {
        self.contacts_tab = tab.min(CONTACTS_TABS - 1);
        match self.contacts_tab {
            0 => vec![call(Request::ListContacts, ReplyTag::Contacts)],
            1 => vec![call(Request::ListRequests, ReplyTag::Requests)],
            2 => vec![call(Request::ListPeers, ReplyTag::Peers)],
            _ => vec![call(Request::ListMyDevices, ReplyTag::Devices)],
        }
    }

    /// 发起方开始配对（扫码或数字比对）；已有配对界面时忽略。
    pub fn pair_start(&mut self, method: PairMethod) -> Vec<Effect> {
        if self.pair.is_some() {
            return Vec::new();
        }
        self.pair = Some(PairUi::Starting);
        self.pair_early_status = None;
        self.pair_nonce += 1;
        self.pair_intent = Some(PairIntent::Start(method.clone()));
        vec![call(
            Request::PairStart { method },
            ReplyTag::Pair {
                nonce: self.pair_nonce,
            },
        )]
    }

    /// 加入方打开选择界面，并刷新可见设备。
    pub fn pair_open_join(&mut self) -> Vec<Effect> {
        self.pair = Some(PairUi::Picking);
        vec![call(Request::ListPeers, ReplyTag::Peers)]
    }

    /// 加入方选定目标（设备或配对链接）；只在选择界面有效。
    pub fn pair_join(&mut self, target: PairTarget) -> Vec<Effect> {
        if self.pair != Some(PairUi::Picking) {
            return Vec::new();
        }
        if matches!(&target, PairTarget::QrUri { uri } if !pair_link_ok(uri)) {
            return Vec::new();
        }
        self.pair = Some(PairUi::Starting);
        self.pair_early_status = None;
        self.pair_nonce += 1;
        self.pair_intent = Some(PairIntent::Join(target.clone()));
        vec![call(
            Request::PairJoin { target },
            ReplyTag::PairJoin {
                nonce: self.pair_nonce,
            },
        )]
    }

    /// 数字比对结果；“不一致”时本地立即显示失败，不等服务端事件。
    pub fn pair_confirm(&mut self, matches: bool) -> Vec<Effect> {
        let Some(PairUi::Showing { session, state, .. }) = self.pair.as_mut() else {
            return Vec::new();
        };
        let session = session.clone();
        // 仅在等待比对时有效；点“一致”后本地置为验证中，避免先一致后又点不一致
        if !matches!(state, PairStateDto::ShowSas { .. }) {
            return Vec::new();
        }
        *state = if matches {
            PairStateDto::Verifying
        } else {
            PairStateDto::Failed {
                reason: strings::PAIR_SAS_MISMATCH.to_string(),
            }
        };
        vec![call(
            Request::PairConfirm { session, matches },
            ReplyTag::Generic,
        )]
    }

    /// 取消配对并关闭界面；向导里的加入流程随之回到填写表单。
    pub fn pair_cancel(&mut self) -> Vec<Effect> {
        let fx = match self.pair.take() {
            // 已结束（完成 / 失败）的会话无需再取消
            Some(PairUi::Showing { session, state, .. })
                if !matches!(state, PairStateDto::Done | PairStateDto::Failed { .. }) =>
            {
                vec![call(Request::PairCancel { session }, ReplyTag::Generic)]
            }
            _ => Vec::new(),
        };
        self.pair_intent = None;
        self.pair_early_status = None;
        if self.wizard_joining {
            self.wizard_joining = false;
            self.init_mode = None;
        }
        fx
    }

    /// 配对完成后关闭界面（会话已结束，不再发取消）。
    pub fn pair_close(&mut self) -> Vec<Effect> {
        self.pair = None;
        self.pair_intent = None;
        self.pair_early_status = None;
        Vec::new()
    }

    /// 配对失败后重试：发起方重新开始同一方式，加入方回到选择界面。
    pub fn pair_retry(&mut self) -> Vec<Effect> {
        match self.pair_intent.clone() {
            Some(PairIntent::Start(method)) => {
                self.pair = None;
                self.pair_start(method)
            }
            Some(PairIntent::Join(_)) => self.pair_open_join(),
            None => Vec::new(),
        }
    }

    /// 加入等待界面点“返回”：回到填写表单。
    pub fn wizard_back(&mut self) -> Vec<Effect> {
        self.wizard_joining = false;
        self.init_mode = None;
        Vec::new()
    }

    /// 会话列表的展示顺序：“我的设备”置顶，其余保持 `convs` 的时间降序。
    pub fn ordered_convs(&self) -> Vec<&ConversationSummary> {
        let (mut pinned, rest): (Vec<_>, Vec<_>) = self
            .convs
            .iter()
            .partition(|c| c.kind == ConvKind::SelfDevices);
        pinned.extend(rest);
        pinned
    }

    /// 点击搜索结果：清空搜索并打开该会话。
    pub fn open_from_search(&mut self, conv: String) -> Vec<Effect> {
        self.search_query.clear();
        self.search_results.clear();
        self.open_conv(conv)
    }

    /// 下载一个文件；回复里的传输 id 用于取消。
    pub fn download(&mut self, hash: String) -> Vec<Effect> {
        if self.pending_downloads.contains(&hash) || self.transfers.contains_key(&hash) {
            return Vec::new();
        }
        self.pending_downloads.insert(hash.clone());
        vec![call(
            Request::DownloadBlob { hash: hash.clone() },
            ReplyTag::Transfer { hash },
        )]
    }

    /// 下载一组文件（文件夹消息）。
    pub fn download_all(&mut self, hashes: Vec<String>) -> Vec<Effect> {
        hashes.into_iter().flat_map(|h| self.download(h)).collect()
    }

    /// 取消某个文件的传输；还不知道传输 id（请求尚未回复）时不发请求。
    pub fn cancel_transfer(&mut self, hash: &str) -> Vec<Effect> {
        match self.transfer_ids.get(hash) {
            Some(t) => {
                self.cancelled_downloads.insert(hash.to_string());
                vec![call(
                    Request::CancelTransfer {
                        transfer: t.clone(),
                    },
                    ReplyTag::Generic,
                )]
            }
            None => Vec::new(),
        }
    }

    /// 撤回自己发的消息；结果经 MessageUpdated 事件到达。
    pub fn recall(&mut self, msg: String) -> Vec<Effect> {
        vec![call(Request::Recall { msg }, ReplyTag::Generic)]
    }

    /// 删除本地副本；成功回复后才从窗口移除。
    pub fn delete_local(&mut self, msg: String) -> Vec<Effect> {
        vec![call(
            Request::DeleteLocal {
                msgs: vec![msg.clone()],
                also_my_devices: false,
            },
            ReplyTag::Deleted { msg },
        )]
    }

    /// 发送前的公共前缀：窗口缺最新消息时先重新拉取最新一页，否则自己发出的消息不会出现。
    fn send_prefix(&mut self) -> Vec<Effect> {
        self.reload_latest()
    }

    /// 发送文字（调用方已做空白规整）；结果经 MessageAdded 事件与错误 toast 到达。
    pub fn send_text(&mut self, conv: String, text: String) -> Vec<Effect> {
        let draft = text.clone();
        let mut fx = self.send_prefix();
        fx.push(call(
            Request::SendText {
                conv: conv.clone(),
                text,
            },
            ReplyTag::Sent {
                conv,
                draft: Some(draft),
            },
        ));
        fx
    }

    /// 发送一组文件（一条消息）。
    pub fn send_files(&mut self, conv: String, paths: Vec<String>) -> Vec<Effect> {
        let mut fx = self.send_prefix();
        fx.push(call(
            Request::SendFiles {
                conv: conv.clone(),
                paths,
            },
            ReplyTag::Sent { conv, draft: None },
        ));
        fx
    }

    /// 拖入：文件合并为一条 SendFiles，每个目录各一条 SendFolder。
    pub fn send_dropped(
        &mut self,
        conv: String,
        files: Vec<String>,
        dirs: Vec<String>,
    ) -> Vec<Effect> {
        if files.is_empty() && dirs.is_empty() {
            return Vec::new();
        }
        let mut fx = self.send_prefix();
        let tag = || ReplyTag::Sent {
            conv: conv.clone(),
            draft: None,
        };
        if !files.is_empty() {
            fx.push(call(
                Request::SendFiles {
                    conv: conv.clone(),
                    paths: files,
                },
                tag(),
            ));
        }
        for path in dirs {
            fx.push(call(
                Request::SendFolder {
                    conv: conv.clone(),
                    path,
                },
                tag(),
            ));
        }
        fx
    }

    /// 发送一个文件夹。
    pub fn send_folder(&mut self, conv: String, path: String) -> Vec<Effect> {
        self.send_dropped(conv, Vec::new(), vec![path])
    }

    /// 把已有文件转发到会话。
    pub fn forward_files(&mut self, conv: String, hashes: Vec<String>) -> Vec<Effect> {
        vec![call(
            Request::ForwardFiles { conv, hashes },
            ReplyTag::Forwarded,
        )]
    }
}

fn call(req: Request, tag: ReplyTag) -> Effect {
    Effect::Call(req, tag)
}

/// 联系人策略里“超过 N MB 仍询问”的输入解析：空串表示不限制（`Some(None)`），
/// 非负整数 MB 换算为字节（`Some(Some(n))`），其余非法（`None`）。
pub fn parse_ask_mb(text: &str) -> Option<Option<u64>> {
    let t = text.trim();
    if t.is_empty() {
        return Some(None);
    }
    let mb: u64 = t.parse().ok()?;
    mb.checked_mul(1 << 20).map(Some)
}

/// 向导显示名是否合法：去首尾空白后 1–32 个字符。
pub fn wizard_name_ok(name: &str) -> bool {
    (1..=32).contains(&name.trim().chars().count())
}

/// 按最近消息时间降序（无消息的排最后）。
fn sort_convs(convs: &mut [ConversationSummary]) {
    convs.sort_by_key(|c| std::cmp::Reverse(c.last_hlc_millis));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, conv: &str, text: &str) -> MessageView {
        MessageView {
            id: id.into(),
            conv: conv.into(),
            author_user: "u".into(),
            author_device: "d".into(),
            author_device_name: "dev".into(),
            is_mine: false,
            hlc_millis: 1,
            body: MessageBodyView::Text { text: text.into() },
            state: MessageState::Sent,
        }
    }

    fn summary(conv: &str, ts: Option<i64>) -> ConversationSummary {
        ConversationSummary {
            conv: conv.into(),
            kind: ConvKind::Dm,
            title: conv.into(),
            last_text: None,
            last_hlc_millis: ts,
            unread: 0,
            peer_online: true,
        }
    }

    fn self_info(initialized: bool) -> SelfInfo {
        SelfInfo {
            initialized,
            user_id: None,
            user_short: None,
            display_name: None,
            device_id: "d".into(),
            device_name: None,
            status: StatusDto::Online,
        }
    }

    fn request(id: &str) -> RequestView {
        RequestView {
            request: id.into(),
            kind: RequestKind::Contact,
            from_user: "u".into(),
            from_name: "张三".into(),
            from_short: "ab".into(),
            preview: String::new(),
            created_at: 1,
        }
    }

    fn reply(tag: ReplyTag, r: Response) -> UiMsg {
        UiMsg::Reply { tag, result: Ok(r) }
    }

    fn calls(effects: &[Effect]) -> Vec<&Request> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Call(r, _) => Some(r),
                Effect::EnableAutostart => None,
            })
            .collect()
    }

    fn has(effects: &[Effect], pred: impl Fn(&Request) -> bool) -> bool {
        calls(effects).into_iter().any(pred)
    }

    /// 打开会话 c 并灌入 n 条消息（id 为 "m0".."m{n-1}"）。
    fn model_in_conv(c: &str, n: usize) -> Model {
        let mut m = Model::new();
        m.open_conv(c.into());
        let msgs: Vec<_> = (0..n).map(|i| msg(&format!("m{i}"), c, "x")).collect();
        m.apply(reply(
            ReplyTag::Messages {
                conv: c.into(),
                older: false,
            },
            Response::Messages { value: msgs },
        ));
        m
    }

    #[test]
    fn rule01_connected_requests_initial_data() {
        let mut m = Model::new();
        let fx = m.apply(UiMsg::Connected);
        assert_eq!(m.conn, ConnState::Connected);
        assert!(has(&fx, |r| matches!(r, Request::GetSelf)));
        assert!(has(&fx, |r| matches!(r, Request::ListConversations)));
        assert!(has(&fx, |r| matches!(r, Request::ListRequests)));
        assert!(has(&fx, |r| matches!(r, Request::GetSettings)));
        // 没有当前会话时不会拉消息
        assert!(!has(&fx, |r| matches!(r, Request::ListMessages { .. })));
        // Resync 同上，有当前会话时额外重拉消息
        let mut m = model_in_conv("c1", 2);
        let fx = m.apply(UiMsg::Resync);
        assert!(has(&fx, |r| matches!(r, Request::GetSelf)));
        assert!(has(
            &fx,
            |r| matches!(r, Request::ListMessages { conv, .. } if conv == "c1")
        ));
    }

    #[test]
    fn rule02_uninitialized_self_goes_to_wizard() {
        let mut m = Model::new();
        m.apply(reply(
            ReplyTag::SelfInfo,
            Response::SelfInfo {
                value: self_info(false),
            },
        ));
        assert_eq!(m.page, Page::Wizard);
        m.apply(reply(
            ReplyTag::SelfInfo,
            Response::SelfInfo {
                value: self_info(true),
            },
        ));
        assert_eq!(m.page, Page::Chats);
        // 已在其它页时不被拉回会话页
        m.page = Page::Settings;
        m.apply(reply(
            ReplyTag::SelfInfo,
            Response::SelfInfo {
                value: self_info(true),
            },
        ));
        assert_eq!(m.page, Page::Settings);
    }

    #[test]
    fn rule03_open_conv_loads_and_marks_read() {
        let mut m = Model::new();
        m.messages.push(msg("old", "x", "old"));
        let fx = m.open_conv("c1".into());
        assert_eq!(m.current_conv.as_deref(), Some("c1"));
        assert!(m.messages.is_empty());
        assert!(has(&fx, |r| matches!(r,
            Request::ListMessages { conv, before: None, limit } if conv == "c1" && *limit == MESSAGE_PAGE)));
        assert!(has(&fx, |r| matches!(r,
            Request::SetUiFocus { conv: Some(c) } if c == "c1")));
        // 回复到达后对最后一条 MarkRead
        let fx = m.apply(reply(
            ReplyTag::Messages {
                conv: "c1".into(),
                older: false,
            },
            Response::Messages {
                value: vec![msg("a", "c1", "1"), msg("b", "c1", "2")],
            },
        ));
        assert!(has(&fx, |r| matches!(r,
            Request::MarkRead { conv, upto } if conv == "c1" && upto == "b")));
        // 窗口无焦点时不发 SetUiFocus
        let mut m = Model::new();
        m.set_window_focused(false);
        let fx = m.open_conv("c2".into());
        assert!(!has(&fx, |r| matches!(r, Request::SetUiFocus { .. })));
    }

    #[test]
    fn rule04_messages_reply_replace_prepend_and_window() {
        let mut m = model_in_conv("c1", MESSAGE_PAGE as usize);
        assert!(m.has_older, "满一页说明可能还有更早的");
        // 少于一页：没有更早的了
        m.apply(reply(
            ReplyTag::Messages {
                conv: "c1".into(),
                older: false,
            },
            Response::Messages {
                value: vec![msg("only", "c1", "x")],
            },
        ));
        assert_eq!(m.messages.len(), 1);
        assert!(!m.has_older);
        // 前插
        m.apply(reply(
            ReplyTag::Messages {
                conv: "c1".into(),
                older: true,
            },
            Response::Messages {
                value: vec![msg("e1", "c1", "x"), msg("e2", "c1", "x")],
            },
        ));
        let ids: Vec<_> = m.messages.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["e1", "e2", "only"]);
        // 前插超过窗口上限：从末尾丢弃
        let mut m = model_in_conv("c1", MESSAGE_WINDOW_MAX);
        let older: Vec<_> = (0..MESSAGE_PAGE as usize)
            .map(|i| msg(&format!("o{i}"), "c1", "x"))
            .collect();
        m.apply(reply(
            ReplyTag::Messages {
                conv: "c1".into(),
                older: true,
            },
            Response::Messages { value: older },
        ));
        assert_eq!(m.messages.len(), MESSAGE_WINDOW_MAX);
        assert_eq!(m.messages[0].id, "o0");
        assert!(m.has_older);
        assert!(m.has_newer, "截掉了最新消息，窗口与最新之间有缺口");
        // 其它会话的回复被丢弃
        let before = m.messages.clone();
        m.apply(reply(
            ReplyTag::Messages {
                conv: "other".into(),
                older: false,
            },
            Response::Messages {
                value: vec![msg("z", "other", "x")],
            },
        ));
        assert_eq!(m.messages, before);
    }

    #[test]
    fn rule05_message_added_appends_trims_and_marks_read() {
        let mut m = model_in_conv("c1", 2);
        let fx = m.apply(UiMsg::Event(Event::MessageAdded {
            msg: msg("n1", "c1", "hi"),
        }));
        assert_eq!(m.messages.len(), 3);
        assert!(has(&fx, |r| matches!(r,
            Request::MarkRead { upto, .. } if upto == "n1")));
        // 其它会话的消息不进当前窗口
        m.apply(UiMsg::Event(Event::MessageAdded {
            msg: msg("x1", "other", "hi"),
        }));
        assert_eq!(m.messages.len(), 3);
        // 窗口无焦点：不 MarkRead
        m.set_window_focused(false);
        let fx = m.apply(UiMsg::Event(Event::MessageAdded {
            msg: msg("n2", "c1", "hi"),
        }));
        assert!(!has(&fx, |r| matches!(r, Request::MarkRead { .. })));
        // 超过上限：从头部丢弃并置 has_older
        let mut m = model_in_conv("c1", MESSAGE_WINDOW_MAX);
        m.has_older = false;
        m.apply(UiMsg::Event(Event::MessageAdded {
            msg: msg("new", "c1", "hi"),
        }));
        assert_eq!(m.messages.len(), MESSAGE_WINDOW_MAX);
        assert_eq!(m.messages[0].id, "m1");
        assert_eq!(m.messages.last().map(|x| x.id.as_str()), Some("new"));
        assert!(m.has_older);
    }

    #[test]
    fn prepend_overflow_sets_has_newer_and_suppresses_append() {
        let mut m = model_in_conv("c1", MESSAGE_WINDOW_MAX);
        assert!(!m.has_newer);
        let older: Vec<_> = (0..MESSAGE_PAGE as usize)
            .map(|i| msg(&format!("o{i}"), "c1", "x"))
            .collect();
        m.apply(reply(
            ReplyTag::Messages {
                conv: "c1".into(),
                older: true,
            },
            Response::Messages { value: older },
        ));
        assert!(m.has_newer);
        let fx = m.apply(UiMsg::Event(Event::MessageAdded {
            msg: msg("fresh", "c1", "hi"),
        }));
        assert!(fx.is_empty());
        assert!(
            m.messages.iter().all(|x| x.id != "fresh"),
            "不追加到有缺口的窗口"
        );
    }

    #[test]
    fn scroll_bottom_with_has_newer_reloads_latest() {
        let mut m = model_in_conv("c1", 3);
        assert!(m.reload_latest().is_empty(), "无缺口不重载");
        m.has_newer = true;
        let fx = m.reload_latest();
        assert!(m.has_newer, "回复成功前保持缺口标记");
        assert!(has(&fx, |r| matches!(r,
            Request::ListMessages { conv, before: None, limit } if conv == "c1" && *limit == MESSAGE_PAGE)));
        assert!(m.reload_latest().is_empty(), "不重复请求");
        // 回复（older:false）替换窗口
        m.apply(reply(
            ReplyTag::Messages {
                conv: "c1".into(),
                older: false,
            },
            Response::Messages {
                value: vec![msg("latest", "c1", "x")],
            },
        ));
        assert_eq!(m.messages.len(), 1);
        assert!(!m.has_newer);
    }

    #[test]
    fn rule06_message_updated_replaces_in_place() {
        let mut m = model_in_conv("c1", 3);
        let mut upd = msg("m1", "c1", "changed");
        upd.state = MessageState::Read;
        m.apply(UiMsg::Event(Event::MessageUpdated { msg: upd.clone() }));
        assert_eq!(m.messages[1], upd);
        assert_eq!(m.messages.len(), 3);
        // 不存在则忽略
        m.apply(UiMsg::Event(Event::MessageUpdated {
            msg: msg("nope", "c1", "x"),
        }));
        assert_eq!(m.messages.len(), 3);
    }

    #[test]
    fn rule07_conversation_updated_sorted_by_recent() {
        let mut m = Model::new();
        for (c, t) in [("a", Some(10)), ("b", Some(30)), ("c", None)] {
            m.apply(UiMsg::Event(Event::ConversationUpdated {
                summary: summary(c, t),
            }));
        }
        let order: Vec<_> = m.convs.iter().map(|c| c.conv.as_str()).collect();
        assert_eq!(order, ["b", "a", "c"]);
        // 替换并重排
        let mut s = summary("a", Some(99));
        s.unread = 5;
        m.apply(UiMsg::Event(Event::ConversationUpdated { summary: s }));
        let order: Vec<_> = m.convs.iter().map(|c| c.conv.as_str()).collect();
        assert_eq!(order, ["a", "b", "c"]);
        assert_eq!(m.convs.len(), 3);
        assert_eq!(m.convs[0].unread, 5);
    }

    #[test]
    fn rule08_transfer_progress_and_finished() {
        let mut m = Model::new();
        m.apply(UiMsg::Event(Event::TransferProgress {
            transfer: "t".into(),
            hash: "h1".into(),
            done: 5,
            total: 10,
            bytes_per_sec: 2,
        }));
        assert_eq!(
            m.transfers.get("h1"),
            Some(&TransferInfo {
                hash: "h1".into(),
                done: 5,
                total: 10,
                bytes_per_sec: 2
            })
        );
        m.apply(UiMsg::Event(Event::TransferFinished {
            transfer: "t".into(),
            hash: "h1".into(),
            ok: true,
            error: None,
        }));
        assert!(m.transfers.is_empty());
        assert_eq!(m.toast, None);
        m.apply(UiMsg::Event(Event::TransferFinished {
            transfer: "t".into(),
            hash: "h2".into(),
            ok: false,
            error: Some("磁盘已满".into()),
        }));
        assert_eq!(m.toast.as_deref(), Some("下载失败：磁盘已满"));
    }

    #[test]
    fn rule09_requests_contacts_devices_events() {
        let mut m = Model::new();
        m.apply(UiMsg::Event(Event::IncomingRequest {
            request: request("r1"),
        }));
        m.apply(UiMsg::Event(Event::IncomingRequest {
            request: request("r1"),
        }));
        assert_eq!(m.requests.len(), 1, "按 request id 去重");
        let fx = m.apply(UiMsg::Event(Event::ContactsChanged));
        assert!(has(&fx, |r| matches!(r, Request::ListContacts)));
        let fx = m.apply(UiMsg::Event(Event::DevicesChanged));
        assert!(has(&fx, |r| matches!(r, Request::ListMyDevices)));
    }

    #[test]
    fn rule10_pair_status_updates_state() {
        let mut m = Model::new();
        m.pair_start(PairMethod::Qr);
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            Response::PairSession {
                value: PairSessionInfo {
                    session: "s1".into(),
                    qr_uri: Some("lanmsg://x".into()),
                },
            },
        ));
        m.apply(UiMsg::Event(Event::PairStatus {
            session: "s1".into(),
            state: PairStateDto::ShowSas {
                code: "123456".into(),
            },
        }));
        match &m.pair {
            Some(PairUi::Showing { state, qr_uri, .. }) => {
                assert_eq!(
                    state,
                    &PairStateDto::ShowSas {
                        code: "123456".into()
                    }
                );
                assert_eq!(qr_uri.as_deref(), Some("lanmsg://x"));
            }
            _ => panic!("pair 应已建立"),
        }
        // 其它会话的状态不影响
        m.apply(UiMsg::Event(Event::PairStatus {
            session: "other".into(),
            state: PairStateDto::Done,
        }));
        assert!(matches!(&m.pair, Some(PairUi::Showing { state, .. })
            if matches!(state, PairStateDto::ShowSas { .. })));
    }

    #[test]
    fn rule11_window_focus_sets_ui_focus() {
        let mut m = model_in_conv("c1", 1);
        let fx = m.set_window_focused(false);
        assert!(has(&fx, |r| matches!(
            r,
            Request::SetUiFocus { conv: None }
        )));
        let fx = m.set_window_focused(true);
        assert!(has(&fx, |r| matches!(r,
            Request::SetUiFocus { conv: Some(c) } if c == "c1")));
        // 没有当前会话：回到前台不发 SetUiFocus
        let mut m = Model::new();
        let fx = m.set_window_focused(true);
        assert!(!has(&fx, |r| matches!(r, Request::SetUiFocus { .. })));
    }

    #[test]
    fn rule12_disconnect_keeps_data() {
        let mut m = model_in_conv("c1", 2);
        m.apply(UiMsg::Event(Event::ConversationUpdated {
            summary: summary("c1", Some(1)),
        }));
        m.apply(UiMsg::Disconnected("连接被重置".into()));
        assert_eq!(m.conn, ConnState::Disconnected("连接被重置".into()));
        assert_eq!(m.messages.len(), 2);
        assert_eq!(m.convs.len(), 1);
        assert_eq!(m.current_conv.as_deref(), Some("c1"));
    }

    #[test]
    fn rule13_error_reply_sets_toast() {
        let mut m = Model::new();
        m.apply(UiMsg::Reply {
            tag: ReplyTag::Generic,
            result: Err(ApiError {
                code: ErrorCode::PeerOffline,
                message: "对方不在线".into(),
            }),
        });
        assert_eq!(m.toast.as_deref(), Some("对方不在线"));
    }

    #[test]
    fn send_failure_restores_draft() {
        let mut m = Model::new();
        let fx = m.send_text("c1".into(), "你好\n世界".into());
        let tag = fx
            .iter()
            .find_map(|e| match e {
                Effect::Call(Request::SendText { .. }, t) => Some(t.clone()),
                _ => None,
            })
            .expect("有发送请求");
        m.apply(UiMsg::Reply {
            tag,
            result: Err(ApiError {
                code: ErrorCode::PeerOffline,
                message: "对方离线".into(),
            }),
        });
        assert_eq!(
            m.draft_restore,
            Some(("c1".to_string(), "你好\n世界".to_string()))
        );
    }

    #[test]
    fn send_success_discards_draft() {
        let mut m = Model::new();
        m.apply(UiMsg::Reply {
            tag: ReplyTag::Sent {
                conv: "c1".into(),
                draft: Some("x".into()),
            },
            result: Ok(Response::Ok),
        });
        assert_eq!(m.draft_restore, None);
    }

    #[test]
    fn io_error_while_disconnected_does_not_toast() {
        let mut m = Model::new();
        m.apply(UiMsg::Disconnected("断开".into()));
        m.apply(UiMsg::Reply {
            tag: ReplyTag::Generic,
            result: Err(ApiError {
                code: ErrorCode::Io,
                message: "未连接".into(),
            }),
        });
        assert_eq!(m.toast, None);
    }

    #[test]
    fn rule14_search_ignores_stale_reply() {
        let mut m = Model::new();
        let fx = m.search("  ".into());
        assert!(fx.is_empty());
        assert!(m.search_results.is_empty());
        let fx = m.search(" hello ".into());
        assert_eq!(
            fx,
            vec![Effect::Call(
                Request::SearchMessages {
                    query: "hello".into(),
                    conv: None,
                    limit: 50
                },
                ReplyTag::Search {
                    query: "hello".into()
                }
            )]
        );
        // 过期回复（查询词已变）被丢弃
        m.search("world".into());
        m.apply(reply(
            ReplyTag::Search {
                query: "hello".into(),
            },
            Response::Messages {
                value: vec![msg("s1", "c", "hello")],
            },
        ));
        assert!(m.search_results.is_empty());
        m.apply(reply(
            ReplyTag::Search {
                query: "world".into(),
            },
            Response::Messages {
                value: vec![msg("s2", "c", "world")],
            },
        ));
        assert_eq!(m.search_results.len(), 1);
        // 清空查询词会清掉结果
        m.search(String::new());
        assert!(m.search_results.is_empty());
    }

    #[test]
    fn rule15_forward_sets_toast() {
        let mut m = Model::new();
        let fx = m.forward_files("c1".into(), vec!["h1".into()]);
        assert_eq!(
            fx,
            vec![Effect::Call(
                Request::ForwardFiles {
                    conv: "c1".into(),
                    hashes: vec!["h1".into()]
                },
                ReplyTag::Forwarded
            )]
        );
        m.apply(reply(
            ReplyTag::Forwarded,
            Response::MsgId { value: "x".into() },
        ));
        assert_eq!(m.toast.as_deref(), Some("已转发"));
    }

    #[test]
    fn disconnect_then_reconnect_triggers_full_refresh() {
        let mut m = model_in_conv("c1", 3);
        m.apply(UiMsg::Disconnected("断开".into()));
        assert_eq!(m.messages.len(), 3, "断开期间数据保留");
        assert!(matches!(m.conn, ConnState::Disconnected(_)));
        let fx = m.apply(UiMsg::Connected);
        assert_eq!(m.conn, ConnState::Connected);
        assert!(has(&fx, |r| matches!(r, Request::GetSelf)));
        assert!(has(&fx, |r| matches!(r, Request::ListConversations)));
        assert!(has(&fx, |r| matches!(r,
            Request::ListMessages { conv, .. } if conv == "c1")));
    }

    #[test]
    fn message_added_is_idempotent() {
        let mut m = model_in_conv("c1", 0);
        let ev = Event::MessageAdded {
            msg: msg("dup", "c1", "hi"),
        };
        m.apply(UiMsg::Event(ev.clone()));
        m.apply(UiMsg::Event(ev));
        assert_eq!(m.messages.len(), 1);
    }

    #[test]
    fn submit_init_rejects_bad_names_with_toast() {
        for bad in ["", "   ", &"长".repeat(33)] {
            let mut m = Model::new();
            let fx = m.submit_init(bad, "dev", InitMode::FirstDevice);
            assert!(fx.is_empty(), "{bad:?}");
            assert_eq!(m.toast.as_deref(), Some(strings::WIZARD_NAME_INVALID));
            assert_eq!(m.init_mode, None);
        }
    }

    #[test]
    fn submit_init_trims_and_defaults_device_name() {
        let mut m = Model::new();
        let fx = m.submit_init("  小李 ", "  ", InitMode::FirstDevice);
        assert_eq!(
            fx,
            vec![Effect::Call(
                Request::Init {
                    display_name: "小李".into(),
                    device_name: strings::DEFAULT_DEVICE_NAME.into(),
                    mode: InitMode::FirstDevice,
                },
                ReplyTag::Init
            )]
        );
        assert!(wizard_name_ok(&"长".repeat(32)));
    }

    #[test]
    fn init_reply_first_device_enables_autostart_and_refreshes_self() {
        let mut m = Model::new();
        m.submit_init("小李", "pc", InitMode::FirstDevice);
        let fx = m.apply(reply(ReplyTag::Init, Response::Ok));
        assert!(fx.contains(&Effect::EnableAutostart));
        assert!(has(&fx, |r| matches!(r, Request::GetSelf)));
        assert!(!m.wizard_joining);
        assert_eq!(m.init_mode, None);
    }

    #[test]
    fn init_reply_join_existing_marks_joining_without_autostart() {
        let mut m = Model::new();
        m.submit_init("小李", "pc", InitMode::JoinExisting);
        let fx = m.apply(reply(ReplyTag::Init, Response::Ok));
        assert!(!fx.contains(&Effect::EnableAutostart));
        assert!(has(&fx, |r| matches!(r, Request::GetSelf)));
        assert!(m.wizard_joining);
    }

    #[test]
    fn wizard_back_returns_to_form() {
        let mut m = Model::new();
        m.submit_init("小李", "pc", InitMode::JoinExisting);
        m.apply(reply(ReplyTag::Init, Response::Ok));
        assert!(m.wizard_joining);
        let fx = m.wizard_back();
        assert!(fx.is_empty());
        assert!(!m.wizard_joining);
        assert_eq!(m.init_mode, None);
    }

    #[test]
    fn init_error_reply_clears_state_without_autostart() {
        for mode in [InitMode::FirstDevice, InitMode::JoinExisting] {
            let mut m = Model::new();
            m.submit_init("小李", "pc", mode);
            let fx = m.apply(UiMsg::Reply {
                tag: ReplyTag::Init,
                result: Err(ApiError {
                    code: ErrorCode::Io,
                    message: "初始化失败".into(),
                }),
            });
            assert!(fx.is_empty());
            assert!(!m.wizard_joining);
            assert!(!m.init_in_flight);
            assert_eq!(m.init_mode, None);
            assert_eq!(m.toast.as_deref(), Some("初始化失败"));
        }
    }

    #[test]
    fn init_in_flight_blocks_duplicate_submit_until_reply() {
        let mut m = Model::new();
        assert_eq!(m.submit_init("小李", "pc", InitMode::FirstDevice).len(), 1);
        assert!(m.init_in_flight);
        assert!(m
            .submit_init("小李", "pc", InitMode::FirstDevice)
            .is_empty());
        m.apply(reply(ReplyTag::Init, Response::Ok));
        assert!(!m.init_in_flight);
        assert_eq!(m.submit_init("小李", "pc", InitMode::FirstDevice).len(), 1);
    }

    #[test]
    fn submit_init_rejects_too_long_device_name() {
        let mut m = Model::new();
        let fx = m.submit_init("小李", &"长".repeat(33), InitMode::FirstDevice);
        assert!(fx.is_empty());
        assert!(!m.init_in_flight);
        assert_eq!(m.toast.as_deref(), Some(strings::WIZARD_DEVICE_INVALID));
    }

    #[test]
    fn wizard_to_chats_requests_conversations() {
        let mut m = Model::new();
        m.apply(reply(
            ReplyTag::SelfInfo,
            Response::SelfInfo {
                value: self_info(false),
            },
        ));
        assert_eq!(m.page, Page::Wizard);
        let fx = m.apply(reply(
            ReplyTag::SelfInfo,
            Response::SelfInfo {
                value: self_info(true),
            },
        ));
        assert_eq!(m.page, Page::Chats);
        assert!(has(&fx, |r| matches!(r, Request::ListConversations)));
        // 已在会话页时再收到 SelfInfo 不重复拉取
        let fx = m.apply(reply(
            ReplyTag::SelfInfo,
            Response::SelfInfo {
                value: self_info(true),
            },
        ));
        assert!(fx.is_empty());
    }

    #[test]
    fn ordered_convs_pins_self_devices_first() {
        let mut m = Model::new();
        let mut me = summary("me", Some(1));
        me.kind = ConvKind::SelfDevices;
        m.convs = vec![summary("a", Some(9)), summary("b", Some(5)), me];
        let ids: Vec<_> = m.ordered_convs().iter().map(|c| c.conv.clone()).collect();
        assert_eq!(ids, ["me", "a", "b"]);
    }

    #[test]
    fn open_from_search_clears_search_and_opens_conv() {
        let mut m = Model::new();
        m.search("你好".into());
        m.search_results = vec![msg("m1", "c9", "你好")];
        let fx = m.open_from_search("c9".into());
        assert_eq!(m.search_query, "");
        assert!(m.search_results.is_empty());
        assert_eq!(m.current_conv.as_deref(), Some("c9"));
        assert!(has(
            &fx,
            |r| matches!(r, Request::ListMessages { conv, .. } if conv == "c9")
        ));
    }

    // ---- Task 12：下载、取消、撤回、删除、加载更早、对端列表 ----

    fn progress_event(transfer: &str, hash: &str) -> UiMsg {
        UiMsg::Event(Event::TransferProgress {
            transfer: transfer.into(),
            hash: hash.into(),
            done: 1,
            total: 10,
            bytes_per_sec: 1,
        })
    }

    #[test]
    fn download_requests_blob_with_hash_tag() {
        let mut m = Model::new();
        let fx = m.download("h1".into());
        assert_eq!(
            fx,
            vec![Effect::Call(
                Request::DownloadBlob { hash: "h1".into() },
                ReplyTag::Transfer { hash: "h1".into() }
            )]
        );
    }

    #[test]
    fn download_reply_records_transfer_id_and_cancel_uses_it() {
        let mut m = Model::new();
        m.apply(reply(
            ReplyTag::Transfer { hash: "h1".into() },
            Response::TransferId { value: "t9".into() },
        ));
        let fx = m.cancel_transfer("h1");
        assert!(has(
            &fx,
            |r| matches!(r, Request::CancelTransfer { transfer } if transfer == "t9")
        ));
    }

    #[test]
    fn cancel_uses_id_from_progress_event_and_unknown_is_noop() {
        let mut m = Model::new();
        assert!(m.cancel_transfer("h1").is_empty(), "未知传输不发请求");
        m.apply(progress_event("t3", "h1"));
        let fx = m.cancel_transfer("h1");
        assert!(has(
            &fx,
            |r| matches!(r, Request::CancelTransfer { transfer } if transfer == "t3")
        ));
    }

    #[test]
    fn transfer_finished_forgets_transfer_id() {
        let mut m = Model::new();
        m.apply(progress_event("t3", "h1"));
        m.apply(UiMsg::Event(Event::TransferFinished {
            transfer: "t3".into(),
            hash: "h1".into(),
            ok: true,
            error: None,
        }));
        assert!(m.cancel_transfer("h1").is_empty());
    }

    #[test]
    fn duplicate_download_sends_one_request() {
        let mut m = Model::new();
        assert_eq!(m.download("h1".into()).len(), 1);
        assert!(m.download("h1".into()).is_empty(), "已在等待中");
        m.apply(progress_event("t1", "h2"));
        assert!(m.download("h2".into()).is_empty(), "已在传输中");
    }

    fn remote_item(hash: &str) -> FileItemView {
        FileItemView {
            hash: hash.into(),
            name: "a.bin".into(),
            size: 10,
            mime: "application/octet-stream".into(),
            state: BlobState::Remote,
            local_path: None,
            thumb_path: None,
            width: None,
            height: None,
            duration_ms: None,
            source_label: String::new(),
        }
    }

    #[test]
    fn effective_state_follows_pending_and_transfers() {
        let mut m = Model::new();
        let item = remote_item("h1");
        assert_eq!(effective_state(&item, &m), BlobState::Remote);
        m.download("h1".into());
        assert_eq!(effective_state(&item, &m), BlobState::Downloading);
        // 下载请求被拒：恢复待下载
        m.apply(UiMsg::Reply {
            tag: ReplyTag::Transfer { hash: "h1".into() },
            result: Err(ApiError {
                code: ErrorCode::Io,
                message: "x".into(),
            }),
        });
        assert_eq!(effective_state(&item, &m), BlobState::Remote);
        // 进度事件到达
        m.apply(progress_event("t1", "h1"));
        assert_eq!(effective_state(&item, &m), BlobState::Downloading);
        // 失败结束：恢复待下载
        m.apply(UiMsg::Event(Event::TransferFinished {
            transfer: "t1".into(),
            hash: "h1".into(),
            ok: false,
            error: Some("断开".into()),
        }));
        assert_eq!(effective_state(&item, &m), BlobState::Remote);
        // 其它状态不被覆盖
        let mut local = remote_item("h1");
        local.state = BlobState::Local;
        m.download("h1".into());
        assert_eq!(effective_state(&local, &m), BlobState::Local);
    }

    #[test]
    fn cancelled_transfer_finish_does_not_toast() {
        let mut m = Model::new();
        m.apply(progress_event("t1", "h1"));
        m.cancel_transfer("h1");
        m.apply(UiMsg::Event(Event::TransferFinished {
            transfer: "t1".into(),
            hash: "h1".into(),
            ok: false,
            error: Some("已取消".into()),
        }));
        assert_eq!(m.toast, None);
        assert!(m.cancelled_downloads.is_empty());
    }

    #[test]
    fn download_all_emits_one_call_per_hash() {
        let mut m = Model::new();
        let fx = m.download_all(vec!["a".into(), "b".into()]);
        assert_eq!(calls(&fx).len(), 2);
    }

    #[test]
    fn recall_requests_core_recall() {
        let mut m = Model::new();
        let fx = m.recall("m1".into());
        assert!(has(
            &fx,
            |r| matches!(r, Request::Recall { msg } if msg == "m1")
        ));
    }

    #[test]
    fn delete_local_removes_message_only_after_ok_reply() {
        let mut m = model_in_conv("c1", 3);
        let fx = m.delete_local("m1".into());
        assert!(has(&fx, |r| matches!(
            r,
            Request::DeleteLocal { msgs, also_my_devices: false } if msgs == &["m1".to_string()]
        )));
        assert_eq!(m.messages.len(), 3, "回复前不动");
        m.apply(reply(ReplyTag::Deleted { msg: "m1".into() }, Response::Ok));
        let ids: Vec<_> = m.messages.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["m0", "m2"]);
    }

    #[test]
    fn load_older_not_repeated_while_in_flight() {
        let mut m = model_in_conv("c1", MESSAGE_PAGE as usize);
        assert!(m.has_older);
        assert_eq!(m.load_older().len(), 1);
        assert!(m.load_older().is_empty(), "上一页未返回时不重复请求");
        m.apply(reply(
            ReplyTag::Messages {
                conv: "c1".into(),
                older: true,
            },
            Response::Messages {
                value: vec![msg("old", "c1", "x")],
            },
        ));
        assert_eq!(m.messages.first().map(|x| x.id.as_str()), Some("old"));
        assert!(!m.has_older, "不满一页说明已到头");
        assert!(m.load_older().is_empty());
    }

    #[test]
    fn load_older_error_releases_in_flight_flag() {
        let mut m = model_in_conv("c1", MESSAGE_PAGE as usize);
        m.load_older();
        m.apply(UiMsg::Reply {
            tag: ReplyTag::Messages {
                conv: "c1".into(),
                older: true,
            },
            result: Err(ApiError {
                code: ErrorCode::Internal,
                message: "x".into(),
            }),
        });
        assert_eq!(m.load_older().len(), 1);
    }

    #[test]
    fn switching_conv_releases_in_flight_flag() {
        let mut m = model_in_conv("c1", MESSAGE_PAGE as usize);
        m.load_older();
        m.open_conv("c2".into());
        m.apply(reply(
            ReplyTag::Messages {
                conv: "c2".into(),
                older: false,
            },
            Response::Messages {
                value: (0..MESSAGE_PAGE)
                    .map(|i| msg(&format!("n{i}"), "c2", "x"))
                    .collect(),
            },
        ));
        assert_eq!(m.load_older().len(), 1);
    }

    fn peer(device: &str, user: &str, name: &str) -> PeerView {
        PeerView {
            device_id: device.into(),
            device_short: "ab".into(),
            user_id: user.into(),
            display_name: name.into(),
            device_name: format!("{name}的电脑"),
            device_type: "laptop".into(),
            addr: "1.2.3.4:5".into(),
            path: PathClassDto::Lan,
            is_contact: true,
            is_mine: false,
        }
    }

    #[test]
    fn peer_events_upsert_and_remove() {
        let mut m = Model::new();
        m.apply(UiMsg::Event(Event::PeerOnline {
            peer: peer("d1", "u1", "小王"),
        }));
        m.apply(UiMsg::Event(Event::PeerOnline {
            peer: peer("d1", "u1", "小王"),
        }));
        assert_eq!(m.peers.len(), 1, "同一设备不重复");
        m.apply(UiMsg::Event(Event::PeerOffline {
            device: "d1".into(),
        }));
        assert!(m.peers.is_empty());
    }

    #[test]
    fn connected_refresh_also_lists_peers() {
        let mut m = Model::new();
        let fx = m.apply(UiMsg::Connected);
        assert!(has(&fx, |r| matches!(r, Request::ListPeers)));
    }

    fn latest_reply(conv: &str) -> UiMsg {
        reply(
            ReplyTag::Messages {
                conv: conv.into(),
                older: false,
            },
            Response::Messages {
                value: vec![msg("latest", conv, "x")],
            },
        )
    }

    #[test]
    fn reload_failure_keeps_has_newer_and_allows_retry() {
        let mut m = model_in_conv("c1", 3);
        m.has_newer = true;
        assert!(!m.reload_latest().is_empty());
        assert!(m.reload_in_flight);
        m.apply(UiMsg::Reply {
            tag: ReplyTag::Messages {
                conv: "c1".into(),
                older: false,
            },
            result: Err(ApiError {
                code: ErrorCode::Io,
                message: "失败".into(),
            }),
        });
        assert!(m.has_newer, "失败后仍提示有新消息");
        assert!(!m.reload_in_flight);
        assert!(!m.reload_latest().is_empty(), "可再次点击重试");
        m.apply(latest_reply("c1"));
        assert!(!m.has_newer && !m.reload_in_flight);
    }

    #[test]
    fn send_with_has_newer_reloads_latest() {
        let mut m = model_in_conv("c1", 3);
        m.has_newer = true;
        let fx = m.send_text("c1".into(), "hi".into());
        let reqs = calls(&fx);
        assert!(matches!(
            reqs[0],
            Request::ListMessages { before: None, .. }
        ));
        assert!(matches!(reqs[1], Request::SendText { text, .. } if text == "hi"));
        // 无缺口时只发送
        let mut m = model_in_conv("c1", 3);
        let fx = m.send_files("c1".into(), vec!["/a".into()]);
        assert_eq!(calls(&fx).len(), 1);
        assert!(matches!(calls(&fx)[0], Request::SendFiles { paths, .. } if paths == &["/a"]));
    }

    #[test]
    fn send_dropped_files_once_and_each_dir_once() {
        let mut m = model_in_conv("c1", 1);
        let fx = m.send_dropped(
            "c1".into(),
            vec!["/a.txt".into(), "/b.png".into()],
            vec!["/d1".into(), "/d2".into()],
        );
        let reqs = calls(&fx);
        assert_eq!(reqs.len(), 3);
        assert!(matches!(reqs[0], Request::SendFiles { paths, .. } if paths.len() == 2));
        assert!(matches!(reqs[1], Request::SendFolder { path, .. } if path == "/d1"));
        assert!(matches!(reqs[2], Request::SendFolder { path, .. } if path == "/d2"));
        // 只有目录：不发空的 SendFiles
        let fx = m.send_dropped("c1".into(), vec![], vec!["/d1".into()]);
        assert_eq!(calls(&fx).len(), 1);
        assert!(m.send_dropped("c1".into(), vec![], vec![]).is_empty());
    }

    // ---- Task 14：联系人、请求、我的设备与配对 ----

    fn err_reply(tag: ReplyTag, msg: &str) -> UiMsg {
        UiMsg::Reply {
            tag,
            result: Err(ApiError {
                code: ErrorCode::Io,
                message: msg.into(),
            }),
        }
    }

    fn pair_info(session: &str, qr: Option<&str>) -> Response {
        Response::PairSession {
            value: PairSessionInfo {
                session: session.into(),
                qr_uri: qr.map(String::from),
            },
        }
    }

    fn pair_event(session: &str, state: PairStateDto) -> UiMsg {
        UiMsg::Event(Event::PairStatus {
            session: session.into(),
            state,
        })
    }

    fn my_device(id: &str, is_self: bool) -> DeviceView {
        DeviceView {
            device_id: id.into(),
            device_short: id.into(),
            name: id.into(),
            device_type: "desktop".into(),
            online: true,
            path: None,
            is_self,
        }
    }

    #[test]
    fn respond_request_removes_only_after_ok() {
        let mut m = Model::new();
        m.requests = vec![request("r1"), request("r2")];
        let fx = m.respond_request("r1".into(), RequestAction::Block);
        assert!(has(&fx, |r| matches!(r,
            Request::RespondRequest { request, action: RequestAction::Block } if request == "r1")));
        assert_eq!(m.requests.len(), 2, "回复前不移除");
        // 在途期间重复点击不重复发
        assert!(m
            .respond_request("r1".into(), RequestAction::Reject)
            .is_empty());
        m.apply(reply(
            ReplyTag::Responded {
                request: "r1".into(),
            },
            Response::Ok,
        ));
        assert_eq!(m.requests.len(), 1);
        assert_eq!(m.requests[0].request, "r2");
    }

    #[test]
    fn respond_request_error_keeps_request_and_allows_retry() {
        let mut m = Model::new();
        m.requests = vec![request("r1")];
        m.respond_request("r1".into(), RequestAction::AcceptOnce);
        m.apply(err_reply(
            ReplyTag::Responded {
                request: "r1".into(),
            },
            "处理失败",
        ));
        assert_eq!(m.requests.len(), 1);
        assert_eq!(m.toast.as_deref(), Some("处理失败"));
        assert_eq!(
            m.respond_request("r1".into(), RequestAction::AcceptOnce)
                .len(),
            1
        );
    }

    #[test]
    fn request_contact_toasts_on_ok_and_dedups_in_flight() {
        let mut m = Model::new();
        let fx = m.request_contact("u1".into());
        assert!(has(&fx, |r| matches!(r,
            Request::RequestContact { user } if user == "u1")));
        assert!(m.request_contact("u1".into()).is_empty(), "在途时不重复发");
        m.apply(reply(
            ReplyTag::Requested { user: "u1".into() },
            Response::Ok,
        ));
        assert_eq!(m.toast.as_deref(), Some(strings::CONTACT_REQUEST_SENT));
        assert!(!m.request_contact("u1".into()).is_empty(), "回复后可再发");
        m.apply(err_reply(ReplyTag::Requested { user: "u1".into() }, "x"));
        assert!(!m.request_contact("u1".into()).is_empty(), "失败后可重试");
    }

    #[test]
    fn pair_link_ok_cases() {
        assert!(pair_link_ok(" lanmsg://pair?x "));
        assert!(!pair_link_ok("http://x"));
        assert!(!pair_link_ok(""));
        assert!(!pair_link_ok("lanmsg://"));
    }

    #[test]
    fn pair_join_rejects_invalid_link() {
        let mut m = Model::new();
        m.pair_open_join();
        let fx = m.pair_join(PairTarget::QrUri {
            uri: "http://x".into(),
        });
        assert!(fx.is_empty());
        assert_eq!(m.pair, Some(PairUi::Picking));
    }

    #[test]
    fn normalize_peer_addr_cases() {
        let p = lanmsg_proto::consts::SERVICE_PORT;
        assert_eq!(normalize_peer_addr("10.8.0.5"), Ok(format!("10.8.0.5:{p}")));
        assert_eq!(
            normalize_peer_addr(" 10.8.0.5:7000 "),
            Ok("10.8.0.5:7000".into())
        );
        assert_eq!(normalize_peer_addr("[::1]:7000"), Ok("[::1]:7000".into()));
        assert_eq!(normalize_peer_addr("::1"), Ok(format!("[::1]:{p}")));
        // 括号无端口：补默认端口
        assert_eq!(normalize_peer_addr("[::1]"), Ok(format!("[::1]:{p}")));
        assert_eq!(normalize_peer_addr("host:7000"), Ok("host:7000".into()));
        assert_eq!(normalize_peer_addr("pc-1.lan"), Ok(format!("pc-1.lan:{p}")));
        assert!(normalize_peer_addr(":7000").is_err());
        assert!(normalize_peer_addr("host:99999").is_err());
        assert!(normalize_peer_addr("host:0").is_err());
        assert!(normalize_peer_addr("[::1]:99999").is_err());
        assert!(normalize_peer_addr("a b").is_err());
        assert!(normalize_peer_addr("").is_err());
    }

    #[test]
    fn pair_cancel_during_starting_ignores_late_reply_and_cancels_session() {
        let mut m = Model::new();
        m.pair_start(PairMethod::Sas);
        assert!(m.pair_cancel().is_empty(), "未拿到会话时无会话可取消");
        let fx = m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        assert_eq!(m.pair, None);
        assert!(has(&fx, |r| matches!(r,
            Request::PairCancel { session } if session == "s1")));
    }

    #[test]
    fn cancel_then_restart_ignores_stale_reply() {
        let mut m = Model::new();
        m.pair_start(PairMethod::Sas);
        let a = m.pair_nonce;
        m.pair_cancel();
        m.pair_start(PairMethod::Sas);
        let b = m.pair_nonce;
        assert_ne!(a, b);
        // A 的回复先到：界面仍在等 B，A 会话被取消
        let fx = m.apply(reply(ReplyTag::Pair { nonce: a }, pair_info("A", None)));
        assert_eq!(m.pair, Some(PairUi::Starting));
        assert!(has(&fx, |r| matches!(r,
            Request::PairCancel { session } if session == "A")));
        // B 的回复到达：正常进入展示
        m.apply(reply(ReplyTag::Pair { nonce: b }, pair_info("B", None)));
        assert!(matches!(&m.pair, Some(PairUi::Showing { session, .. }) if session == "B"));
    }

    #[test]
    fn stale_join_reply_and_error_do_not_touch_new_pair() {
        let mut m = Model::new();
        m.pair_open_join();
        m.pair_join(PairTarget::Device {
            device: "d1".into(),
        });
        let a = m.pair_nonce;
        m.pair_cancel();
        m.pair_open_join();
        m.pair_join(PairTarget::Device {
            device: "d2".into(),
        });
        let b = m.pair_nonce;
        // 旧请求的错误回复不得把新会话的界面打回选择页
        m.apply(err_reply(ReplyTag::PairJoin { nonce: a }, "失败"));
        assert_eq!(m.pair, Some(PairUi::Starting));
        let fx = m.apply(reply(ReplyTag::PairJoin { nonce: a }, pair_info("A", None)));
        assert_eq!(m.pair, Some(PairUi::Starting));
        assert!(has(&fx, |r| matches!(r,
            Request::PairCancel { session } if session == "A")));
        m.apply(reply(ReplyTag::PairJoin { nonce: b }, pair_info("B", None)));
        assert!(matches!(&m.pair, Some(PairUi::Showing { session, .. }) if session == "B"));
    }

    #[test]
    fn pair_status_before_reply_is_applied_after_reply() {
        let mut m = Model::new();
        m.pair_start(PairMethod::Sas);
        m.apply(pair_event(
            "s1",
            PairStateDto::ShowSas {
                code: "123456".into(),
            },
        ));
        assert_eq!(m.pair, Some(PairUi::Starting));
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        assert!(matches!(&m.pair, Some(PairUi::Showing { state, .. })
            if matches!(state, PairStateDto::ShowSas { .. })));
        // 属于其他会话的缓存不应用
        m.pair_cancel();
        m.pair_start(PairMethod::Sas);
        m.apply(pair_event("old", PairStateDto::Verifying));
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s2", None),
        ));
        assert!(matches!(&m.pair, Some(PairUi::Showing { state, .. })
            if *state == PairStateDto::WaitingPeer));
    }

    #[test]
    fn pair_confirm_match_sets_verifying_once() {
        let mut m = Model::new();
        m.pair_start(PairMethod::Sas);
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        m.apply(pair_event(
            "s1",
            PairStateDto::ShowSas {
                code: "123456".into(),
            },
        ));
        assert!(!m.pair_confirm(true).is_empty());
        assert!(matches!(&m.pair, Some(PairUi::Showing { state, .. })
            if *state == PairStateDto::Verifying));
        assert!(m.pair_confirm(false).is_empty(), "已确认后不再接受不一致");
    }

    #[test]
    fn set_contact_policy_updates_locally_and_resyncs_on_error() {
        let mut m = Model::new();
        m.contacts = vec![ContactView {
            user_id: "u1".into(),
            user_short: "ab".into(),
            display_name: "张三".into(),
            online: true,
            devices: vec![],
            auto_accept: false,
            ask_above_bytes: None,
        }];
        let fx = m.set_contact_policy("u1".into(), true, Some(5 << 20));
        assert!(has(&fx, |r| matches!(r,
            Request::SetContactPolicy { user, auto_accept: true, ask_above_bytes: Some(n) }
            if user == "u1" && *n == 5 << 20)));
        assert!(m.contacts[0].auto_accept);
        assert_eq!(m.contacts[0].ask_above_bytes, Some(5 << 20));
        let fx = m.apply(err_reply(ReplyTag::Policy, "保存失败"));
        assert!(
            has(&fx, |r| matches!(r, Request::ListContacts)),
            "失败后以服务端为准"
        );
        assert_eq!(m.toast.as_deref(), Some("保存失败"));
    }

    #[test]
    fn open_dm_reply_switches_to_chats_and_opens_conv() {
        let mut m = Model::new();
        m.page = Page::Contacts;
        let fx = m.open_dm("u1".into());
        assert!(has(
            &fx,
            |r| matches!(r, Request::OpenDm { user } if user == "u1")
        ));
        let fx = m.apply(reply(ReplyTag::Dm, Response::ConvId { value: "c9".into() }));
        assert_eq!(m.page, Page::Chats);
        assert_eq!(m.current_conv.as_deref(), Some("c9"));
        assert!(has(
            &fx,
            |r| matches!(r, Request::ListMessages { conv, .. } if conv == "c9")
        ));
        assert!(has(&fx, |r| matches!(r, Request::ListConversations)));
    }

    #[test]
    fn remove_device_refuses_self_and_refreshes_after_ok() {
        let mut m = Model::new();
        m.me = Some(self_info(true));
        m.devices = vec![my_device("d", true), my_device("d2", false)];
        // self_info 的 device_id 是 "d"
        assert!(m.remove_device("d".into()).is_empty());
        assert_eq!(m.toast.as_deref(), Some(strings::CANNOT_REMOVE_SELF));
        m.toast = None;
        let fx = m.remove_device("d2".into());
        assert!(has(
            &fx,
            |r| matches!(r, Request::RemoveDevice { device } if device == "d2")
        ));
        assert!(m.toast.is_none());
        let fx = m.apply(reply(ReplyTag::DeviceRemoved, Response::Ok));
        assert!(has(&fx, |r| matches!(r, Request::ListMyDevices)));
    }

    #[test]
    fn add_manual_peer_validates_and_refreshes_peers() {
        let mut m = Model::new();
        assert!(m.add_manual_peer("  ".into()).is_empty());
        assert_eq!(m.toast.as_deref(), Some(strings::MANUAL_PEER_EMPTY));
        let fx = m.add_manual_peer(" 10.8.0.5:7000 ".into());
        assert!(has(&fx, |r| matches!(r,
            Request::AddManualPeer { addr } if addr == "10.8.0.5:7000")));
        let fx = m.apply(reply(ReplyTag::ManualPeer, Response::Ok));
        assert!(has(&fx, |r| matches!(r, Request::ListPeers)));
        assert_eq!(m.toast.as_deref(), Some(strings::MANUAL_PEER_SENT));
    }

    #[test]
    fn pair_start_blocks_duplicates_and_shows_session() {
        let mut m = Model::new();
        let fx = m.pair_start(PairMethod::Qr);
        assert!(has(&fx, |r| matches!(
            r,
            Request::PairStart {
                method: PairMethod::Qr
            }
        )));
        assert_eq!(m.pair, Some(PairUi::Starting));
        assert!(m.pair_start(PairMethod::Sas).is_empty());
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s1", Some("lanmsg://pair?x")),
        ));
        assert!(
            matches!(&m.pair, Some(PairUi::Showing { session, qr_uri, state })
            if session == "s1" && qr_uri.is_some() && *state == PairStateDto::WaitingPeer)
        );
    }

    #[test]
    fn pair_start_error_closes_dialog() {
        let mut m = Model::new();
        m.pair_start(PairMethod::Sas);
        m.apply(err_reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            "无法开始",
        ));
        assert_eq!(m.pair, None);
        assert_eq!(m.toast.as_deref(), Some("无法开始"));
    }

    #[test]
    fn pair_join_from_picker_and_error_returns_to_picker() {
        let mut m = Model::new();
        // 未在选择界面时忽略
        assert!(m
            .pair_join(PairTarget::Device { device: "d".into() })
            .is_empty());
        m.pair_open_join();
        let fx = m.pair_join(PairTarget::QrUri {
            uri: "lanmsg://p".into(),
        });
        assert!(has(&fx, |r| matches!(r,
            Request::PairJoin { target: PairTarget::QrUri { uri } } if uri == "lanmsg://p")));
        assert_eq!(m.pair, Some(PairUi::Starting));
        m.apply(err_reply(
            ReplyTag::PairJoin {
                nonce: m.pair_nonce,
            },
            "链接无效",
        ));
        assert_eq!(m.pair, Some(PairUi::Picking));
        m.pair_join(PairTarget::Device { device: "d".into() });
        m.apply(reply(
            ReplyTag::PairJoin {
                nonce: m.pair_nonce,
            },
            pair_info("s2", None),
        ));
        assert!(matches!(&m.pair, Some(PairUi::Showing { session, .. }) if session == "s2"));
    }

    #[test]
    fn init_join_existing_opens_picker_and_lists_peers() {
        let mut m = Model::new();
        m.submit_init("小李", "pc", InitMode::JoinExisting);
        let fx = m.apply(reply(ReplyTag::Init, Response::Ok));
        assert_eq!(m.pair, Some(PairUi::Picking));
        assert!(has(&fx, |r| matches!(r, Request::ListPeers)));
    }

    #[test]
    fn pair_confirm_sends_and_mismatch_fails_locally() {
        let mut m = Model::new();
        m.pair_start(PairMethod::Sas);
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        m.apply(pair_event(
            "s1",
            PairStateDto::ShowSas {
                code: "123456".into(),
            },
        ));
        let fx = m.pair_confirm(true);
        assert!(has(&fx, |r| matches!(r,
            Request::PairConfirm { session, matches: true } if session == "s1")));
        // 已点“一致”后进入验证中；另起会话验证“不一致”
        let mut m = Model::new();
        m.pair_start(PairMethod::Sas);
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        m.apply(pair_event(
            "s1",
            PairStateDto::ShowSas {
                code: "123456".into(),
            },
        ));
        let fx = m.pair_confirm(false);
        assert!(has(&fx, |r| matches!(
            r,
            Request::PairConfirm { matches: false, .. }
        )));
        assert!(matches!(
            &m.pair,
            Some(PairUi::Showing {
                state: PairStateDto::Failed { .. },
                ..
            })
        ));
        // 没有会话时不发请求
        let mut m = Model::new();
        assert!(m.pair_confirm(true).is_empty());
    }

    #[test]
    fn pair_cancel_sends_cancel_and_resets_wizard_joining() {
        let mut m = Model::new();
        m.wizard_joining = true;
        m.pair_start(PairMethod::Sas);
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        let fx = m.pair_cancel();
        assert!(has(
            &fx,
            |r| matches!(r, Request::PairCancel { session } if session == "s1")
        ));
        assert_eq!(m.pair, None);
        assert!(!m.wizard_joining);
        // 失败 / 完成的会话不再发取消
        m.pair_start(PairMethod::Sas);
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s3", None),
        ));
        m.apply(pair_event(
            "s3",
            PairStateDto::Failed { reason: "x".into() },
        ));
        assert!(m.pair_cancel().is_empty());
        // 选择界面直接关闭：没有会话可取消
        m.pair_open_join();
        assert!(m.pair_cancel().is_empty());
        assert_eq!(m.pair, None);
    }

    #[test]
    fn pair_done_refreshes_devices_and_self_and_ends_wizard() {
        let mut m = Model::new();
        m.wizard_joining = true;
        m.pair_open_join();
        m.pair_join(PairTarget::Device { device: "d".into() });
        m.apply(reply(
            ReplyTag::PairJoin {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        let fx = m.apply(pair_event("s1", PairStateDto::Done));
        assert!(has(&fx, |r| matches!(r, Request::ListMyDevices)));
        assert!(has(&fx, |r| matches!(r, Request::GetSelf)));
        assert!(!m.wizard_joining);
        assert!(matches!(
            &m.pair,
            Some(PairUi::Showing {
                state: PairStateDto::Done,
                ..
            })
        ));
        // 1.5 秒后由界面关闭：不再发 PairCancel
        assert!(m.pair_close().is_empty());
        assert_eq!(m.pair, None);
    }

    #[test]
    fn pair_retry_replays_start_or_returns_to_picker() {
        let mut m = Model::new();
        m.pair_start(PairMethod::Sas);
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        m.apply(pair_event(
            "s1",
            PairStateDto::Failed {
                reason: "超时".into(),
            },
        ));
        let fx = m.pair_retry();
        assert!(has(&fx, |r| matches!(
            r,
            Request::PairStart {
                method: PairMethod::Sas
            }
        )));
        assert_eq!(m.pair, Some(PairUi::Starting));

        let mut m = Model::new();
        m.pair_open_join();
        m.pair_join(PairTarget::Device { device: "d".into() });
        m.apply(reply(
            ReplyTag::PairJoin {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        m.apply(pair_event(
            "s1",
            PairStateDto::Failed { reason: "x".into() },
        ));
        let fx = m.pair_retry();
        assert_eq!(m.pair, Some(PairUi::Picking));
        assert!(has(&fx, |r| matches!(r, Request::ListPeers)));
    }

    #[test]
    fn parse_ask_mb_cases() {
        assert_eq!(parse_ask_mb(""), Some(None));
        assert_eq!(parse_ask_mb(" 100 "), Some(Some(100 << 20)));
        assert_eq!(parse_ask_mb("0"), Some(Some(0)));
        assert_eq!(parse_ask_mb("abc"), None);
        assert_eq!(parse_ask_mb("-1"), None);
        assert_eq!(parse_ask_mb("99999999999999999999"), None);
    }

    #[test]
    fn set_contacts_tab_clamps_and_loads_data() {
        let mut m = Model::new();
        let fx = m.set_contacts_tab(3);
        assert_eq!(m.contacts_tab, 3);
        assert!(has(&fx, |r| matches!(r, Request::ListMyDevices)));
        m.set_contacts_tab(99);
        assert_eq!(m.contacts_tab, CONTACTS_TABS - 1);
    }
}
