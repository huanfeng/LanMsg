// P1 核心骨架：lanmsg-cli 命令行客户端（Task 12）。
// 全部子命令经 IpcClient 调用 lanmsgd。注释中文，标识符英文。

use std::path::PathBuf;
use std::process::exit;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};

use lanmsg_api::ipc::{endpoint_name, IpcClient};
use lanmsg_api::types::{
    BlobState, Event, InitMode, MessageBodyView, PairMethod, PairStateDto, PairTarget, Request,
    RequestAction, RequestKind, Response, SettingsDto, StatusDto,
};

/// IpcClient::events() 返回的事件接收端。
type EventRx = tokio::sync::broadcast::Receiver<Event>;

#[derive(Parser)]
#[command(name = "lanmsg-cli", about = "LanMsg 命令行客户端（需先启动 lanmsgd）")]
struct Cli {
    /// 数据目录（用于计算 IPC 端点，需与 lanmsgd 一致）
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 初始化身份（首台设备）
    Init {
        /// 显示名称
        #[arg(long)]
        name: String,
        /// 设备名称，默认主机名
        #[arg(long)]
        device_name: Option<String>,
    },
    /// 查看自身身份信息
    #[command(name = "self")]
    ShowSelf,
    /// 列出在线对端设备
    Peers,
    /// 列出会话
    Convs,
    /// 打开与某用户的私聊，打印会话 ID
    Dm { user_id: String },
    /// 发送文字消息，打印消息 ID
    Send { conv: String, text: String },
    /// 打印会话消息记录
    Log {
        conv: String,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// 持续打印事件（每行一个 JSON）
    Watch,
    /// 手动添加对端地址并连接
    AddPeer { addr: String },
    /// 切换在线状态
    Status { state: String },
    /// 以新设备加入现有用户（需另一台已有设备发起 pair-start）
    Join {
        /// 设备名称
        #[arg(long)]
        device_name: String,
    },
    /// 发起「我的设备」配对，打印会话与状态直至完成/失败
    PairStart {
        /// 用 6 位数字比对方式（默认二维码 URI 方式）
        #[arg(long)]
        sas: bool,
    },
    /// 加入配对：参数以 lanmsg:// 开头视为 URI，否则视为对端设备 hex
    PairJoin { target: String },
    /// 对指定配对会话做数字比对确认（yes|no）
    PairConfirm { session: String, answer: String },
    /// 列出我的设备
    Devices,
    /// 移除一台我的设备
    RemoveDevice { device: String },
    /// 列出联系人
    Contacts,
    /// 向某用户发起联系人请求
    RequestContact { user: String },
    /// 列出待处理请求
    Requests,
    /// 处理请求：once|add|reject|block
    Respond { request: String, action: String },
    /// 向会话发送文件（可多个）
    SendFiles {
        conv: String,
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// 向会话发送文件夹
    SendFolder { conv: String, path: String },
    /// 下载文件；--wait 等待完成并打印耗时与平均速度
    Download {
        hash: String,
        #[arg(long)]
        wait: bool,
    },
    /// 撤回自己的消息
    Recall { msg: String },
    /// 标记会话已读至某消息
    Read { conv: String, msg: String },
    /// 查看设置；`settings set <key> <value>` 修改（支持 auto_image_max、
    /// auto_file_max、auto_file_wifi_only、download_dir、device_name）
    Settings { set: Option<Vec<String>> },
    /// 修改显示名
    Profile { name: String },
    /// 列出会话中的文件项（hash 名字 大小 状态）
    FilesOf { conv: String },
    /// 关闭 lanmsgd
    Shutdown,
}

/// 连接到 lanmsgd；失败时提示并退出 1。
async fn connect_or_die(data_dir: &Option<PathBuf>) -> IpcClient {
    let dir = data_dir
        .clone()
        .unwrap_or_else(lanmsg_core::paths::default_data_dir);
    let endpoint = endpoint_name(&dir);
    match IpcClient::connect(&endpoint).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("无法连接 lanmsgd（端点 {endpoint}）：{e}，请先启动 lanmsgd");
            exit(1);
        }
    }
}

/// hh:mm:ss（UTC）。
fn hhmmss(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let s = secs.rem_euclid(60);
    let m = (secs.div_euclid(60)).rem_euclid(60);
    let h = (secs.div_euclid(3600)).rem_euclid(24);
    format!("{h:02}:{m:02}:{s:02}")
}

