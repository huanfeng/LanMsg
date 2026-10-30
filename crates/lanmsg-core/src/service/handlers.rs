// P1 核心骨架：请求处理器（Task 11）。
// 已实现：GetSelf、Init{FirstDevice}、SetStatus、ListPeers、ListConversations、
// ListMessages、OpenDm、SendText、AddManualPeer、Shutdown。
// 其余一律返回 `未实现：<变体名>`（唯一允许的占位，后续阶段替换）。
// 注释中文，标识符英文。

use std::net::SocketAddr;
use std::str::FromStr;

use lanmsg_api::types::{ApiError, ErrorCode, InitMode, Request, Response, SelfInfo, StatusDto};
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
        Request::AddManualPeer { addr } => add_manual_peer(inner, addr).await,
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
            return Err(ApiError {
                code: ErrorCode::Internal,
                message: "未实现：Init(JoinExisting)".into(),
            });
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
        .map(|p| peer_view(p, self_user.as_ref()))
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
