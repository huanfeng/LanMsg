// P2 Task 5：联系人双向确认与准入过滤。
// Trust 负责：连接准入（accept_peer 七条规则）、联系人请求/应答状态机、
// 联系人策略、证书更新接收（含自身设备转发），以及 ContactRecord 应用
// （自身设备同步与配对包共用）。
// 注释中文，标识符英文。

use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use rand::RngCore;
use tokio::sync::broadcast;

use lanmsg_api::types::{Event, RequestPreviewView};
use lanmsg_proto::consts::STRANGER_GATE_WINDOW;
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId, UserId};
use lanmsg_proto::types::{
    ContactRecord, ContactRequest, ContactResponse, ContactStateWire, DeviceCert, Envelope,
    MsgBody, PathClass, Revocation, Status, SystemEvent,
};

use crate::error::CoreError;
use crate::hlc::HlcClock;
use crate::identity::Identity;
use crate::msg::Messaging;
use crate::net::{ConnManager, PeerInfo};
use crate::store::{
    ContactRow, ContactState, ConvKindRow, ConvRow, DeviceRow, Gate, RequestKindRow, RequestRow,
    Store,
};

/// 当前墙钟时间（毫秒）。
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// accept_peer 的规则实现（Trust::accept_peer 与 Service 的 accept_filter 复用）。
/// 拆成自由函数是因为 ConnManager::bind 需要 accept_filter 闭包先于 Trust 构造。
pub(crate) fn accept_rules(
    store: &Store,
    identity: &RwLock<Identity>,
    p: &PeerInfo,
    status: Status,
    pair_window_open: bool,
) -> bool {
    // 规则 1：无证书对端仅在配对窗口开放时接受。
    let Some(cert) = &p.cert else {
        return pair_window_open;
    };
    // 规则 2：设备已吊销 → 拒绝。
    if store.is_revoked(&p.device).unwrap_or(false) {
        return false;
    }
    // 规则 3：自己的设备 → 接受。
    let self_user = identity.read().unwrap().user_id();
    if self_user.is_some_and(|u| u == cert.user) {
        return true;
    }
    let state = store
        .get_contact(&cert.user)
        .ok()
        .flatten()
        .map(|r| r.state);
    // 规则 4：Blocked → 拒绝。
    if state == Some(ContactState::Blocked) {
        return false;
    }
    // 规则 5：Invisible → 拒绝（自己的设备已在规则 3 接受）。
    if matches!(status, Status::Invisible) {
        return false;
    }
    // 规则 6：Accepted 联系人 → 任意路径接受。
    if state == Some(ContactState::Accepted) {
        return true;
    }
    // 规则 7：陌生人按路径与方向：LAN 接受；VPN 仅出站接受；其余拒绝。
    match p.path {
        PathClass::Lan => true,
        PathClass::Vpn => p.outbound,
        PathClass::Internet | PathClass::Relay => false,
    }
}

/// 陌生人消息门控决策（P2 Task 6）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// 直接入库可见。
    Accept,
    /// 入库隐藏并登记/合并待处理请求。
    Pending {
        /// open 请求 ID（同作者同类型折叠复用）。
        request_id: String,
    },
    /// 丢弃：不入库、不回 Ack。
    Drop,
}

/// 联系人与信任模块。
#[derive(Clone)]
pub struct Trust {
    store: Arc<Store>,
    identity: Arc<RwLock<Identity>>,
    clock: Arc<std::sync::Mutex<HlcClock>>,
    conns: ConnManager,
    events: broadcast::Sender<Event>,
}

impl Trust {
    /// 构造 Trust。
    pub fn new(
        store: Arc<Store>,
        identity: Arc<RwLock<Identity>>,
        clock: Arc<std::sync::Mutex<HlcClock>>,
        conns: ConnManager,
        events: broadcast::Sender<Event>,
    ) -> Trust {
        Trust {
            store,
            identity,
            clock,
            conns,
            events,
        }
    }

    /// 连接准入：按七条规则判定（status 与 pair_window_open 取调用时的值）。
    pub fn accept_peer(&self, p: &PeerInfo, status: Status, pair_window_open: bool) -> bool {
        accept_rules(&self.store, &self.identity, p, status, pair_window_open)
    }