fn status_from_str(s: &str) -> StatusDto {
    match s {
        "invisible" => StatusDto::Invisible,
        "busy" => StatusDto::Busy,
        _ => StatusDto::Online,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = connect_or_die(&cli.data_dir).await;

    match cli.cmd {
        Cmd::Init { name, device_name } => {
            let device_name =
                device_name.unwrap_or_else(|| hostname().unwrap_or_else(|| "未命名设备".into()));
            let resp = client
                .call(Request::Init {
                    display_name: name,
                    device_name,
                    mode: InitMode::FirstDevice,
                })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("已初始化"),
                other => println!("{other:?}"),
            }
        }
        Cmd::ShowSelf => {
            let resp = client.call(Request::GetSelf).await.map_err(api_err)?;
            match resp {
                Response::SelfInfo { value } => {
                    println!("initialized: {}", value.initialized);
                    println!("user_id: {}", value.user_id.as_deref().unwrap_or("-"));
                    println!("user_short: {}", value.user_short.as_deref().unwrap_or("-"));
                    println!(
                        "display_name: {}",
                        value.display_name.as_deref().unwrap_or("-")
                    );
                    println!("device_id: {}", value.device_id);
                    println!(
                        "device_name: {}",
                        value.device_name.as_deref().unwrap_or("-")
                    );
                    println!("status: {:?}", value.status);
                }
                other => println!("{other:?}"),
            }
        }
        Cmd::Peers => {
            let resp = client.call(Request::ListPeers).await.map_err(api_err)?;
            match resp {
                Response::Peers { value } => {
                    println!(
                        "{:<10} {:<16} {:<20} {:<22} 路径",
                        "设备短号", "显示名", "设备名", "地址"
                    );
                    for p in value {
                        println!(
                            "{:<10} {:<16} {:<20} {:<22} {:?}",
                            p.device_short, p.display_name, p.device_name, p.addr, p.path
                        );
                    }
                }
                other => println!("{other:?}"),
            }
        }
        Cmd::Convs => {
            let resp = client
                .call(Request::ListConversations)
                .await
                .map_err(api_err)?;
            match resp {
                Response::Conversations { value } => {
                    println!("{:<66} {:<14} {:<6} 最后消息", "会话", "标题", "未读");
                    for c in value {
                        println!(
                            "{:<66} {:<14} {:<6} {}",
                            c.conv,
                            c.title,
                            c.unread,
                            c.last_text.as_deref().unwrap_or("-")
                        );
                    }
                }
                other => println!("{other:?}"),
            }
        }
        Cmd::Dm { user_id } => {
            let resp = client
                .call(Request::OpenDm { user: user_id })
                .await
                .map_err(api_err)?;
            match resp {
                Response::ConvId { value } => println!("{value}"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Send { conv, text } => {
            let resp = client
                .call(Request::SendText { conv, text })
                .await
                .map_err(api_err)?;
            match resp {
                Response::MsgId { value } => println!("{value}"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Log { conv, limit } => {
            let self_device = match client.call(Request::GetSelf).await {
                Ok(Response::SelfInfo { value }) => value.device_id,
                _ => String::new(),
            };
            let resp = client
                .call(Request::ListMessages {
                    conv,
                    before: None,
                    limit,
                })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Messages { value } => {
                    for m in value {
                        let name = if m.is_mine && m.author_device == self_device {
                            "我".to_string()
                        } else if m.is_mine {
                            format!("我（{}）", m.author_device_name)
                        } else {
                            m.author_device_name.clone()
                        };
                        if let MessageBodyView::Text { text } | MessageBodyView::System { text } =
                            &m.body
                        {
                            println!("[{}] {}: {}", hhmmss(m.hlc_millis), name, text);
                        } else {
                            println!("[{}] {}: <不支持的消息类型>", hhmmss(m.hlc_millis), name);
                        }
                    }
                }
                other => println!("{other:?}"),
            }
        }
        Cmd::Watch => {
            let mut rx = client.events();
            loop {
                match tokio::time::timeout(Duration::from_secs(3600), rx.recv()).await {
                    Ok(Ok(e)) => println!("{}", serde_json::to_string(&e)?),
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                    // 事件流关闭 = 后台服务已断开：以非零码退出，脚本才能区分于超时正常结束
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                        eprintln!("后台服务已断开");
                        exit(WATCH_DISCONNECTED_EXIT);
                    }
                    // 一小时无事件：正常结束
                    Err(_) => break,
                }
            }
        }
        Cmd::AddPeer { addr } => {
            let resp = client
                .call(Request::AddManualPeer { addr })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("已连接"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Status { state } => {
            let resp = client
                .call(Request::SetStatus {
                    status: status_from_str(&state),
                })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("状态已切换为 {state}"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Join { device_name } => {
            let resp = client
                .call(Request::Init {
                    display_name: String::new(),
                    device_name,
                    mode: InitMode::JoinExisting,
                })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("已加入（等待配对完成）"),
                other => println!("{other:?}"),
            }
        }
        Cmd::PairStart { sas } => {
            // 先订阅事件再发请求，避免错过快速出现的状态。
            let rx = client.events();
            let resp = client
                .call(Request::PairStart {
                    method: if sas { PairMethod::Sas } else { PairMethod::Qr },
                })
                .await
                .map_err(api_err)?;
            let Response::PairSession { value } = resp else {
                println!("{resp:?}");
                return Ok(());
            };
            if let Some(uri) = &value.qr_uri {
                println!("qr_uri: {uri}");
            }
            println!("session: {}", value.session);
            watch_pair_status(rx, &value.session).await;
        }
        Cmd::PairJoin { target } => {
            let target = if target.starts_with("lanmsg://") {
                PairTarget::QrUri { uri: target }
            } else {
                PairTarget::Device { device: target }
            };
            // 先订阅事件再发请求：SAS 的 ShowSas 可能在调用返回前就已发出。
            let rx = client.events();
            let resp = client
                .call(Request::PairJoin { target })
                .await
                .map_err(api_err)?;
            if let Response::PairSession { value } = &resp {
                println!("session: {}", value.session);
            }
            println!("已发起加入，等待配对状态…");
            watch_pair_any(rx).await;
        }
        Cmd::PairConfirm { session, answer } => {
            let matches = match answer.as_str() {
                "yes" => true,
                "no" => false,
                other => {
                    eprintln!("答案应为 yes|no，实际 {other}");
                    exit(1);
                }
            };
            let resp = client
                .call(Request::PairConfirm { session, matches })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("已提交确认"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Devices => {
            let resp = client.call(Request::ListMyDevices).await.map_err(api_err)?;
            match resp {
                Response::Devices { value } => {
                    println!(
                        "{:<10} {:<20} {:<6} {:<6} 设备ID",
                        "短号", "设备名", "在线", "本机"
                    );
                    for d in value {
                        println!(
                            "{:<10} {:<20} {:<6} {:<6} {}",
                            d.device_short,
                            d.name,
                            if d.online { "在线" } else { "离线" },
                            if d.is_self { "是" } else { "否" },
                            d.device_id,
                        );
                    }
                }
                other => println!("{other:?}"),
            }
        }
        Cmd::RemoveDevice { device } => {
            let resp = client
                .call(Request::RemoveDevice { device })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("已移除"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Contacts => {
            let resp = client.call(Request::ListContacts).await.map_err(api_err)?;
            match resp {
                Response::Contacts { value } => {
                    println!(
                        "{:<12} {:<16} {:<6} {:<14} 用户ID",
                        "显示名", "短号", "在线", "下载上限"
                    );
                    for c in value {
                        println!(
                            "{:<12} {:<16} {:<6} {:<14} {}",
                            c.display_name,
                            c.user_short,
                            if c.online { "在线" } else { "离线" },
                            match c.ask_above_bytes {
                                Some(n) => human_size(n),
                                None => "不限".into(),
                            },
                            c.user_id,
                        );
                    }
                }
                other => println!("{other:?}"),
            }
        }
        Cmd::RequestContact { user } => {
            let resp = client
                .call(Request::RequestContact { user })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("请求已发送"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Requests => {
            let resp = client.call(Request::ListRequests).await.map_err(api_err)?;
            match resp {
                Response::Requests { value } => {
                    if value.is_empty() {
                        println!("（无待处理请求）");
                    }
                    for r in value {
                        println!(
                            "{} {} {} {}",
                            r.request,
                            kind_str(&r.kind),
                            r.from_name,
                            r.preview
                        );
                    }
                }
                other => println!("{other:?}"),
            }
        }
        Cmd::Respond { request, action } => {
            let action = match action.as_str() {
                "once" => RequestAction::AcceptOnce,
                "add" => RequestAction::AddContact,
                "reject" => RequestAction::Reject,
                "block" => RequestAction::Block,
                other => {
                    eprintln!("动作应为 once|add|reject|block，实际 {other}");
                    exit(1);
                }
            };
            let resp = client
                .call(Request::RespondRequest { request, action })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("已处理"),
                other => println!("{other:?}"),
            }
        }
        Cmd::SendFiles { conv, paths } => {
            let resp = client
                .call(Request::SendFiles { conv, paths })
                .await
                .map_err(api_err)?;
            match resp {
                Response::MsgId { value } => println!("已开始发送 {value}（后台准备与传输）"),
                other => println!("{other:?}"),
            }
        }
        Cmd::SendFolder { conv, path } => {
            let resp = client
                .call(Request::SendFolder { conv, path })
                .await
                .map_err(api_err)?;
            match resp {
                Response::MsgId { value } => println!("已开始发送 {value}（后台准备与传输）"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Download { hash, wait } => {
            if !wait {
                let resp = client
                    .call(Request::DownloadBlob { hash })
                    .await
                    .map_err(api_err)?;
                match resp {
                    Response::TransferId { value } => println!("transfer: {value}"),
                    other => println!("{other:?}"),
                }
            } else {
                download_and_wait(&client, hash).await;
            }
        }
        Cmd::Recall { msg } => {
            let resp = client
                .call(Request::Recall { msg })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("已撤回"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Read { conv, msg } => {
            let resp = client
                .call(Request::MarkRead { conv, upto: msg })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("已标记已读"),
                other => println!("{other:?}"),
            }
        }
        Cmd::Settings { set } => match set {
            None => {
                let resp = client.call(Request::GetSettings).await.map_err(api_err)?;
                match resp {
                    Response::Settings { value } => print_settings(&value),
                    other => println!("{other:?}"),
                }
            }
            Some(kv) => settings_set(&client, kv).await?,
        },
        Cmd::Profile { name } => {
            let resp = client
                .call(Request::SetProfile { display_name: name })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Ok => println!("资料已更新"),
                other => println!("{other:?}"),
            }
        }
        Cmd::FilesOf { conv } => {
            let resp = client
                .call(Request::ListMessages {
                    conv,
                    before: None,
                    limit: 1000,
                })
                .await
                .map_err(api_err)?;
            match resp {
                Response::Messages { value } => {
                    for m in value {
                        let items: Vec<_> = match &m.body {
                            MessageBodyView::Files { items } => items.clone(),
                            MessageBodyView::Folder { items, .. } => items.clone(),
                            _ => continue,
                        };
                        for it in items {
                            println!(
                                "{} {} {} {}",
                                it.hash,
                                it.name,
                                it.size,
                                blob_state_str(&it.state)
                            );
                        }
                    }
                }
                other => println!("{other:?}"),
            }
        }
        Cmd::Shutdown => {
            let resp = client.call(Request::Shutdown).await.map_err(api_err)?;
            match resp {
                Response::Ok => println!("lanmsgd 已退出"),
                other => println!("{other:?}"),
            }
        }
    }
    Ok(())
}

/// API 错误转 anyhow。
fn api_err(e: lanmsg_api::types::ApiError) -> anyhow::Error {
    anyhow::anyhow!("API 错误 {:?}: {}", e.code, e.message)
}

/// 主机名（Init 的 device_name 默认值）。
fn hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}

/// 字节数转人类可读大小。
fn human_size(n: u64) -> String {
    let kb = n as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{kb:.0} KB");
    }
    let mb = kb / 1024.0;
    if mb < 1024.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.1} GB", mb / 1024.0)
    }
}

/// 请求类型中文。
fn kind_str(k: &RequestKind) -> &'static str {
    match k {
        RequestKind::Contact => "联系人",
        RequestKind::Message => "消息",
        RequestKind::File => "文件",
    }
}

/// Blob 状态中文。
fn blob_state_str(s: &BlobState) -> &'static str {
    match s {
        BlobState::Remote => "远端",
        BlobState::Downloading => "下载中",
        BlobState::Local => "本地",
        BlobState::SourceOffline => "源离线",
        BlobState::Preparing => "准备中",
    }
}

/// `watch` 因后台服务断开（事件流关闭）而退出的退出码；超时正常结束为 0。
const WATCH_DISCONNECTED_EXIT: i32 = 2;

/// 打印设置。
fn print_settings(s: &SettingsDto) {
    println!("device_name: {}", s.device_name);
    println!("download_dir: {}", s.download_dir);
    println!("auto_image_max: {}", s.auto_image_max);
    println!("auto_file_max: {}", s.auto_file_max);
    println!("auto_file_wifi_only: {}", s.auto_file_wifi_only);
    println!("localsend_enabled: {}", s.localsend_enabled);
    println!("background_mode: {:?}", s.background_mode);
}

/// settings set <key> <value>：读改写。value 按原类型解析（数字/布尔/字符串）。
async fn settings_set(client: &IpcClient, mut kv: Vec<String>) -> anyhow::Result<()> {
    // clap 把 "set" 一并收进位置参数，先剥掉。
    if kv.first().map(String::as_str) == Some("set") {
        kv.remove(0);
    }
    if kv.len() != 2 {
        eprintln!("用法：settings set <key> <value>（key 支持 auto_image_max、auto_file_max、auto_file_wifi_only、download_dir、device_name）");
        exit(1);
    }
    let (key, value) = (kv[0].clone(), kv[1].clone());
    let resp = client.call(Request::GetSettings).await.map_err(api_err)?;
    let Response::Settings { value: mut s } = resp else {
        anyhow::bail!("期望 Settings 响应");
    };
    match key.as_str() {
        "auto_image_max" => s.auto_image_max = value.parse()?,
        "auto_file_max" => s.auto_file_max = value.parse()?,
        "auto_file_wifi_only" => s.auto_file_wifi_only = value.parse()?,
        "download_dir" => s.download_dir = value,
        "device_name" => s.device_name = value,
        other => {
            eprintln!("不支持的设置项: {other}");
            exit(1);
        }
    }
    client
        .call(Request::SetSettings { settings: s })
        .await
        .map_err(api_err)?;
    println!("已更新 {key}");
    Ok(())
}

/// 持续打印指定会话的 PairStatus 直至 Done/Failed。
async fn watch_pair_status(mut rx: EventRx, session: &str) {
    loop {
        match rx.recv().await {
            Ok(Event::PairStatus { session: s, state }) if s == session => {
                if print_pair_state(&state) {
                    return;
                }
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(_) => return,
        }
    }
}

/// 打印任意会话的 PairStatus 直至 Done/Failed（不按 session 过滤；
/// 加入方同一时刻只有一个配对在进行，PairJoin 用它等待结果）。
async fn watch_pair_any(mut rx: EventRx) {
    loop {
        match rx.recv().await {
            Ok(Event::PairStatus { state, .. }) => {
                if print_pair_state(&state) {
                    return;
                }
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(_) => return,
        }
    }
}

/// 打印一条配对状态；返回 true 表示已结束（Done/Failed）。
fn print_pair_state(state: &PairStateDto) -> bool {
    match state {
        PairStateDto::WaitingPeer => println!("等待对端加入…"),
        PairStateDto::ShowSas { code } => println!("比对数字：{code}"),
        PairStateDto::Verifying => println!("校验中…"),
        PairStateDto::Done => {
            println!("配对完成");
            return true;
        }
        PairStateDto::Failed { reason } => {
            println!("配对失败：{reason}");
            return true;
        }
    }
    false
}

/// 发起下载并等待完成，打印耗时与平均速度（按 计划格式 耗时 X.XXs 平均 X.X MB/s）。
async fn download_and_wait(client: &IpcClient, hash: String) {
    // 先订阅事件流再发起请求，避免快速完成的事件被错过。
    let mut rx = client.events();
    let t0 = std::time::Instant::now();
    let resp = match client.call(Request::DownloadBlob { hash }).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("API 错误 {:?}: {}", e.code, e.message);
            exit(1);
        }
    };
    let Response::TransferId { value: transfer } = resp else {
        println!("{resp:?}");
        return;
    };
    let mut total = 0u64;
    loop {
        match rx.recv().await {
            Ok(Event::TransferFinished {
                transfer: t,
                ok,
                error,
                ..
            }) if t == transfer => {
                if !ok {
                    eprintln!("下载失败：{}", error.as_deref().unwrap_or("未知错误"));
                    exit(1);
                }
                break;
            }
            Ok(Event::TransferProgress { total: n, .. }) => {
                total = n;
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(_) => break,
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    if total > 0 {
        let mbps = total as f64 / 1024.0 / 1024.0 / secs;
        println!("耗时 {secs:.2}s 平均 {mbps:.1} MB/s");
    } else {
        println!("耗时 {secs:.2}s");
    }
}
