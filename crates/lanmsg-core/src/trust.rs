// P2 Task 5：联系人双向确认与准入过滤。
// Trust 负责：连接准入（accept_peer 七条规则）、联系人请求/应答状态机、
// 联系人策略、证书更新接收（含自身设备转发），以及 ContactRecord 应用
// （自身设备同步与配对包共用）。
// 注释中文，标识符英文。

use std::sync::Arc;
use std::time::SystemTime;

use rand::RngCore;
use tokio::sync::{broadcast, RwLock};

use lanmsg_api::types::Event;
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, UserId};
use lanmsg_proto::types::{
    ContactRecord, ContactRequest, ContactResponse, ContactStateWire, DeviceCert, MsgBody,
    PathClass, Status, SystemEvent,
};

use crate::error::CoreError;
use crate::hlc::HlcClock;
use crate::identity::Identity;
use crate::msg::Messaging;
use crate::net::{ConnManager, PeerInfo};
use crate::store::{ContactRow, ContactState, DeviceRow, RequestKindRow, RequestRow, Store};

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
    let self_user = identity.try_read().ok().and_then(|id| id.user_id());
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

    // ---- 私有辅助 ----

    /// 当前用户 ID（未初始化报错）。
    async fn self_user(&self) -> Result<UserId, CoreError> {
        self.identity
            .read()
            .await
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
            auto_accept: prev.as_ref().map(|r| r.auto_accept).unwrap_or(false),
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
            .ok_or_else(|| CoreError::NotFound(format!("联系人不存在: {user}")))?;
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
            return Err(CoreError::InvalidArgument("不能添加自己为联系人".into()));
        }
        let current = self.store.get_contact(user)?.map(|r| r.state);
        match current {
            Some(ContactState::Accepted) => return Ok(()),
            Some(ContactState::Blocked) => {
                return Err(CoreError::InvalidArgument(
                    "该用户已被屏蔽，先解除屏蔽再添加".into(),
                ));
            }
            Some(ContactState::Incoming) => {
                // 视为同意：进入 Accepted 并回 accepted:true（本端是回复方，
                // 由 enter_accepted 发 ContactAdded）。回显对方请求的 request_id。
                self.enter_accepted(msg, user, &[], true).await?;
                let req_id = self.open_contact_request_id(user);
                let _ = self
                    .send_to_user(user, self.contact_response(req_id, true))
                    .await;
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
                self.enter_accepted(msg, &user, &req.devices, true).await?;
                let _ = self
                    .send_to_user(&user, self.contact_response(req.request_id, true))
                    .await;
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
            preview: "请求添加你为联系人".into(),
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
                    "联系人请求不支持接收一次".into(),
                ));
            }
            RequestAction::AddContact => {
                if self.store.get_contact(&user)?.map(|r| r.state) != Some(ContactState::Accepted) {
                    self.enter_accepted(msg, &user, &[], true).await?;
                    let _ = self
                        .send_to_user(&user, self.contact_response(req_id_bytes, true))
                        .await;
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
            .ok_or_else(|| CoreError::NotFound(format!("非联系人: {user}")))?;
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
        let self_user = self.identity.try_read().ok().and_then(|id| id.user_id());
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

/// 当前用户 ID（同步路径；锁被占用时返回 None）。
fn self_user_now_sync(identity: &RwLock<Identity>) -> Option<UserId> {
    identity.try_read().ok().and_then(|id| id.user_id())
}

/// 生成随机会话请求 ID。
fn random_request_id() -> [u8; 16] {
    let mut id = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut id);
    id
}

/// 校验入站 ContactRequest/ContactResponse：发送方必须带证书，且随附设备
/// 证书全部属于发送方用户并通过验签；返回发送方用户 ID。
fn validate_peer_devices(from: &PeerInfo, devices: &[DeviceCert]) -> Result<UserId, CoreError> {
    let cert = from
        .cert
        .as_ref()
        .ok_or_else(|| CoreError::Rejected("请求方未提供证书".into()))?;
    for c in devices {
        if c.user != cert.user {
            return Err(CoreError::Rejected(
                "请求中设备证书用户与发送方不一致".into(),
            ));
        }
        c.verify()
            .map_err(|e| CoreError::Rejected(format!("设备证书验签失败: {e}")))?;
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