    /// 陌生人消息门控：按七条规则判定入站消息去向（规则顺序即优先级）。
    ///
    /// 1. from 设备属于自己 → Accept（自己设备转交一律信任，门控只在首个
    ///    接收设备上发生）；2. 作者用户是自己 → Accept；3. 作者 Blocked →
    ///    Drop；4. 作者 Accepted → Accept；5. 会话存在且 gate Open、窗口为空
    ///    或未到 → Accept；6. 非 Lan 且非（Vpn 且出站）→ Drop；7. 其余 →
    ///    会话置 Pending 并登记/合并 open 请求 → Pending。
    pub fn gate_inbound(
        &self,
        from: &PeerInfo,
        env: &Envelope,
        now: i64,
    ) -> Result<GateDecision, CoreError> {
        let self_user = self_user_now_sync(&self.identity);

        // 规则 1：from 设备属于自己 → Accept。
        if let Some(cert) = &from.cert {
            if self_user.is_some_and(|u| u == cert.user) {
                return Ok(GateDecision::Accept);
            }
        }

        let author_user = self
            .store
            .get_device(&env.id.author)?
            .map(|r| r.user)
            .ok_or_else(|| CoreError::Rejected("author device not in store".into()))?;

        // 规则 2：作者用户是自己 → Accept。
        if self_user.is_some_and(|u| u == author_user) {
            return Ok(GateDecision::Accept);
        }
        let state = self.store.get_contact(&author_user)?.map(|r| r.state);
        // 规则 3：作者用户 Blocked → Drop。
        if state == Some(ContactState::Blocked) {
            return Ok(GateDecision::Drop);
        }
        // 规则 4：作者用户 Accepted 联系人 → Accept。
        if state == Some(ContactState::Accepted) {
            return Ok(GateDecision::Accept);
        }
        // 规则 5：会话已存在且 gate Open、窗口为空或未到 → Accept。
        if self.store.get_conversation(&env.conv)?.is_some() {
            let (gate, until) = self.store.get_gate(&env.conv)?;
            if gate == Gate::Open && until.is_none_or(|t| t > now) {
                return Ok(GateDecision::Accept);
            }
        }
        // 规则 6：路径过滤：非 Lan 且非（Vpn 且出站）→ Drop。
        let path_ok = match from.path {
            PathClass::Lan => true,
            PathClass::Vpn => from.outbound,
            PathClass::Internet | PathClass::Relay => false,
        };
        if !path_ok {
            return Ok(GateDecision::Drop);
        }
        // 规则 7：会话置 Pending 并登记/合并 open 请求。
        let kind = gate_request_kind(&env.body);
        let preview = request_preview(&env.body);
        // 会话不存在则先建（set_gate 需要行存在）。
        if self.store.get_conversation(&env.conv)?.is_none() {
            self.store.ensure_conversation(&ConvRow {
                conv: env.conv,
                kind: ConvKindRow::Dm,
                peer_user: Some(author_user),
                title: None,
                created_at: now,
            })?;
        }
        self.store.set_gate(&env.conv, Gate::Pending, None)?;
        // 复用同 (from_user, kind) 的既有 open 请求 id，仅更新 preview；无则新建。
        let request_id = self
            .store
            .list_open_requests()?
            .into_iter()
            .find(|r| r.kind == kind && r.from_user == author_user)
            .map(|r| r.request_id)
            .unwrap_or_else(|| crate::identity::hex_encode(&random_request_id()));
        self.store.upsert_open_request(&RequestRow {
            request_id: request_id.clone(),
            kind,
            from_user: author_user,
            conv: Some(env.conv),
            preview,
            created_at: now,
        })?;
        Ok(GateDecision::Pending { request_id })
    }

    // ---- 私有辅助 ----

    /// 当前用户 ID（未初始化报错）。
    async fn self_user(&self) -> Result<UserId, CoreError> {
        self.identity
            .read()
            .unwrap()
            .user_id()
            .ok_or(CoreError::NotInitialized)
    }

    /// 生成新 HLC（本地严格递增，保证 if_newer 写入生效）。
    fn next_hlc(&self) -> lanmsg_proto::types::Hlc {
        let mut clock = self.clock.lock().unwrap();
        clock.now(now_millis())
    }

    /// 写联系人状态（保留策略字段，hlc 取新值），仅严格更新才落库。
    fn set_state(&self, user: &UserId, state: ContactState) -> Result<(), CoreError> {
        let prev = self.store.get_contact(user)?;
        let row = ContactRow {
            user: *user,
            state,
            // 新建联系人行时默认 true（计划迁移 2：auto_accept DEFAULT 1）；
            // 已有行保留原值。
            auto_accept: prev.as_ref().map(|r| r.auto_accept).unwrap_or(true),
            ask_above_bytes: prev.and_then(|r| r.ask_above_bytes),
            updated: self.next_hlc(),
        };
        self.store.upsert_contact_if_newer(&row)?;
        Ok(())
    }

    /// 写入对方设备证书（保留既有 last_* 字段，避免覆盖最近活跃信息）。
    fn upsert_device_cert(&self, user: &UserId, cert: &DeviceCert) -> Result<(), CoreError> {
        let prev = self.store.get_device(&cert.device)?;
        let (last_addr, last_path, last_seen) = match prev {
            Some(p) => (p.last_addr, p.last_path, p.last_seen),
            None => (None, None, None),
        };
        self.store.upsert_device(&DeviceRow {
            device: cert.device,
            user: *user,
            cert: cert.clone(),
            name: cert.device_name.clone(),
            device_type: cert.device_type,
            last_addr,
            last_path,
            last_seen,
            revoked: false,
        })?;
        Ok(())
    }

    /// 把 ContactRequest/ContactResponse 发给对方全部已连接设备。
    async fn send_to_user(&self, user: &UserId, frame: Frame) -> Result<(), CoreError> {
        let targets: Vec<_> = self
            .conns
            .peers()
            .into_iter()
            .filter(|p| p.cert.as_ref().is_some_and(|c| c.user == *user))
            .map(|p| p.device)
            .collect();
        if targets.is_empty() {
            return Err(CoreError::PeerOffline);
        }
        for device in &targets {
            let _ = self.conns.send(device, frame.clone()).await;
        }
        Ok(())
    }

