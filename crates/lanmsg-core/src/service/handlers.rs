// P1 核心骨架：请求处理器（Task 11）。
// 已实现：GetSelf、Init{FirstDevice}、SetStatus、ListPeers、ListConversations、
// ListMessages、OpenDm、SendText、AddManualPeer、Shutdown。
// 其余一律返回 `未实现：<变体名>`（唯一允许的占位，后续阶段替换）。
// 注释中文，标识符英文。

use std::net::SocketAddr;
use std::str::FromStr;

use lanmsg_api::types::{
    ApiError, ErrorCode, InitMode, Request, Response, SelfInfo, SettingsDto, StatusDto,
};
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, MsgId, UserId};
use lanmsg_proto::types::{DeviceType, MsgBody, Status};

use crate::service::views::{conv_summary, message_view, peer_view};
use crate::service::{build_announce, now_millis, Inner};
use crate::store::{ConvKindRow, DeviceRow, UserRow};

/// 请求入口。
pub(crate) async fn handle(inner: &Inner, req: Request) -> Result<Response, ApiError> {
    match req {
        Request::GetSelf => get_self(inner).await,
        Request::Init {
            display_name,
            device_name,
            mode,
        } => init(inner, display_name, device_name, mode).await,
        Request::SetStatus { status } => set_status(inner, status).await,
        Request::ListPeers => list_peers(inner).await,
        Request::ListConversations => list_conversations(inner).await,
        Request::ListMessages {
            conv,
            before,
            limit,
        } => list_messages(inner, conv, before, limit).await,
        Request::OpenDm { user } => open_dm(inner, user).await,
        Request::SendText { conv, text } => send_text(inner, conv, text).await,
        Request::GetSettings => get_settings(inner).await,
        Request::SetSettings { settings } => set_settings(inner, settings).await,
        Request::SetProfile { display_name } => set_profile(inner, display_name).await,
        Request::AddManualPeer { addr } => add_manual_peer(inner, addr).await,
        Request::ListContacts => list_contacts(inner).await,
        Request::RequestContact { user } => request_contact(inner, user).await,
        Request::ListRequests => list_requests(inner).await,
        Request::RespondRequest { request, action } => {
            respond_request(inner, request, action).await
        }
        Request::SetContactPolicy {
            user,
            auto_accept,
            ask_above_bytes,
        } => set_contact_policy(inner, user, auto_accept, ask_above_bytes).await,
        Request::Shutdown => {
            inner.discovery.shutdown();
            inner.conns.shutdown().await;
            Ok(Response::Ok)
        }
        other => Err(ApiError {
            code: ErrorCode::Internal,
            message: format!("未实现：{}", variant_name(&other)),
        }),
    }
}

async fn get_self(inner: &Inner) -> Result<Response, ApiError> {
    let id = inner.identity.read().await;
    let st = inner.state.read().unwrap();
    let display = inner.store.get_setting("display_name").ok().flatten();
    let device_name = inner.store.get_setting("device_name").ok().flatten();
    Ok(Response::SelfInfo {
        value: SelfInfo {
            initialized: id.is_initialized(),
            user_id: id.user_id().map(|u| u.to_hex()),
            user_short: id.user_id().map(|u| u.short()),
            display_name: display,
            device_id: id.device_id().to_hex(),
            device_name,
            status: to_status_dto(st.status.clone()),
        },
    })
}

