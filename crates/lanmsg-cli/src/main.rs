// P1 核心骨架：lanmsg-cli 命令行客户端（Task 12）。
// 全部子命令经 IpcClient 调用 lanmsgd。注释中文，标识符英文。

use std::path::PathBuf;
use std::process::exit;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};

use lanmsg_api::ipc::{endpoint_name, IpcClient};
use lanmsg_api::types::{InitMode, MessageBodyView, Request, Response, StatusDto};

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
                        let name = if m.is_mine {
                            "我".to_string()
                        } else {
                            m.author_device_name.clone()
                        };
                        if let MessageBodyView::Text { text } = &m.body {
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
                    _ => break,
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