    /// 状态进入 Accepted 的公共动作：写对方设备证书、确保 DM 会话、关闭
    /// 该用户的 open 联系人请求、置 Accepted；`grant` 为真时（本端回复同意）
    /// 在 DM 发可见消息 System{ContactAdded}；最后同步 ContactUpdated 并发
    /// ContactsChanged。
    async fn enter_accepted(
        &self,
        msg: &Messaging,
        user: &UserId,
        peer_devices: &[DeviceCert],
        grant: bool,
    ) -> Result<(), CoreError> {
        let self_user = self.self_user().await?;
        for cert in peer_devices {
            self.upsert_device_cert(user, cert)?;
        }
        let conv = ConvId::dm(&self_user, user);
        self.store.ensure_conversation(&crate::store::ConvRow {
            conv,
            kind: crate::store::ConvKindRow::Dm,
            peer_user: Some(*user),
            title: None,
            created_at: now_millis(),
        })?;
        self.close_contact_requests(user)?;
        self.set_state(user, ContactState::Accepted)?;
        if grant {
            msg.send(
                &conv,
                MsgBody::System {
                    event: SystemEvent::ContactAdded { user: *user },
                },
            )
            .await?;
        }
        self.publish_contact_updated(msg, user).await
    }

    /// 状态变化/策略变化后的同步：SelfDevices 会话发隐藏消息
    /// System{ContactUpdated(record)}，并发 ContactsChanged。
    async fn publish_contact_updated(
        &self,
        msg: &Messaging,
        user: &UserId,
    ) -> Result<(), CoreError> {
        let self_user = self.self_user().await?;
        let rec = self
            .contact_record(user)?
            .ok_or_else(|| CoreError::NotFound(format!("contact not found: {user}")))?;
        let conv = ConvId::self_devices(&self_user);
        msg.send_hidden(
            &conv,
            MsgBody::System {
                event: SystemEvent::ContactUpdated(rec),
            },
        )
        .await?;
        let _ = self.events.send(Event::ContactsChanged);
        Ok(())
    }

    /// 关闭该用户全部 open 的 Contact 类请求。
    fn close_contact_requests(&self, user: &UserId) -> Result<(), CoreError> {
        for r in self.store.list_open_requests()? {
            if r.kind == RequestKindRow::Contact && r.from_user == *user {
                self.store.close_request(&r.request_id)?;
            }
        }
        Ok(())
    }

    // ---- 联系人请求/应答状态机 ----

    /// 本端发起联系人请求。
    ///
    /// 状态机：Incoming → 视为同意（回 accepted:true）；Accepted → 不变；
    /// Blocked → 拒绝操作；其余（无/Rejected/Outgoing）→ 置 Outgoing 并向
    /// 对方全部已连接设备发 ContactRequest；对方无在线设备时返回
    /// PeerOffline 且不改状态。
    pub async fn request_contact(&self, msg: &Messaging, user: &UserId) -> Result<(), CoreError> {
        let self_user = self.self_user().await?;
        if *user == self_user {
            return Err(CoreError::InvalidArgument(
                "cannot add self as contact".into(),
            ));
        }
        let current = self.store.get_contact(user)?.map(|r| r.state);
        match current {
            Some(ContactState::Accepted) => return Ok(()),
            Some(ContactState::Blocked) => {
                return Err(CoreError::InvalidArgument(
                    "user is blocked, unblock before adding".into(),
                ));
            }
            Some(ContactState::Incoming) => {
                // 视为同意：进入 Accepted 并回 accepted:true（本端是回复方，
                // 由 enter_accepted 发 ContactAdded）。回显对方请求的 request_id。
                // 先回响应再发消息：对方需先经 ContactResponse 进入 Accepted，
                // 否则随后的 ContactAdded 会被对方的陌生人门控隐藏。
                let req_id = self.open_contact_request_id(user);
                let _ = self
                    .send_to_user(user, self.contact_response(req_id, true))
                    .await;
                self.enter_accepted(msg, user, &[], true).await?;
                return Ok(());
            }
            _ => {}
        }
        // 先确认对方有在线设备，再改状态（PeerOffline 且不改状态）。
        let devices: Vec<_> = self
            .conns
            .peers()
            .into_iter()
            .filter(|p| p.cert.as_ref().is_some_and(|c| c.user == *user))
            .map(|p| p.device)
            .collect();
        if devices.is_empty() {
            return Err(CoreError::PeerOffline);
        }
        if current != Some(ContactState::Outgoing) {
            self.set_state(user, ContactState::Outgoing)?;
        }
        let req = ContactRequest {
            request_id: random_request_id(),
            from_display_name: self.store.get_setting("display_name")?.unwrap_or_default(),
            devices: self
                .store
                .devices_of_user(&self_user)?
                .into_iter()
                .map(|d| d.cert)
                .collect(),
        };
        let _ = self.send_to_user(user, Frame::ContactRequest(req)).await;
        Ok(())
    }