/// Init{FirstDevice} 完整动作：init_first_device → 覆盖用户显示名与设置 →
/// 设备行 → 更新 announce 与 HelloInfo → 更新 self_user。
async fn init(
    inner: &Inner,
    display_name: String,
    device_name: String,
    mode: InitMode,
) -> Result<Response, ApiError> {
    match mode {
        InitMode::JoinExisting => {
            // 已初始化 → AlreadyInitialized；未初始化 → 记录待接入标记。
            let initialized = inner.identity.read().await.is_initialized();
            if initialized {
                return Err(ApiError {
                    code: ErrorCode::AlreadyInitialized,
                    message: "服务已初始化".into(),
                });
            }
            inner
                .store
                .set_setting(crate::settings::SETTING_DEVICE_NAME, &device_name)
                .map_err(crate::error::CoreError::from)?;
            inner
                .store
                .set_setting(crate::settings::SETTING_JOIN_PENDING, "1")
                .map_err(crate::error::CoreError::from)?;
            return Ok(Response::Ok);
        }
        InitMode::FirstDevice => {}
    }
    let device_type = default_device_type();
    let now = now_millis();

    {
        let mut id = inner.identity.write().await;
        id.init_first_device(
            inner.keystore.as_ref(),
            &inner.store,
            &device_name,
            device_type,
            now,
        )?;
    }

    let user = inner.identity.read().await.user_id().ok_or(ApiError {
        code: ErrorCode::Internal,
        message: "初始化后缺少用户身份".into(),
    })?;

    // 覆盖用户显示名（init_first_device 先以 device_name 填充）。
    inner
        .store
        .upsert_user(&UserRow {
            user,
            display_name: display_name.clone(),
            is_self: true,
            updated_at: now,
        })
        .map_err(crate::error::CoreError::from)?;
    inner
        .store
        .set_setting("display_name", &display_name)
        .map_err(crate::error::CoreError::from)?;
    inner
        .store
        .set_setting("device_name", &device_name)
        .map_err(crate::error::CoreError::from)?;

    // 自己的设备行。
    let cert = inner.identity.read().await.cert.clone();
    if let Some(cert) = &cert {
        inner
            .store
            .upsert_device(&DeviceRow {
                device: cert.device,
                user: cert.user,
                cert: cert.clone(),
                name: device_name.clone(),
                device_type,
                last_addr: None,
                last_path: None,
                last_seen: None,
                revoked: false,
            })
            .map_err(crate::error::CoreError::from)?;
    }

    // 更新 HelloInfo 与 announce。
    inner.hello_tx.send_replace(crate::net::HelloInfo {
        display_name: display_name.clone(),
        cert: cert.clone(),
        listen_port: 0,
    });
    if let Some(ann) = build_announce(
        &inner.identity,
        &display_name,
        Status::Online,
        inner.local_addr.port(),
    )
    .await
    {
        let _ = inner.announce_tx.send(Some(ann));
    }

    // 更新 accept_filter / 路由可见的 self_user。
    inner.state.write().unwrap().self_user = Some(user);

    Ok(Response::Ok)
}

async fn set_status(inner: &Inner, status: StatusDto) -> Result<Response, ApiError> {
    let status = from_status_dto(status);
    inner.state.write().unwrap().status = status.clone();

    // 更新 announce：已初始化且非 Invisible 时为 Some。
    let initialized = inner.identity.read().await.is_initialized();
    if initialized && !matches!(status, Status::Invisible) {
        let display = inner
            .store
            .get_setting("display_name")
            .map_err(crate::error::CoreError::from)?
            .unwrap_or_default();
        if let Some(ann) = build_announce(
            &inner.identity,
            &display,
            status.clone(),
            inner.local_addr.port(),
        )
        .await
        {
            let _ = inner.announce_tx.send(Some(ann));
        }
    } else {
        let _ = inner.announce_tx.send(None);
    }

    // 向所有连接发 Presence。
    for p in inner.conns.peers() {
        let _ = inner
            .conns
            .send(&p.device, Frame::Presence(status.clone()))
            .await;
    }
    Ok(Response::Ok)
}

async fn list_peers(inner: &Inner) -> Result<Response, ApiError> {
    let self_user = inner.state.read().unwrap().self_user;
    let value = inner
        .conns
        .peers()
        .iter()
        .map(|p| peer_view(&inner.store, p, self_user.as_ref()))
        .collect();
    Ok(Response::Peers { value })
}

async fn list_conversations(inner: &Inner) -> Result<Response, ApiError> {
    let rows = inner
        .store
        .list_conversations()
        .map_err(crate::error::CoreError::from)?;
    let self_user = inner.state.read().unwrap().self_user;
    let peers = inner.conns.peers();
    let value = rows
        .into_iter()
        .map(|row| {
            let peer_online = match (&row.kind, &row.peer_user) {
                (ConvKindRow::Dm, Some(pu)) => peers
                    .iter()
                    .any(|p| p.cert.as_ref().is_some_and(|c| c.user == *pu)),
                _ => false,
            };
            conv_summary(inner.store.as_ref(), self_user.as_ref(), &row, peer_online)
        })
        .collect();
    Ok(Response::Conversations { value })
}

