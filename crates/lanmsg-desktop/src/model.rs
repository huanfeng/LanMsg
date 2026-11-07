// 界面状态模型：纯数据，不依赖 windui，所有状态迁移逻辑在此单测。
// 注释中文，标识符英文。

use std::collections::{BTreeMap, BTreeSet};

use lanmsg_api::types::*;

use crate::bridge::{ReplyTag, UiMsg};
use crate::error_text::{error_text, transfer_fail_text};
use crate::strings;

/// 一次拉取的消息页大小。
pub const MESSAGE_PAGE: u32 = 50;
/// 内存中保留的消息窗口上限。
pub const MESSAGE_WINDOW_MAX: usize = 300;
/// 文件视图一页的大小。
pub const FILE_PAGE: u32 = 100;
/// 静默刷新时一次最多取回的文件条数（与 core 的单次上限一致）。
const FILE_REFRESH_MAX: u32 = 500;
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
    /// 后台服务是用户主动退出的：不自动拉起，提示手动启动。
    DaemonQuit,
    /// 拉起后台服务失败；内容是展示给用户的原因。
    LaunchFailed(String),
}

impl ConnState {
    /// 是否已确认与后台服务断开（含主动退出、拉起失败）。
    pub fn is_down(&self) -> bool {
        matches!(
            self,
            ConnState::Disconnected(_) | ConnState::DaemonQuit | ConnState::LaunchFailed(_)
        )
    }
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
    /// 每次发出“从头查询”自增；追加页与首页的回复带回它，不一致的迟到回复丢弃。
    pub files_epoch: u64,
    /// 追加下一页的请求已发出、尚未回复。
    pub files_loading: bool,
    /// 上次返回满页，还可能有更早的文件。
    pub files_more: bool,
    /// 静默刷新分批取回时已收到的部分，凑够目标条数后才整体替换 `files`。
    pub files_refresh_acc: Vec<FileView>,
    /// 已收到过至少一次文件查询的回复（区分“加载中”与“没有文件”）。
    pub files_loaded: bool,
    /// 文件页视图的重建序号：进入文件页或用户改筛选时自增（回到列表顶部）；
    /// 事件触发的静默刷新不自增，滚动位置得以保留。
    pub files_view_epoch: u64,
    pub settings: Option<SettingsDto>,
    /// 已发出、尚未收到回复的设置保存数；>0 时设置页各组“保存”置灰，
    /// 避免连续保存时用尚未刷新的旧值把另一组写回。
    pub settings_saving: u32,
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
    /// 发送失败待回填输入框的草稿，按会话保存；由 UiState 在该会话输入框存在时取走。
    pub draft_restore: BTreeMap<String, String>,
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
            files_epoch: 0,
            files_loading: false,
            files_more: false,
            files_refresh_acc: Vec::new(),
            files_loaded: false,
            files_view_epoch: 0,
            settings: None,
            settings_saving: 0,
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
            draft_restore: BTreeMap::new(),
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
                // 断线后在途请求的回复可能不会再来，别让保存按钮一直停在“正在保存”
                self.settings_saving = 0;
                Vec::new()
            }
            UiMsg::DaemonQuit => {
                self.conn = ConnState::DaemonQuit;
                self.settings_saving = 0;
                Vec::new()
            }
            UiMsg::LaunchFailed(reason) => {
                self.conn = ConnState::LaunchFailed(reason);
                self.settings_saving = 0;
                Vec::new()
            }
            UiMsg::Reply { tag, result } => match result {
                Ok(resp) => self.apply_reply(tag, resp),
                Err(e) => {
                    // 断线期间的自动请求会得到“未连接”，顶部提示条已说明，不再弹 toast
                    let offline_io = e.code == ErrorCode::Io && self.conn.is_down();
                    if !offline_io {
                        self.toast = Some(error_text(&e.code).to_string());
                    }
                    match &tag {
                        // 文字发送失败：草稿交还输入框，避免用户重打
                        ReplyTag::Sent {
                            conv,
                            draft: Some(d),
                        } => {
                            // 同一会话连续失败：先失败的排在前面，保持时间顺序
                            let merged = match self.draft_restore.remove(conv) {
                                Some(earlier) => format!("{earlier}\n{d}"),
                                None => d.clone(),
                            };
                            self.draft_restore.insert(conv.clone(), merged);
                        }
                        ReplyTag::Messages { older: true, .. } => self.older_in_flight = false,
                        ReplyTag::Messages { older: false, .. } => self.reload_in_flight = false,
                        ReplyTag::Files {
                            append: true,
                            epoch,
                        } if *epoch == self.files_epoch => self.files_loading = false,
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
                        ReplyTag::SettingsSaved => {
                            self.settings_saving = self.settings_saving.saturating_sub(1)
                        }
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
            (ReplyTag::Files { append, epoch }, Response::Files { value }) => {
                // 条件变了之后迟到的回复（含旧的追加页）一律丢弃
                if epoch != self.files_epoch {
                    return Vec::new();
                }
                self.files_loaded = true;
                self.files_more = value.len() as u32 >= self.file_filter.limit;
                if append {
                    self.files_loading = false;
                    // 回复条数可能多于 limit（状态筛选凑满页），不截断；翻页游标取 `files` 最后一项
                    self.files.extend(value);
                } else {
                    self.files = value;
                }
                Vec::new()
            }
            (
                ReplyTag::FilesRefresh {
                    epoch,
                    target,
                    asked,
                },
                Response::Files { value },
            ) => {
                if epoch != self.files_epoch {
                    return Vec::new();
                }
                let got = value.len() as u32;
                // 带状态筛选时 core 会凑满页，回复条数可能多于 limit：客户端不截断，
                // 游标取累积列表的最后一项，才不会丢项；`got >= asked` 对多于 limit 的回复同样成立
                self.files_refresh_acc.extend(value);
                let have = self.files_refresh_acc.len() as u32;
                // 本批满了且还没凑够：以已取回的最后一项为游标继续取
                if got >= asked && have < target {
                    let Some(last) = self.files_refresh_acc.last() else {
                        return Vec::new();
                    };
                    let mut filter = self.file_filter.clone();
                    filter.before_millis = Some(last.hlc_millis);
                    filter.limit = (target - have).min(FILE_REFRESH_MAX);
                    return vec![call(
                        Request::QueryFiles {
                            filter: filter.clone(),
                        },
                        ReplyTag::FilesRefresh {
                            epoch,
                            target,
                            asked: filter.limit,
                        },
                    )];
                }
                self.files = std::mem::take(&mut self.files_refresh_acc);
                self.files_loaded = true;
                self.files_more = got >= asked;
                Vec::new()
            }
            (ReplyTag::Settings, Response::Settings { value }) => {
                self.settings = Some(value);
                Vec::new()
            }
            (ReplyTag::SettingsSaved, Response::Ok) => {
                self.settings_saving = self.settings_saving.saturating_sub(1);
                self.toast = Some(strings::SETTINGS_SAVED.to_string());
                vec![call(Request::GetSettings, ReplyTag::Settings)]
            }
            (ReplyTag::ProfileSaved, Response::Ok) => {
                vec![call(Request::GetSelf, ReplyTag::SelfInfo)]
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
                self.files.retain(|f| f.msg != msg);
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

    /// 事件处理；文件页打开时，会改变文件列表内容的事件（新文件消息、传输结束）
    /// 额外触发一次静默刷新，让“下载”按钮及时变成“打开”。
    fn apply_event(&mut self, ev: Event) -> Vec<Effect> {
        let touches_files = self.page == Page::Files
            && match &ev {
                Event::TransferFinished { .. } => true,
                Event::MessageAdded { msg } | Event::MessageUpdated { msg } => {
                    matches!(
                        msg.body,
                        MessageBodyView::Files { .. } | MessageBodyView::Folder { .. }
                    )
                }
                _ => false,
            };
        let mut fx = self.apply_event_inner(ev);
        if touches_files {
            fx.extend(self.refresh_files());
        }
        fx
    }

    fn apply_event_inner(&mut self, ev: Event) -> Vec<Effect> {
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
                hash, ok, reason, ..
            } => {
                self.transfers.remove(&hash);
                self.transfer_ids.remove(&hash);
                self.pending_downloads.remove(&hash);
                let cancelled = self.cancelled_downloads.remove(&hash);
                if !ok && !cancelled {
                    let reason = reason.unwrap_or(TransferFailReason::Unknown);
                    self.toast = Some(transfer_fail_text(&reason).to_string());
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
            Page::Files => self.query_files(true),
            // 设置页内嵌“我的设备”，一并刷新设备列表
            Page::Settings => vec![
                call(Request::GetSettings, ReplyTag::Settings),
                call(Request::ListMyDevices, ReplyTag::Devices),
            ],
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

    /// 选择界面里手动或周期刷新候选设备；core 没有“重新发现”接口，只重新取列表。
    pub fn pair_refresh_peers(&mut self) -> Vec<Effect> {
        if self.pair != Some(PairUi::Picking) {
            return Vec::new();
        }
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
        // 仅在等待比对时有效；点“一致”后本地置为验证中，避免先一致后又点不一致。
        // 随请求带上界面当前显示的数字，核心据此拒绝过期的确认。
        let PairStateDto::ShowSas { code, .. } = &*state else {
            return Vec::new();
        };
        let code = code.clone();
        *state = if matches {
            PairStateDto::Verifying
        } else {
            PairStateDto::Failed {
                reason: PairFailReason::SasMismatch,
            }
        };
        vec![call(
            Request::PairConfirm {
                session,
                matches,
                code,
            },
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

    /// 从头查询文件列表（条件变化、进入文件页）；`reset_view` 为真时文件页回到列表顶部。
    fn query_files(&mut self, reset_view: bool) -> Vec<Effect> {
        self.files_epoch += 1;
        self.files_loading = false;
        if reset_view {
            self.files_view_epoch += 1;
        }
        self.file_filter.before_millis = None;
        vec![call(
            Request::QueryFiles {
                filter: self.file_filter.clone(),
            },
            ReplyTag::Files {
                append: false,
                epoch: self.files_epoch,
            },
        )]
    }

    /// 静默刷新文件列表：保持当前条件与已加载的条数（不退回第一页、不动滚动位置）。
    /// 已加载条数超过 core 单次上限时分多批取回（见 `ReplyTag::FilesRefresh`），凑够后整体替换。
    pub fn refresh_files(&mut self) -> Vec<Effect> {
        self.files_epoch += 1;
        self.files_loading = false;
        self.files_refresh_acc.clear();
        let target = (self.files.len() as u32).max(FILE_PAGE);
        let mut filter = self.file_filter.clone();
        filter.before_millis = None;
        filter.limit = target.min(FILE_REFRESH_MAX);
        vec![call(
            Request::QueryFiles {
                filter: filter.clone(),
            },
            ReplyTag::FilesRefresh {
                epoch: self.files_epoch,
                target,
                asked: filter.limit,
            },
        )]
    }

    /// 文件类型筛选；条件没变不重复查询。
    pub fn set_file_kind(&mut self, kind: FileKindFilter) -> Vec<Effect> {
        if self.file_filter.kind == kind {
            return Vec::new();
        }
        self.file_filter.kind = kind;
        self.query_files(true)
    }

    /// 会话筛选（None = 全部会话）。
    pub fn set_file_conv(&mut self, conv: Option<String>) -> Vec<Effect> {
        if self.file_filter.conv == conv {
            return Vec::new();
        }
        self.file_filter.conv = conv;
        self.query_files(true)
    }

    /// 状态筛选（None = 全部状态）。
    pub fn set_file_state(&mut self, state: Option<BlobState>) -> Vec<Effect> {
        if self.file_filter.state == state {
            return Vec::new();
        }
        self.file_filter.state = state;
        self.query_files(true)
    }

    /// 名字搜索；去首尾空白，空白等于不搜。
    pub fn set_file_name(&mut self, name: String) -> Vec<Effect> {
        let name = Some(name.trim().to_string()).filter(|n| !n.is_empty());
        if self.file_filter.name_query == name {
            return Vec::new();
        }
        self.file_filter.name_query = name;
        self.query_files(true)
    }

    /// 滚到底：上次是满页且没有追加请求在途时，以最后一项的时间为游标追加下一页。
    pub fn load_more_files(&mut self) -> Vec<Effect> {
        if !self.files_more || self.files_loading {
            return Vec::new();
        }
        let Some(last) = self.files.last() else {
            return Vec::new();
        };
        let mut filter = self.file_filter.clone();
        filter.before_millis = Some(last.hlc_millis);
        self.files_loading = true;
        vec![call(
            Request::QueryFiles { filter },
            ReplyTag::Files {
                append: true,
                epoch: self.files_epoch,
            },
        )]
    }

    /// 保存设置页：`SetSettings`；显示名变了（去空白后）再发 `SetProfile`。
    pub fn save_settings(&mut self, settings: SettingsDto, display_name: &str) -> Vec<Effect> {
        self.settings_saving += 1;
        let mut fx = vec![call(
            Request::SetSettings { settings },
            ReplyTag::SettingsSaved,
        )];
        let name = display_name.trim();
        let current = self
            .me
            .as_ref()
            .and_then(|me| me.display_name.as_deref())
            .unwrap_or_default();
        if name != current {
            fx.push(call(
                Request::SetProfile {
                    display_name: name.to_string(),
                },
                ReplyTag::ProfileSaved,
            ));
        }
        fx
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
            last_kind: None,
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
            reason: None,
        }));
        assert!(m.transfers.is_empty());
        assert_eq!(m.toast, None);
        m.apply(UiMsg::Event(Event::TransferFinished {
            transfer: "t".into(),
            hash: "h2".into(),
            ok: false,
            error: Some("disk full".into()),
            reason: Some(TransferFailReason::Io),
        }));
        assert_eq!(m.toast.as_deref(), Some(strings::DOWNLOAD_IO_FAILED));
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
            state: sas_dto("123456"),
        }));
        match &m.pair {
            Some(PairUi::Showing { state, qr_uri, .. }) => {
                assert_eq!(state, &sas_dto("123456"));
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
                message: "peer offline".into(),
            }),
        });
        assert_eq!(
            m.toast.as_deref(),
            Some(error_text(&ErrorCode::PeerOffline))
        );
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
                message: "peer offline".into(),
            }),
        });
        assert_eq!(
            m.draft_restore.get("c1").map(String::as_str),
            Some("你好\n世界")
        );
    }

    fn fail_send(m: &mut Model, conv: &str, text: &str) {
        m.apply(UiMsg::Reply {
            tag: ReplyTag::Sent {
                conv: conv.into(),
                draft: Some(text.into()),
            },
            result: Err(ApiError {
                code: ErrorCode::PeerOffline,
                message: "peer offline".into(),
            }),
        });
    }

    #[test]
    fn send_failures_keep_drafts_per_conv_in_time_order() {
        let mut m = Model::new();
        fail_send(&mut m, "c1", "第一条");
        fail_send(&mut m, "c2", "另一会话");
        fail_send(&mut m, "c1", "第二条");
        assert_eq!(m.draft_restore.len(), 2);
        assert_eq!(m.draft_restore["c1"], "第一条\n第二条");
        assert_eq!(m.draft_restore["c2"], "另一会话");
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
        assert!(m.draft_restore.is_empty());
    }

    #[test]
    fn io_error_while_disconnected_does_not_toast() {
        let mut m = Model::new();
        m.apply(UiMsg::Disconnected("断开".into()));
        m.apply(UiMsg::Reply {
            tag: ReplyTag::Generic,
            result: Err(ApiError {
                code: ErrorCode::Io,
                message: "not connected".into(),
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
    fn daemon_quit_and_launch_failed_flow_back_to_connected() {
        let mut m = model_in_conv("c1", 3);
        m.apply(UiMsg::DaemonQuit);
        assert_eq!(m.conn, ConnState::DaemonQuit);
        assert!(m.conn.is_down());
        assert_eq!(m.messages.len(), 3, "主动退出期间数据保留");
        m.apply(UiMsg::LaunchFailed("找不到 lanmsgd".into()));
        assert_eq!(m.conn, ConnState::LaunchFailed("找不到 lanmsgd".into()));
        assert!(m.conn.is_down());
        // 点“启动后台服务”后桥发 Disconnected，连上后回到 Connected
        m.apply(UiMsg::Disconnected("正在启动后台服务…".into()));
        assert!(m.conn.is_down());
        m.apply(UiMsg::Connected);
        assert_eq!(m.conn, ConnState::Connected);
        assert!(!m.conn.is_down());
    }

    #[test]
    fn offline_io_error_is_silent_when_daemon_quit() {
        let mut m = model_in_conv("c1", 1);
        m.apply(UiMsg::DaemonQuit);
        m.toast = None;
        m.apply(UiMsg::Reply {
            tag: ReplyTag::Generic,
            result: Err(ApiError {
                code: ErrorCode::Io,
                message: "not connected".into(),
            }),
        });
        assert_eq!(m.toast, None);
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
                    message: "init failed".into(),
                }),
            });
            assert!(fx.is_empty());
            assert!(!m.wizard_joining);
            assert!(!m.init_in_flight);
            assert_eq!(m.init_mode, None);
            assert_eq!(m.toast.as_deref(), Some(error_text(&ErrorCode::Io)));
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
            reason: None,
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
            error: Some("source offline".into()),
            reason: Some(TransferFailReason::SourceOffline),
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
            error: Some("cancelled".into()),
            reason: Some(TransferFailReason::Cancelled),
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
                message: "failed".into(),
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

    fn sas_dto(code: impl Into<String>) -> PairStateDto {
        PairStateDto::ShowSas {
            code: code.into(),
            peer_name: "我的手机".into(),
            peer_type: "phone".into(),
            peer_short: "a1b2c3d4".into(),
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
            "failed",
        ));
        assert_eq!(m.requests.len(), 1);
        assert_eq!(m.toast.as_deref(), Some(error_text(&ErrorCode::Io)));
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
    fn qr_pair_dialog_switches_to_sas_when_peer_joins_by_digits() {
        let mut m = Model::new();
        m.pair_start(PairMethod::Qr);
        let n = m.pair_nonce;
        m.apply(reply(
            ReplyTag::Pair { nonce: n },
            pair_info("s1", Some("lanmsg://pair?s=x")),
        ));
        m.apply(pair_event("s1", sas_dto("123456")));
        assert!(matches!(
            &m.pair,
            Some(PairUi::Showing { qr_uri: Some(_), state: PairStateDto::ShowSas { code, .. }, .. })
                if code == "123456"
        ));
        let fx = m.pair_confirm(true);
        assert!(has(&fx, |r| matches!(r,
            Request::PairConfirm { session, matches: true, code }
                if session == "s1" && code == "123456")));
    }

    #[test]
    fn pair_refresh_peers_requests_list_only_while_picking() {
        let mut m = Model::new();
        assert!(m.pair_refresh_peers().is_empty());
        m.pair_open_join();
        let fx = m.pair_refresh_peers();
        assert!(has(&fx, |r| matches!(r, Request::ListPeers)));
        m.pair_cancel();
        assert!(m.pair_refresh_peers().is_empty());
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
        m.apply(pair_event("s1", sas_dto("123456")));
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
        m.apply(pair_event("s1", sas_dto("123456")));
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
        let fx = m.apply(err_reply(ReplyTag::Policy, "failed"));
        assert!(
            has(&fx, |r| matches!(r, Request::ListContacts)),
            "失败后以服务端为准"
        );
        assert_eq!(m.toast.as_deref(), Some(error_text(&ErrorCode::Io)));
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
            "failed",
        ));
        assert_eq!(m.pair, None);
        assert_eq!(m.toast.as_deref(), Some(error_text(&ErrorCode::Io)));
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
        m.apply(pair_event("s1", sas_dto("123456")));
        let fx = m.pair_confirm(true);
        assert!(has(&fx, |r| matches!(r,
            Request::PairConfirm { session, matches: true, code }
                if session == "s1" && code == "123456")));
        // 已点“一致”后进入验证中；另起会话验证“不一致”
        let mut m = Model::new();
        m.pair_start(PairMethod::Sas);
        m.apply(reply(
            ReplyTag::Pair {
                nonce: m.pair_nonce,
            },
            pair_info("s1", None),
        ));
        m.apply(pair_event("s1", sas_dto("123456")));
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
            PairStateDto::Failed {
                reason: PairFailReason::Unknown,
            },
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
                reason: PairFailReason::Timeout,
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
            PairStateDto::Failed {
                reason: PairFailReason::Unknown,
            },
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

    // ---- 文件页与设置页（Task 15） ----

    fn file_view(i: usize) -> FileView {
        FileView {
            item: remote_item(&format!("h{i}")),
            conv: "c1".into(),
            conv_title: "小王".into(),
            msg: format!("m{i}"),
            hlc_millis: 10_000 - i as i64,
        }
    }

    fn files_page(n: usize) -> Vec<FileView> {
        (0..n).map(file_view).collect()
    }

    fn files_reply(tag: ReplyTag, n: usize) -> UiMsg {
        reply(
            tag,
            Response::Files {
                value: files_page(n),
            },
        )
    }

    fn query_tag(fx: &[Effect]) -> ReplyTag {
        fx.iter()
            .find_map(|e| match e {
                Effect::Call(Request::QueryFiles { .. }, t) => Some(t.clone()),
                _ => None,
            })
            .expect("应发出 QueryFiles")
    }

    fn query_filter(fx: &[Effect]) -> FileFilter {
        calls(fx)
            .into_iter()
            .find_map(|r| match r {
                Request::QueryFiles { filter } => Some(filter.clone()),
                _ => None,
            })
            .expect("应发出 QueryFiles")
    }

    #[test]
    fn file_filter_change_resets_cursor_and_queries() {
        let mut m = Model::new();
        m.file_filter.before_millis = Some(5);
        let fx = m.set_file_kind(FileKindFilter::Image);
        let f = query_filter(&fx);
        assert_eq!(f.kind, FileKindFilter::Image);
        assert_eq!(f.before_millis, None);
        assert_eq!(f.limit, FILE_PAGE);
        assert!(matches!(
            query_tag(&fx),
            ReplyTag::Files { append: false, .. }
        ));

        let fx = m.set_file_conv(Some("c1".into()));
        assert_eq!(query_filter(&fx).conv.as_deref(), Some("c1"));
        let fx = m.set_file_state(Some(BlobState::Local));
        assert_eq!(query_filter(&fx).state, Some(BlobState::Local));
        // 之前的条件保留
        assert_eq!(query_filter(&fx).kind, FileKindFilter::Image);
    }

    #[test]
    fn file_name_filter_trims_and_blank_clears() {
        let mut m = Model::new();
        let fx = m.set_file_name("  报告 ".into());
        assert_eq!(query_filter(&fx).name_query.as_deref(), Some("报告"));
        let fx = m.set_file_name("   ".into());
        assert_eq!(query_filter(&fx).name_query, None);
        // 与当前条件相同不重复查询
        assert!(m.set_file_name(String::new()).is_empty());
    }

    #[test]
    fn stale_files_reply_is_ignored() {
        let mut m = Model::new();
        let old = query_tag(&m.set_file_kind(FileKindFilter::Image));
        let new = query_tag(&m.set_file_kind(FileKindFilter::Video));
        m.apply(files_reply(new, 3));
        assert_eq!(m.files.len(), 3);
        // 先发后到的旧回复不能覆盖新结果
        m.apply(files_reply(old, 7));
        assert_eq!(m.files.len(), 3);
    }

    #[test]
    fn files_more_follows_page_fullness() {
        let mut m = Model::new();
        let tag = query_tag(&m.set_page(Page::Files));
        m.apply(files_reply(tag, FILE_PAGE as usize));
        assert!(m.files_more && m.files_loaded);
        let tag = query_tag(&m.set_file_kind(FileKindFilter::Image));
        m.apply(files_reply(tag, 3));
        assert!(!m.files_more);
        assert!(m.load_more_files().is_empty(), "不满页不再翻页");
    }

    #[test]
    fn load_more_files_uses_last_time_and_appends_once() {
        let mut m = Model::new();
        let tag = query_tag(&m.set_page(Page::Files));
        m.apply(files_reply(tag, FILE_PAGE as usize));
        let last = m.files.last().map(|f| f.hlc_millis);
        let fx = m.load_more_files();
        assert_eq!(query_filter(&fx).before_millis, last);
        let tag = query_tag(&fx);
        assert!(matches!(tag, ReplyTag::Files { append: true, .. }));
        // 请求在途时不重复发
        assert!(m.load_more_files().is_empty());
        let mut next = files_page(2);
        next[0].msg = "later0".into();
        next[1].msg = "later1".into();
        m.apply(reply(tag, Response::Files { value: next }));
        assert_eq!(m.files.len(), FILE_PAGE as usize + 2);
        assert!(!m.files_more && !m.files_loading);
    }

    #[test]
    fn load_more_error_clears_in_flight_and_stale_append_is_dropped() {
        let mut m = Model::new();
        let tag = query_tag(&m.set_page(Page::Files));
        m.apply(files_reply(tag, FILE_PAGE as usize));
        let tag = query_tag(&m.load_more_files());
        m.apply(err_reply(tag, "失败"));
        assert!(!m.files_loading);
        // 追加请求在途时条件变了：旧的追加结果不能混进新列表
        let append = query_tag(&m.load_more_files());
        let fresh = query_tag(&m.set_file_kind(FileKindFilter::Video));
        m.apply(files_reply(fresh, 4));
        m.apply(files_reply(append, 9));
        assert_eq!(m.files.len(), 4);
        assert!(!m.files_loading);
    }

    #[test]
    fn files_page_entry_and_filter_bump_view_epoch_but_refresh_does_not() {
        let mut m = Model::new();
        m.set_page(Page::Files);
        let e1 = m.files_view_epoch;
        m.set_file_kind(FileKindFilter::Image);
        assert!(m.files_view_epoch > e1);
        let e2 = m.files_view_epoch;
        m.page = Page::Files;
        m.apply(UiMsg::Event(Event::TransferFinished {
            transfer: "t".into(),
            hash: "h".into(),
            ok: true,
            error: None,
            reason: None,
        }));
        assert_eq!(m.files_view_epoch, e2);
    }

    #[test]
    fn transfer_finished_refreshes_files_only_on_files_page() {
        let mut m = Model::new();
        m.files = files_page(3);
        let ev = || {
            UiMsg::Event(Event::TransferFinished {
                transfer: "t".into(),
                hash: "h0".into(),
                ok: true,
                error: None,
                reason: None,
            })
        };
        assert!(!has(&m.apply(ev()), |r| matches!(
            r,
            Request::QueryFiles { .. }
        )));
        m.page = Page::Files;
        let fx = m.apply(ev());
        // 刷新保持已加载的条数，不退回第一页
        assert!(has(&fx, |r| matches!(r, Request::QueryFiles { .. })));
        assert_eq!(query_filter(&fx).limit, FILE_PAGE);
        assert!(matches!(query_tag(&fx), ReplyTag::FilesRefresh { .. }));
    }

    #[test]
    fn refresh_keeps_loaded_count_when_more_pages_loaded() {
        let mut m = Model::new();
        m.page = Page::Files;
        m.files = files_page(FILE_PAGE as usize + 40);
        let fx = m.refresh_files();
        assert_eq!(query_filter(&fx).limit, FILE_PAGE + 40);
        assert_eq!(query_filter(&fx).before_millis, None);
        // 条数不多时一批就够，回复直接替换
        m.apply(files_reply(query_tag(&fx), FILE_PAGE as usize + 40));
        assert_eq!(m.files.len(), FILE_PAGE as usize + 40);
    }

    #[test]
    fn refresh_beyond_core_cap_fetches_in_batches_and_replaces_once() {
        let mut m = Model::new();
        m.page = Page::Files;
        m.files = files_page(FILE_REFRESH_MAX as usize + 120);
        let before = m.files.clone();
        let fx = m.refresh_files();
        // 第一批不超过 core 的单次上限
        assert_eq!(query_filter(&fx).limit, FILE_REFRESH_MAX);
        assert_eq!(query_filter(&fx).before_millis, None);
        let fx = m.apply(files_reply(query_tag(&fx), FILE_REFRESH_MAX as usize));
        // 还差 120 条：以已取回的最后一项为游标继续取，期间原列表不动
        assert_eq!(m.files, before);
        assert_eq!(query_filter(&fx).limit, 120);
        assert_eq!(
            query_filter(&fx).before_millis,
            Some(file_view(FILE_REFRESH_MAX as usize - 1).hlc_millis)
        );
        let fx = m.apply(files_reply(query_tag(&fx), 120));
        assert!(fx.is_empty());
        assert_eq!(m.files.len(), FILE_REFRESH_MAX as usize + 120);
        assert!(m.files_more, "末批取满，后面可能还有");
    }

    #[test]
    fn refresh_stops_when_source_has_fewer_files() {
        let mut m = Model::new();
        m.files = files_page(FILE_REFRESH_MAX as usize + 120);
        let fx = m.refresh_files();
        // 期间有文件被删，第一批没取满：直接结束，不再追问
        let fx = m.apply(files_reply(query_tag(&fx), 450));
        assert!(fx.is_empty());
        assert_eq!(m.files.len(), 450);
        assert!(!m.files_more);
    }

    #[test]
    fn stale_refresh_batch_is_dropped_after_filter_change() {
        let mut m = Model::new();
        m.files = files_page(FILE_REFRESH_MAX as usize + 10);
        let fx = m.refresh_files();
        let tag = query_tag(&fx);
        m.set_file_kind(FileKindFilter::Image);
        let before = m.files.clone();
        let fx = m.apply(files_reply(tag, FILE_REFRESH_MAX as usize));
        assert!(fx.is_empty());
        assert_eq!(m.files, before);
    }

    #[test]
    fn new_files_message_refreshes_files_page() {
        let mut m = Model::new();
        m.page = Page::Files;
        let mut mv = msg("m9", "c1", "x");
        mv.body = MessageBodyView::Files {
            items: vec![remote_item("h9")],
        };
        let fx = m.apply(UiMsg::Event(Event::MessageAdded { msg: mv }));
        assert!(has(&fx, |r| matches!(r, Request::QueryFiles { .. })));
        let fx = m.apply(UiMsg::Event(Event::MessageAdded {
            msg: msg("m10", "c1", "纯文字"),
        }));
        assert!(!has(&fx, |r| matches!(r, Request::QueryFiles { .. })));
    }

    #[test]
    fn deleted_reply_drops_files_of_that_message() {
        let mut m = Model::new();
        m.files = files_page(3);
        m.apply(reply(ReplyTag::Deleted { msg: "m1".into() }, Response::Ok));
        assert_eq!(m.files.len(), 2);
        assert!(m.files.iter().all(|f| f.msg != "m1"));
    }

    fn settings_dto() -> SettingsDto {
        SettingsDto {
            device_name: "旧名".into(),
            download_dir: "/d".into(),
            auto_image_max: 1 << 20,
            auto_file_max: 2 << 20,
            auto_file_wifi_only: true,
            localsend_enabled: true,
            unicast_targets: Vec::new(),
            background_mode: BackgroundMode::Realtime,
            localsend_listening: None,
        }
    }

    #[test]
    fn save_settings_sends_profile_only_when_name_changed() {
        let mut m = Model::new();
        let mut me = self_info(true);
        me.display_name = Some("小李".into());
        m.me = Some(me);
        let fx = m.save_settings(settings_dto(), "小李");
        assert!(has(&fx, |r| matches!(r, Request::SetSettings { .. })));
        assert!(!has(&fx, |r| matches!(r, Request::SetProfile { .. })));
        let fx = m.save_settings(settings_dto(), "  新名字 ");
        assert!(has(&fx, |r| matches!(r,
            Request::SetProfile { display_name } if display_name == "新名字")));
    }

    #[test]
    fn settings_saved_reply_toasts_and_reloads() {
        let mut m = Model::new();
        let fx = m.apply(reply(ReplyTag::SettingsSaved, Response::Ok));
        assert_eq!(m.toast.as_deref(), Some(strings::SETTINGS_SAVED));
        assert!(has(&fx, |r| matches!(r, Request::GetSettings)));
        let fx = m.apply(reply(ReplyTag::ProfileSaved, Response::Ok));
        assert!(has(&fx, |r| matches!(r, Request::GetSelf)));
    }

    #[test]
    fn settings_save_failure_shows_localized_text() {
        let mut m = Model::new();
        m.apply(err_reply(ReplyTag::SettingsSaved, "failed"));
        assert_eq!(m.toast.as_deref(), Some(error_text(&ErrorCode::Io)));
    }

    #[test]
    fn settings_saving_spans_request_until_reply() {
        let mut m = Model::new();
        m.save_settings(settings_dto(), "");
        assert_eq!(m.settings_saving, 1);
        m.apply(reply(ReplyTag::SettingsSaved, Response::Ok));
        assert_eq!(m.settings_saving, 0);
        m.save_settings(settings_dto(), "");
        m.apply(err_reply(ReplyTag::SettingsSaved, "失败"));
        assert_eq!(m.settings_saving, 0, "失败回复也要放开按钮");
        m.save_settings(settings_dto(), "");
        m.apply(UiMsg::Disconnected("断开".into()));
        assert_eq!(m.settings_saving, 0, "断线后回复不会再来");
    }
}