    /// 收到对端 ContactRequest。
    ///
    /// 校验：发送方必须带证书，且请求中设备证书均属于发送方用户并通过验签。
    /// 状态机：Blocked → 忽略不回复；Accepted → 回 accepted:true（对方可能
    /// 丢了状态）；Outgoing → 自动同意（回 accepted:true，本端发 ContactAdded）；
    /// 其余 → 置 Incoming、登记 open 请求并发 IncomingRequest。
    pub async fn on_contact_request(
        &self,
        msg: &Messaging,
        from: &PeerInfo,
        req: ContactRequest,
    ) -> Result<(), CoreError> {
        let user = validate_peer_devices(from, &req.devices)?;
        for cert in &req.devices {
            self.upsert_device_cert(&user, cert)?;
        }
        let current = self.store.get_contact(&user)?.map(|r| r.state);
        match current {
            Some(ContactState::Blocked) => return Ok(()),
            Some(ContactState::Accepted) => {
                let _ = self
                    .send_to_user(&user, self.contact_response(req.request_id, true))
                    .await;
                return Ok(());
            }
            Some(ContactState::Outgoing) => {
                // 先回响应再进入 Accepted（理由同 request_contact 的 Incoming 分支）。
                let _ = self
                    .send_to_user(&user, self.contact_response(req.request_id, true))
                    .await;
                self.enter_accepted(msg, &user, &req.devices, true).await?;
                return Ok(());
            }
            _ => {}
        }
        if current != Some(ContactState::Incoming) {
            self.set_state(&user, ContactState::Incoming)?;
        }
        let row = RequestRow {
            request_id: crate::identity::hex_encode(&req.request_id),
            kind: RequestKindRow::Contact,
            from_user: user,
            conv: None,
            // 联系人请求没有内容预览，界面按 kind 显示本地化文案。
            preview: RequestPreviewView::None,
            created_at: now_millis(),
        };
        self.store.upsert_open_request(&row)?;
        let request_view = crate::service::views::request_view(&self.store, &row);
        let _ = self.events.send(Event::IncomingRequest {
            request: request_view,
        });
        Ok(())
    }

    /// 收到对端 ContactResponse。
    ///
    /// Outgoing：accepted:true → Accepted（本端非回复方，不发 ContactAdded）；
    /// accepted:false → Rejected。其余状态忽略（Blocked 静默、Accepted 已是
    /// 联系人）。
    pub async fn on_contact_response(
        &self,
        msg: &Messaging,
        from: &PeerInfo,
        resp: ContactResponse,
    ) -> Result<(), CoreError> {
        let user = validate_peer_devices(from, &resp.devices)?;
        for cert in &resp.devices {
            self.upsert_device_cert(&user, cert)?;
        }
        if self.store.get_contact(&user)?.map(|r| r.state) != Some(ContactState::Outgoing) {
            return Ok(());
        }
        if resp.accepted {
            self.enter_accepted(msg, &user, &[], false).await?;
        } else {
            self.set_state(&user, ContactState::Rejected)?;
            self.publish_contact_updated(msg, &user).await?;
        }
        Ok(())
    }

    /// 本端处理 open 请求（RespondRequest，kind == Contact）。
    ///
    /// AddContact：Incoming → Accepted（回 true，发 ContactAdded）；Reject：
    /// → Rejected（回 false）；Block：任意状态 → Blocked（回 false，关闭对方
    /// 全部连接）；AcceptOnce：非法。
    pub async fn respond_contact(
        &self,
        msg: &Messaging,
        req: &RequestRow,
        action: lanmsg_api::types::RequestAction,
    ) -> Result<(), CoreError> {
        use lanmsg_api::types::RequestAction;
        let user = req.from_user;
        let req_id_bytes = request_id_bytes(&req.request_id);
        match action {
            RequestAction::AcceptOnce => {
                return Err(CoreError::InvalidArgument(
                    "contact request does not support accept-once".into(),
                ));
            }
            RequestAction::AddContact => {
                if self.store.get_contact(&user)?.map(|r| r.state) != Some(ContactState::Accepted) {
                    // 先回响应再进入 Accepted（理由同 request_contact 的 Incoming 分支）。
                    let _ = self
                        .send_to_user(&user, self.contact_response(req_id_bytes, true))
                        .await;
                    self.enter_accepted(msg, &user, &[], true).await?;
                }
            }
            RequestAction::Reject => {
                if self.store.get_contact(&user)?.map(|r| r.state) != Some(ContactState::Rejected) {
                    self.set_state(&user, ContactState::Rejected)?;
                    self.publish_contact_updated(msg, &user).await?;
                }
                let _ = self
                    .send_to_user(&user, self.contact_response(req_id_bytes, false))
                    .await;
            }
            RequestAction::Block => {
                if self.store.get_contact(&user)?.map(|r| r.state) != Some(ContactState::Blocked) {
                    self.set_state(&user, ContactState::Blocked)?;
                    self.publish_contact_updated(msg, &user).await?;
                }
                // 先回 accepted:false（尽力而为），再断开对方全部连接。
                let _ = self
                    .send_to_user(&user, self.contact_response(req_id_bytes, false))
                    .await;
                for p in self.conns.peers() {
                    if p.cert.as_ref().is_some_and(|c| c.user == user) {
                        self.conns.disconnect(&p.device).await;
                    }
                }
            }
        }
        self.store.close_request(&req.request_id)?;
        Ok(())
    }