async fn list_messages(
    inner: &Inner,
    conv: String,
    before: Option<String>,
    limit: u32,
) -> Result<Response, ApiError> {
    let conv = parse_conv(&conv)?;
    let before = match before {
        Some(s) => Some(MsgId::from_str(&s).map_err(|_| invalid_arg(format!("非法消息 ID: {s}")))?),
        None => None,
    };
    let msgs = inner
        .store
        .list_messages(&conv, before.as_ref(), limit)
        .map_err(crate::error::CoreError::from)?;
    let self_device = inner.identity.read().await.device_id();
    let value = msgs
        .iter()
        .map(|m| message_view(inner.store.as_ref(), &self_device, m))
        .collect();
    Ok(Response::Messages { value })
}

async fn open_dm(inner: &Inner, user: String) -> Result<Response, ApiError> {
    let user = parse_user(&user)?;
    let conv = inner.msg.open_dm(&user)?;
    Ok(Response::ConvId {
        value: conv.to_hex(),
    })
}

async fn send_text(inner: &Inner, conv: String, text: String) -> Result<Response, ApiError> {
    let conv = parse_conv(&conv)?;
    let msg_id = inner.msg.send(&conv, MsgBody::Text { text }).await?;
    Ok(Response::MsgId {
        value: msg_id.to_string(),
    })
}

/// 解析 `ip:port` 或 `host:port` 并拨号。
async fn add_manual_peer(inner: &Inner, addr: String) -> Result<Response, ApiError> {
    let addr: SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(_) => tokio::net::lookup_host(&addr)
            .await
            .map_err(|_| invalid_arg(format!("无法解析地址: {addr}")))?
            .next()
            .ok_or_else(|| invalid_arg(format!("无法解析地址: {addr}")))?,
    };
    inner.conns.connect(addr, false).await?;
    Ok(Response::Ok)
}

// ---- P2 Task 5：联系人双向确认与准入过滤 ----

/// 发起联系人请求。
async fn request_contact(inner: &Inner, user: String) -> Result<Response, ApiError> {
    let user = parse_user(&user)?;
    inner.trust.request_contact(&inner.msg, &user).await?;
    Ok(Response::Ok)
}

/// 列出已接受联系人。
async fn list_contacts(inner: &Inner) -> Result<Response, ApiError> {
    let rows = inner
        .store
        .list_contacts(Some(crate::store::ContactState::Accepted))
        .map_err(crate::error::CoreError::from)?;
    let peers = inner.conns.peers();
    let self_user = inner.state.read().unwrap().self_user;
    let value = rows
        .iter()
        .map(|row| {
            crate::service::views::contact_view(&inner.store, &peers, row, self_user.as_ref())
        })
        .collect();
    Ok(Response::Contacts { value })
}

/// 列出全部 open 请求。
async fn list_requests(inner: &Inner) -> Result<Response, ApiError> {
    let rows = inner
        .store
        .list_open_requests()
        .map_err(crate::error::CoreError::from)?;
    let value = rows
        .iter()
        .map(|r| crate::service::views::request_view(&inner.store, r))
        .collect();
    Ok(Response::Requests { value })
}

/// 处理请求：按 kind 分派；Contact 走本任务，Message/File 归 Task 6。
async fn respond_request(
    inner: &Inner,
    request: String,
    action: lanmsg_api::types::RequestAction,
) -> Result<Response, ApiError> {
    let row = inner
        .store
        .get_request(&request)
        .map_err(crate::error::CoreError::from)?
        .ok_or_else(|| ApiError {
            code: ErrorCode::NotFound,
            message: format!("请求不存在: {request}"),
        })?;
    match row.kind {
        crate::store::RequestKindRow::Contact => {
            inner
                .trust
                .respond_contact(&inner.msg, &row, action)
                .await?;
        }
        // Message/File：陌生人消息门控的接收一次/加为联系人/拒绝/屏蔽。
        crate::store::RequestKindRow::Message | crate::store::RequestKindRow::File => {
            inner
                .trust
                .respond_message_request(&inner.msg, &row, action, now_millis())
                .await?;
        }
    }
    Ok(Response::Ok)
}

/// 设置联系人策略（自动同意与大文件询问阈值）。
async fn set_contact_policy(
    inner: &Inner,
    user: String,
    auto_accept: bool,
    ask_above_bytes: Option<u64>,
) -> Result<Response, ApiError> {
    let user = parse_user(&user)?;
    inner
        .trust
        .set_policy(&inner.msg, &user, auto_accept, ask_above_bytes)
        .await?;
    Ok(Response::Ok)
}

