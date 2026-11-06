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
    Showing {
        session: String,
        qr_uri: Option<String>,
        state: PairStateDto,
    },
}

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
    pub toast: Option<String>,
    pub window_focused: bool,
    pub search_query: String,
    pub search_results: Vec<MessageView>,
    /// 已提交、尚未得到回复的向导初始化模式。
    pub init_mode: Option<InitMode>,
    /// 以“加入已有设备”方式完成了 Init，等待配对（Task 14 接管）。
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
            toast: None,
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
                    Vec::new()
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
            (ReplyTag::Pair, Response::PairSession { value }) => {
                self.pair = Some(PairUi::Showing {
                    session: value.session,
                    qr_uri: value.qr_uri,
                    state: PairStateDto::WaitingPeer,
                });
                Vec::new()
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
                    Some(InitMode::JoinExisting) => self.wizard_joining = true,
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
            (ReplyTag::Forwarded, _) => {
                self.toast = Some(strings::FORWARDED.to_string());
                Vec::new()
            }
            // Sent / Transfer / Generic 等无需更新模型：结果经事件到达
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
                if let Some(PairUi::Showing {
                    session: cur,
                    state: cur_state,
                    ..
                }) = self.pair.as_mut()
                {
                    if *cur == session {
                        *cur_state = state;
                    }
                }
                Vec::new()
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
        let mut fx = self.send_prefix();
        fx.push(call(
            Request::SendText {
                conv: conv.clone(),
                text,
            },
            ReplyTag::Sent { conv },
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
            ReplyTag::Sent { conv },
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
        let tag = || ReplyTag::Sent { conv: conv.clone() };
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
        m.apply(reply(
            ReplyTag::Pair,
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
            None => panic!("pair 应已建立"),
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
}