    /// 本端处理 open 消息/文件请求（RespondRequest，kind == Message/File）。
    ///
    /// AcceptOnce：gate 置 Open（窗口 = STRANGER_GATE_WINDOW）并揭示该用户
    /// 在本会话的全部非隐藏类消息，发 ConversationUpdated；AddContact：同
    /// AcceptOnce 但不设窗口，随后发起联系人请求；Reject：删除该用户在会话
    /// 中的消息、gate 置 Rejected、关闭请求（新消息重新进入规则 7）；Block：
    /// 同 Reject 并把联系人置 Blocked（断开对方全部连接）。
    pub async fn respond_message_request(
        &self,
        msg: &Messaging,
        req: &RequestRow,
        action: lanmsg_api::types::RequestAction,
        now: i64,
    ) -> Result<(), CoreError> {
        use lanmsg_api::types::RequestAction;
        let conv = req.conv.ok_or_else(|| {
            CoreError::InvalidArgument("message request is missing conversation".into())
        })?;
        let user = req.from_user;
        match action {
            RequestAction::AcceptOnce => {
                self.store.set_gate(
                    &conv,
                    Gate::Open,
                    Some(now + STRANGER_GATE_WINDOW.as_millis() as i64),
                )?;
                self.reveal_messages(&conv, &user)?;
                self.store.close_request(&req.request_id)?;
                self.publish_conversation_updated(&conv).await?;
            }
            RequestAction::AddContact => {
                self.store.set_gate(&conv, Gate::Open, None)?;
                self.reveal_messages(&conv, &user)?;
                self.store.close_request(&req.request_id)?;
                self.publish_conversation_updated(&conv).await?;
                self.request_contact(msg, &user).await?;
            }
            RequestAction::Reject => {
                self.reject_and_close(&conv, &req.request_id, &user)?;
            }
            RequestAction::Block => {
                self.reject_and_close(&conv, &req.request_id, &user)?;
                if self.store.get_contact(&user)?.map(|r| r.state) != Some(ContactState::Blocked) {
                    self.set_state(&user, ContactState::Blocked)?;
                    self.publish_contact_updated(msg, &user).await?;
                }
                // 断开对方全部连接（Task 5 Block 效果）。
                for p in self.conns.peers() {
                    if p.cert.as_ref().is_some_and(|c| c.user == user) {
                        self.conns.disconnect(&p.device).await;
                    }
                }
            }
        }
        Ok(())
    }

    /// Reject/Block 公共部分：删除该用户在会话中的消息、gate 置 Rejected、
    /// 关闭请求。
    fn reject_and_close(
        &self,
        conv: &ConvId,
        request_id: &str,
        user: &UserId,
    ) -> Result<(), CoreError> {
        let ids = self.store.messages_of_user_in_conv(conv, user)?;
        self.store.mark_deleted(&ids)?;
        self.store.set_gate(conv, Gate::Rejected, None)?;
        self.store.close_request(request_id)?;
        Ok(())
    }

    /// 揭示会话中该用户的全部非隐藏类消息（AcceptOnce/AddContact 用）。
    fn reveal_messages(&self, conv: &ConvId, user: &UserId) -> Result<(), CoreError> {
        let mut visible = Vec::new();
        for id in self.store.messages_of_user_in_conv(conv, user)? {
            if let Some(m) = self.store.get_message(&id)? {
                if !lanmsg_proto::body_is_hidden(&m.env.body) {
                    visible.push(id);
                }
            }
        }
        self.store.set_hidden(&visible, false)?;
        Ok(())
    }

    /// 会话可见性变化后同步 ConversationUpdated。
    async fn publish_conversation_updated(&self, conv: &ConvId) -> Result<(), CoreError> {
        let self_user = self.self_user().await?;
        let conv_row = self
            .store
            .get_conversation(conv)?
            .ok_or_else(|| CoreError::NotFound(format!("conversation not found: {conv}")))?;
        let peer_online = match conv_row.kind {
            ConvKindRow::Dm => conv_row.peer_user.is_some_and(|pu| {
                self.conns
                    .peers()
                    .iter()
                    .any(|p| p.cert.as_ref().is_some_and(|c| c.user == pu))
            }),
            _ => false,
        };
        let _ = self.events.send(Event::ConversationUpdated {
            summary: crate::service::views::conv_summary(
                &self.store,
                Some(&self_user),
                &conv_row,
                peer_online,
            ),
        });
        Ok(())
    }

    /// 设置联系人策略（非联系人 → NotFound）；变化同步 ContactUpdated 并发
    /// ContactsChanged。
    pub async fn set_policy(
        &self,
        msg: &Messaging,
        user: &UserId,
        auto_accept: bool,
        ask_above: Option<u64>,
    ) -> Result<(), CoreError> {
        let prev = self
            .store
            .get_contact(user)?
            .ok_or_else(|| CoreError::NotFound(format!("not a contact: {user}")))?;
        let row = ContactRow {
            user: *user,
            state: prev.state,
            auto_accept,
            ask_above_bytes: ask_above,
            updated: self.next_hlc(),
        };
        self.store.upsert_contact_if_newer(&row)?;
        self.publish_contact_updated(msg, user).await
    }

