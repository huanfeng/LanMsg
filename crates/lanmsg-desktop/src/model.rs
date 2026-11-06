// 界面状态模型：纯数据，不依赖 windui，所有状态迁移逻辑在此单测。
// 注释中文，标识符英文。

use std::collections::BTreeMap;

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
    pub pair: Option<PairUi>,
    pub toast: Option<String>,
    pub window_focused: bool,
    pub search_query: String,
    pub search_results: Vec<MessageView>,
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
            pair: None,
            toast: None,
            window_focused: true,
            search_query: String::new(),
            search_results: Vec::new(),
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
                    self.toast = Some(e.message);
                    Vec::new()
                }
            },
            UiMsg::Event(ev) => self.apply_event(ev),
        }
    }

    /// 连接（重连）后的全量刷新。
    fn refresh_all(&mut self) -> Vec<Effect> {
        self.conn = ConnState::Connected;
        let mut fx = vec![
            call(Request::GetSelf, ReplyTag::SelfInfo),
            call(Request::ListConversations, ReplyTag::Conversations),
            call(Request::ListRequests, ReplyTag::Requests),
            call(Request::GetSettings, ReplyTag::Settings),
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
                } else if self.page == Page::Wizard {
                    self.page = Page::Chats;
                }
                self.me = Some(value);
                Vec::new()
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
                    let mut merged = value;
                    merged.append(&mut self.messages);
                    merged.truncate(MESSAGE_WINDOW_MAX);
                    self.messages = merged;
                    Vec::new()
                } else {
                    self.messages = value;
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
                hash,
                done,
                total,
                bytes_per_sec,
                ..
            } => {
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
                if !ok {
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
        if !self.has_older {
            return Vec::new();
        }
        vec![self.list_messages_call(conv, Some(first.id.clone()), true)]
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
            .map(|e| match e {
                Effect::Call(r, _) => r,
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
}