// ---- P2 Task 4：设置与资料 ----

/// GetSettings：从存储加载（无记录返回默认值）。
async fn get_settings(inner: &Inner) -> Result<Response, ApiError> {
    let value = crate::settings::load_settings(&inner.store)?;
    Ok(Response::Settings { value })
}

/// SetSettings：校验 → 保存 → 设备名变化且已初始化时重签证书，
/// 更新 HelloInfo/announce 并向已连接对端推 CertUpdate。
async fn set_settings(inner: &Inner, settings: SettingsDto) -> Result<Response, ApiError> {
    // 校验：下载目录 trim 后非空（阈值 u64 恒 ≥ 0，无需检查）。
    if settings.download_dir.trim().is_empty() {
        return Err(invalid_arg("下载目录不能为空".into()));
    }

    crate::settings::save_settings(&inner.store, &settings)?;

    // 设备名写入独立 setting。
    let old = inner
        .store
        .get_setting(crate::settings::SETTING_DEVICE_NAME)
        .ok()
        .flatten();
    inner
        .store
        .set_setting(crate::settings::SETTING_DEVICE_NAME, &settings.device_name)
        .map_err(crate::error::CoreError::from)?;

    // 设备名变化且已初始化 → 重签证书并广播。
    let renamed = old.as_deref() != Some(settings.device_name.as_str());
    let initialized = inner.identity.read().await.is_initialized();
    if renamed && initialized {
        let now = now_millis();
        let new_cert = {
            let mut id = inner.identity.write().await;
            id.reissue_cert(&inner.store, &settings.device_name, now)?
        };
        apply_cert_and_broadcast(inner, &new_cert).await;
    }

    Ok(Response::Ok)
}

/// 证书更新后的公共动作：HelloInfo 换新、announce 换新、向已连接对端推 CertUpdate。
async fn apply_cert_and_broadcast(inner: &Inner, new_cert: &lanmsg_proto::types::DeviceCert) {
    let mut hello = inner.hello_tx.borrow().clone();
    hello.cert = Some(new_cert.clone());
    inner.hello_tx.send_replace(hello);

    let display = inner
        .store
        .get_setting("display_name")
        .ok()
        .flatten()
        .unwrap_or_default();
    let status = inner.state.read().unwrap().status.clone();
    if let Some(ann) =
        build_announce(&inner.identity, &display, status, inner.local_addr.port()).await
    {
        inner.announce_tx.send_replace(Some(ann));
    }

    for p in inner.conns.peers() {
        if p.cert.is_some() {
            let _ = inner
                .conns
                .send(&p.device, Frame::CertUpdate(vec![new_cert.clone()]))
                .await;
        }
    }
}

/// SetProfile：空名 → InvalidArgument；更新 users 自己行、display_name setting、
/// HelloInfo、announce，并在 SelfDevices 会话发 ProfileUpdated 隐藏消息（同步到本人其他设备）。
async fn set_profile(inner: &Inner, display_name: String) -> Result<Response, ApiError> {
    let name = display_name.trim();
    if name.is_empty() {
        return Err(invalid_arg("显示名不能为空".into()));
    }
    let name = name.to_string();

    let self_user = inner
        .state
        .read()
        .unwrap()
        .self_user
        .ok_or_else(|| ApiError {
            code: ErrorCode::NotInitialized,
            message: "服务尚未初始化".into(),
        })?;

    // users 表自己行（is_self 保持）。
    let is_self = inner
        .store
        .get_user(&self_user)
        .ok()
        .flatten()
        .map(|r| r.is_self)
        .unwrap_or(true);
    inner
        .store
        .upsert_user(&UserRow {
            user: self_user,
            display_name: name.clone(),
            is_self,
            updated_at: now_millis(),
        })
        .map_err(crate::error::CoreError::from)?;
    inner
        .store
        .set_setting("display_name", &name)
        .map_err(crate::error::CoreError::from)?;

    // HelloInfo 换新显示名（证书与端口保持）。
    let mut hello = inner.hello_tx.borrow().clone();
    hello.display_name = name.clone();
    inner.hello_tx.send_replace(hello);

    // announce 换新显示名。
    let status = inner.state.read().unwrap().status.clone();
    if let Some(ann) = build_announce(&inner.identity, &name, status, inner.local_addr.port()).await
    {
        inner.announce_tx.send_replace(Some(ann));
    }

    // SelfDevices 会话发隐藏消息，同步到本人其他在线设备（after_ingest 应用）。
    let conv = ConvId::self_devices(&self_user);
    inner
        .msg
        .send_hidden(
            &conv,
            MsgBody::System {
                event: lanmsg_proto::types::SystemEvent::ProfileUpdated { display_name: name },
            },
        )
        .await?;

    Ok(Response::Ok)
}