    // ---- 联系人记录与证书更新 ----

    /// 应用一条联系人记录（自身设备同步与配对包共用）：联系人行按 HLC 严格
    /// 更新写入，设备证书入库；返回联系人行是否变化。
    pub fn apply_contact_record(&self, rec: &ContactRecord) -> Result<bool, CoreError> {
        let state = contact_state_from_wire(rec.state);
        let written = self.store.upsert_contact_if_newer(&ContactRow {
            user: rec.user,
            state,
            auto_accept: rec.auto_accept,
            ask_above_bytes: rec.ask_above_bytes,
            updated: rec.updated.clone(),
        })?;
        for cert in &rec.devices {
            self.upsert_device_cert(&rec.user, cert)?;
        }
        Ok(written)
    }

    /// 收到 CertUpdate 帧：逐证书验签，证书所属用户必须是自己或 Accepted
    /// 联系人且设备未吊销，入库；自己的新设备证书转发给其他已连接的自己
    /// 设备（只转发 store 中先前没有的）。
    pub async fn on_cert_update(
        &self,
        from: &PeerInfo,
        certs: Vec<DeviceCert>,
    ) -> Result<(), CoreError> {
        let Some(from_cert) = &from.cert else {
            return Ok(());
        };
        let self_user = self.identity.read().unwrap().user_id();
        let mut forward: Vec<DeviceCert> = Vec::new();
        for cert in certs {
            if cert.verify().is_err() {
                tracing::warn!(peer = %from.device, "CertUpdate 证书验签失败，忽略");
                continue;
            }
            // 计划规则（自己或 Accepted 联系人）之外，放宽接受"发送方用户
            // 自己的证书"：发送方经 TLS+Hello 已认证，只能注入本人证书，且
            // 保住陌生人在线改名的既有行为（Task 4 测试）。
            let allowed = self_user.is_some_and(|u| u == cert.user)
                || cert.user == from_cert.user
                || self
                    .store
                    .get_contact(&cert.user)
                    .ok()
                    .flatten()
                    .is_some_and(|r| r.state == ContactState::Accepted);
            if !allowed {
                tracing::warn!(peer = %from.device, "CertUpdate 证书用户非自己或联系人，忽略");
                continue;
            }
            if self.store.is_revoked(&cert.device)? {
                tracing::warn!(peer = %from.device, "CertUpdate 证书设备已吊销，忽略");
                continue;
            }
            let known = self.store.get_device(&cert.device)?.is_some();
            self.upsert_device_cert(&cert.user, &cert)?;
            if cert.device == from.device {
                self.conns
                    .update_peer_cert(&from.device, cert.clone())
                    .await;
            }
            if !known && cert.user == from_cert.user && cert.device != from.device {
                forward.push(cert);
            }
        }
        if forward.is_empty() {
            return Ok(());
        }
        // 转发给自己其他已连接设备（不含来源设备）。
        for p in self.conns.peers() {
            let mine = p
                .cert
                .as_ref()
                .is_some_and(|c| Some(c.user) == self_user && p.device != from.device);
            if mine {
                let _ = self
                    .conns
                    .send(&p.device, Frame::CertUpdate(forward.clone()))
                    .await;
            }
        }
        Ok(())
    }

    /// 查找该用户的 open Contact 请求 ID 字节（无则全零；应答时回显用）。
    fn open_contact_request_id(&self, user: &UserId) -> [u8; 16] {
        for r in self.store.list_open_requests().unwrap_or_default() {
            if r.kind == RequestKindRow::Contact && r.from_user == *user {
                return request_id_bytes(&r.request_id);
            }
        }
        [0u8; 16]
    }

    // ---- 设备吊销（P2 Task 8）----

    /// 移除自己的设备：本设备 → InvalidArgument；目标不存在或不属于自己 →
    /// NotFound。签发吊销记录（用户密钥签名）入库并向所有已连接、已初始化
    /// 对端（自己设备 + 联系人）发 Frame::Revocation，随后断开该设备；最后
    /// 在 SelfDevices 会话发可见消息 System{DeviceRemoved} 并发 DevicesChanged。
    pub async fn revoke(&self, msg: &Messaging, device: &DeviceId) -> Result<(), CoreError> {
        let self_user = self.self_user().await?;
        let self_device = self.identity.read().unwrap().device_id();
        if *device == self_device {
            return Err(CoreError::InvalidArgument(
                "cannot remove this device".into(),
            ));
        }
        let row = self
            .store
            .get_device(device)?
            .ok_or_else(|| CoreError::NotFound(format!("device not found: {device}")))?;
        if row.user != self_user {
            return Err(CoreError::NotFound(format!(
                "device does not belong to current user: {device}"
            )));
        }
        // 吊销记录须由用户密钥签发（读锁作用域内取引用，无 await 穿越锁）。
        let rev = {
            let id = self.identity.read().unwrap();
            let user = id.user.as_ref().ok_or(CoreError::NotInitialized)?;
            Revocation::issue(user, *device, now_millis())
        };
        self.store.insert_revocation(&rev)?;
        self.store.revoke_device(device)?;
        // 发给所有已连接、已初始化对端（带证书即已完成初始化）。
        for p in self.conns.peers() {
            if p.cert.is_some() {
                let _ = self
                    .conns
                    .send(&p.device, Frame::Revocation(rev.clone()))
                    .await;
            }
        }
        // 缓冲后再断开（同 Task 7）：Revocation 帧经 write 通道异步冲刷，
        // 立即断开会因读半部带未读数据关闭 socket（RST）而丢失在途帧。
        if self.conns.is_connected(device) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            self.conns.disconnect(device).await;
        }
        msg.send(
            &ConvId::self_devices(&self_user),
            MsgBody::System {
                event: SystemEvent::DeviceRemoved { device: *device },
            },
        )
        .await?;
        let _ = self.events.send(Event::DevicesChanged);
        Ok(())
    }

    /// 收到对端 Revocation 帧：验签（r.user 的公钥）；r.user 必须是自己或
    /// Accepted 联系人；目标设备的已知证书 user 必须等于 r.user（未知设备也
    /// 接受并入库，以便将来拒绝）。新插入时：revoke_device、断开该设备、
    /// 转发给其他已连接的自己设备（不含来源），并按归属发 DevicesChanged
    /// （自己）/ ContactsChanged（联系人）。被吊销的是本机时：清空内存身份
    /// （此后按未初始化处理；HelloInfo 证书清空与停通告由 Service 层完成）。
    /// 返回是否新插入。
    pub async fn on_revocation(&self, from: &PeerInfo, r: Revocation) -> Result<bool, CoreError> {
        if r.verify().is_err() {
            return Err(CoreError::Rejected(
                "revocation signature verification failed".into(),
            ));
        }
        let self_device = self.identity.read().unwrap().device_id();
        let self_user = self_user_now_sync(&self.identity);
        let is_self_user = self_user.is_some_and(|u| u == r.user);
        if !is_self_user && !self.is_contact(&r.user) {
            return Err(CoreError::Rejected(
                "revocation publisher is neither self nor a contact".into(),
            ));
        }
        if let Some(row) = self.store.get_device(&r.device)? {
            if row.user != r.user {
                return Err(CoreError::Rejected(
                    "revoked device does not belong to the user".into(),
                ));
            }
        }
        if !self.store.insert_revocation(&r)? {
            return Ok(false);
        }
        self.store.revoke_device(&r.device)?;
        if r.device == self_device {
            // 本机被吊销：清空内存身份，无连接可断。
            self.identity.write().unwrap().mark_revoked_self();
        } else {
            self.conns.disconnect(&r.device).await;
        }
        // 转发给其他已连接的自己设备（不含来源与被吊销设备本身）。
        // 发送放后台任务：慢对端的写队列不应阻塞事件循环。
        let targets: Vec<DeviceId> = self
            .conns
            .peers()
            .into_iter()
            .filter(|p| {
                p.cert.as_ref().is_some_and(|c| Some(c.user) == self_user)
                    && p.device != from.device
                    && p.device != r.device
            })
            .map(|p| p.device)
            .collect();
        if !targets.is_empty() {
            let conns = self.conns.clone();
            let rev = r.clone();
            tokio::spawn(async move {
                for d in targets {
                    if let Err(e) = conns.send(&d, Frame::Revocation(rev.clone())).await {
                        tracing::debug!(peer = %d, error = %e, "吊销转发发送失败");
                    }
                }
            });
        }
        if is_self_user {
            let _ = self.events.send(Event::DevicesChanged);
        } else {
            let _ = self.events.send(Event::ContactsChanged);
        }
        Ok(true)
    }

    // ---- 查询 ----

    /// 是否为已接受联系人。
    pub fn is_contact(&self, user: &UserId) -> bool {
        self.store
            .get_contact(user)
            .ok()
            .flatten()
            .is_some_and(|r| r.state == ContactState::Accepted)
    }

    /// 构造联系人记录（配对包与同步用）；无联系人行返回 None。
    pub fn contact_record(&self, user: &UserId) -> Result<Option<ContactRecord>, CoreError> {
        let Some(row) = self.store.get_contact(user)? else {
            return Ok(None);
        };
        let display_name = self
            .store
            .get_user(user)?
            .map(|u| u.display_name)
            .unwrap_or_else(|| user.short());
        let devices = self
            .store
            .devices_of_user(user)?
            .into_iter()
            .map(|d| d.cert)
            .collect();
        Ok(Some(ContactRecord {
            user: *user,
            display_name,
            state: contact_state_to_wire(row.state),
            devices,
            auto_accept: row.auto_accept,
            ask_above_bytes: row.ask_above_bytes,
            updated: row.updated,
        }))
    }

    /// 构造本端发出的 ContactResponse（回显请求 ID；设备证书带上自己的全部设备）。
    fn contact_response(&self, request_id: [u8; 16], accepted: bool) -> Frame {
        let devices = self_user_now_sync(&self.identity)
            .map(|u| {
                self.store
                    .devices_of_user(&u)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|d| d.cert)
                    .collect()
            })
            .unwrap_or_default();
        Frame::ContactResponse(ContactResponse {
            request_id,
            accepted,
            devices,
        })
    }
}

// ---- 模块级辅助 ----