// ---- 辅助 ----

fn parse_conv(s: &str) -> Result<ConvId, ApiError> {
    ConvId::from_hex(s).map_err(|_| invalid_arg(format!("非法会话 ID: {s}")))
}

fn parse_user(s: &str) -> Result<UserId, ApiError> {
    UserId::from_hex(s).map_err(|_| invalid_arg(format!("非法用户 ID: {s}")))
}

fn invalid_arg(message: String) -> ApiError {
    ApiError {
        code: ErrorCode::InvalidArgument,
        message,
    }
}

fn to_status_dto(s: Status) -> StatusDto {
    match s {
        Status::Online => StatusDto::Online,
        Status::Invisible => StatusDto::Invisible,
        Status::Busy => StatusDto::Busy,
    }
}

fn from_status_dto(s: StatusDto) -> Status {
    match s {
        StatusDto::Online => Status::Online,
        StatusDto::Invisible => Status::Invisible,
        StatusDto::Busy => Status::Busy,
    }
}

/// 编译目标为 android/ios → Phone，否则 Desktop。
fn default_device_type() -> DeviceType {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        DeviceType::Phone
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        DeviceType::Desktop
    }
}

/// 请求变体名（未实现响应用）。
fn variant_name(req: &Request) -> &'static str {
    match req {
        Request::GetSelf => "GetSelf",
        Request::Init { .. } => "Init",
        Request::SetStatus { .. } => "SetStatus",
        Request::SetProfile { .. } => "SetProfile",
        Request::ListPeers => "ListPeers",
        Request::ListConversations => "ListConversations",
        Request::ListMessages { .. } => "ListMessages",
        Request::OpenDm { .. } => "OpenDm",
        Request::SendText { .. } => "SendText",
        Request::SendFiles { .. } => "SendFiles",
        Request::SendFolder { .. } => "SendFolder",
        Request::Recall { .. } => "Recall",
        Request::MarkRead { .. } => "MarkRead",
        Request::DeleteLocal { .. } => "DeleteLocal",
        Request::DownloadBlob { .. } => "DownloadBlob",
        Request::CancelTransfer { .. } => "CancelTransfer",
        Request::ListContacts => "ListContacts",
        Request::RequestContact { .. } => "RequestContact",
        Request::ListRequests => "ListRequests",
        Request::RespondRequest { .. } => "RespondRequest",
        Request::SetContactPolicy { .. } => "SetContactPolicy",
        Request::ListMyDevices => "ListMyDevices",
        Request::RemoveDevice { .. } => "RemoveDevice",
        Request::PairStart { .. } => "PairStart",
        Request::PairJoin { .. } => "PairJoin",
        Request::PairConfirm { .. } => "PairConfirm",
        Request::PairCancel { .. } => "PairCancel",
        Request::QueryFiles { .. } => "QueryFiles",
        Request::SearchMessages { .. } => "SearchMessages",
        Request::ForwardFiles { .. } => "ForwardFiles",
        Request::GetSettings => "GetSettings",
        Request::SetSettings { .. } => "SetSettings",
        Request::AddManualPeer { .. } => "AddManualPeer",
        Request::ScanSubnet { .. } => "ScanSubnet",
        Request::ListInterfaces => "ListInterfaces",
        Request::StationCreate { .. } => "StationCreate",
        Request::StationList => "StationList",
        Request::StationPut { .. } => "StationPut",
        Request::StationSetPinned { .. } => "StationSetPinned",
        Request::StationItems { .. } => "StationItems",
        Request::StationRemove { .. } => "StationRemove",
        Request::WebStart => "WebStart",
        Request::WebStop => "WebStop",
        Request::WebRespond { .. } => "WebRespond",
        Request::SetUiFocus { .. } => "SetUiFocus",
        Request::Shutdown => "Shutdown",
    }
}