/// 当前用户 ID（同步路径；std 锁持有极短，直接阻塞读取）。
fn self_user_now_sync(identity: &RwLock<Identity>) -> Option<UserId> {
    identity.read().unwrap().user_id()
}

/// 生成随机会话请求 ID。
fn random_request_id() -> [u8; 16] {
    let mut id = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut id);
    id
}

/// 消息体对应的请求类型：Files/Folder → File，其余 → Message。
pub(crate) fn gate_request_kind(body: &MsgBody) -> RequestKindRow {
    match body {
        MsgBody::Files { .. } | MsgBody::Folder { .. } => RequestKindRow::File,
        _ => RequestKindRow::Message,
    }
}

/// 请求预览（结构化）：Text 取前 50 字符；Files 带首个文件名、个数与总大小；
/// Folder 带文件夹名与文件个数；其余无可预览内容。文案由各端按本地语言拼。
fn request_preview(body: &MsgBody) -> RequestPreviewView {
    match body {
        MsgBody::Text { text } => RequestPreviewView::Text {
            text: text.chars().take(50).collect(),
        },
        MsgBody::Files { items } => RequestPreviewView::Files {
            first_name: items.first().map(|b| b.name.clone()).unwrap_or_default(),
            count: items.len() as u32,
            total_size: items.iter().map(|b| b.size).sum(),
        },
        MsgBody::Folder { name, entries } => RequestPreviewView::Folder {
            name: name.clone(),
            count: entries.len() as u32,
        },
        _ => RequestPreviewView::None,
    }
}

/// 校验入站 ContactRequest/ContactResponse：发送方必须带证书，且随附设备
/// 证书全部属于发送方用户并通过验签；返回发送方用户 ID。
fn validate_peer_devices(from: &PeerInfo, devices: &[DeviceCert]) -> Result<UserId, CoreError> {
    let cert = from
        .cert
        .as_ref()
        .ok_or_else(|| CoreError::Rejected("requester did not provide a certificate".into()))?;
    for c in devices {
        if c.user != cert.user {
            return Err(CoreError::Rejected(
                "user of device certificate in request does not match sender".into(),
            ));
        }
        c.verify().map_err(|e| {
            CoreError::Rejected(format!("device certificate verification failed: {e}"))
        })?;
    }
    Ok(cert.user)
}

/// ContactState → 线上状态。
fn contact_state_to_wire(s: ContactState) -> ContactStateWire {
    match s {
        ContactState::Outgoing => ContactStateWire::Outgoing,
        ContactState::Incoming => ContactStateWire::Incoming,
        ContactState::Accepted => ContactStateWire::Accepted,
        ContactState::Rejected => ContactStateWire::Rejected,
        ContactState::Blocked => ContactStateWire::Blocked,
    }
}

/// 线上状态 → ContactState。
fn contact_state_from_wire(w: ContactStateWire) -> ContactState {
    match w {
        ContactStateWire::Outgoing => ContactState::Outgoing,
        ContactStateWire::Incoming => ContactState::Incoming,
        ContactStateWire::Accepted => ContactState::Accepted,
        ContactStateWire::Rejected => ContactState::Rejected,
        ContactStateWire::Blocked => ContactState::Blocked,
    }
}

/// hex 字符串 → 16 字节请求 ID（非法则全零）。
fn request_id_bytes(hex: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    let bytes = crate::identity::hex_decode(hex).unwrap_or_default();
    for (i, b) in bytes.iter().take(16).enumerate() {
        out[i] = *b;
    }
    out
}

#[cfg(test)]
mod request_preview_tests {
    use super::*;
    use lanmsg_proto::ids::BlobHash;
    use lanmsg_proto::types::{BlobMeta, FolderEntry};

    fn blob(name: &str, size: u64) -> BlobMeta {
        BlobMeta {
            hash: BlobHash([1u8; 32]),
            size,
            name: name.into(),
            mime: "application/octet-stream".into(),
            media: None,
            thumb: None,
        }
    }

    #[test]
    fn text_is_truncated_to_50_chars() {
        let p = request_preview(&MsgBody::Text {
            text: "测".repeat(80),
        });
        assert_eq!(
            p,
            RequestPreviewView::Text {
                text: "测".repeat(50)
            }
        );
    }

    #[test]
    fn files_carry_first_name_count_and_total() {
        let p = request_preview(&MsgBody::Files {
            items: vec![blob("a.bin", 10), blob("b.bin", 32)],
        });
        assert_eq!(
            p,
            RequestPreviewView::Files {
                first_name: "a.bin".into(),
                count: 2,
                total_size: 42
            }
        );
    }

    #[test]
    fn folder_carries_name_and_count() {
        let p = request_preview(&MsgBody::Folder {
            name: "docs".into(),
            entries: vec![FolderEntry {
                rel_path: "x.md".into(),
                blob: blob("x.md", 1),
            }],
        });
        assert_eq!(
            p,
            RequestPreviewView::Folder {
                name: "docs".into(),
                count: 1
            }
        );
    }

    #[test]
    fn other_bodies_have_no_preview() {
        let p = request_preview(&MsgBody::Read {
            upto: lanmsg_proto::types::Hlc {
                millis: 0,
                counter: 0,
            },
        });
        assert_eq!(p, RequestPreviewView::None);
    }
}
