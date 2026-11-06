// IPC 桥：后台线程里的 IpcClient。
// UI 线程经 `Bridge::call` 投递请求，结果与事件经 windui 通道（`UiMsg`）回到 UI 线程。
// 连接失败时拉起 lanmsgd 一次，之后每秒重试；任何阻塞都发生在后台线程。
// 断线只靠 `IpcClient::events()` 的 Closed 判定（读任务退出即关闭），不对单个调用的 Io 错误做断线推断。
// 注释中文，标识符英文。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use lanmsg_api::ipc::IpcClient;
use lanmsg_api::types::{ApiError, ErrorCode, Event, Request, Response};
use tokio::sync::{broadcast, mpsc};
use windui::prelude::Sender;

/// 重连间隔。
const RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// 发往 UI 线程的消息。
#[derive(Debug, Clone)]
pub enum UiMsg {
    Connected,
    Disconnected(String),
    /// 事件流丢失过消息（Lagged），需要全量刷新。
    Resync,
    Reply {
        tag: ReplyTag,
        result: Result<Response, ApiError>,
    },
    Event(Event),
}

/// 请求回复的用途标签：Model 据此决定把回复写到哪里。
#[derive(Debug, Clone, PartialEq)]
pub enum ReplyTag {
    SelfInfo,
    Conversations,
    Messages {
        conv: String,
        older: bool,
    },
    Peers,
    Contacts,
    Requests,
    Devices,
    Files {
        append: bool,
    },
    Settings,
    Init,
    Pair,
    /// `draft` 是发送文字时的原文：失败时用来回填输入框。
    Sent {
        conv: String,
        draft: Option<String>,
    },
    Transfer {
        hash: String,
    },
    Deleted {
        msg: String,
    },
    Search {
        query: String,
    },
    Forwarded,
    Generic,
}

/// 界面侧的桥句柄（可克隆，随处投递请求）。
#[derive(Clone)]
pub struct Bridge {
    tx: Option<mpsc::UnboundedSender<(Request, ReplyTag)>>,
}

impl Bridge {
    /// 启动后台线程并开始连接。
    pub fn start(endpoint: String, data_dir: PathBuf, ui: Sender<UiMsg>) -> Bridge {
        let (tx, rx) = mpsc::unbounded_channel();
        let spawned = std::thread::Builder::new()
            .name("lanmsg-bridge".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!("桥线程创建运行时失败: {e}");
                        let _ = ui.send(UiMsg::Disconnected(e.to_string()));
                        return;
                    }
                };
                rt.block_on(run(endpoint, data_dir, ui, rx));
            });
        if let Err(e) = spawned {
            tracing::error!("启动桥线程失败: {e}");
        }
        Bridge { tx: Some(tx) }
    }

    /// 演示/测试用：丢弃全部调用。
    pub fn detached() -> Bridge {
        Bridge { tx: None }
    }

    /// 投递请求；回复以 `UiMsg::Reply` 带着 `tag` 回到 UI 线程。
    pub fn call(&self, req: Request, tag: ReplyTag) {
        if let Some(tx) = &self.tx {
            // 桥线程已退出时发送失败，等同未连接，直接忽略
            let _ = tx.send((req, tag));
        }
    }
}

fn not_connected() -> ApiError {
    ApiError {
        code: ErrorCode::Io,
        message: crate::strings::NOT_CONNECTED.to_string(),
    }
}

/// 桥主循环：连接 → 服务 → 断开 → 重试。
async fn run(
    endpoint: String,
    data_dir: PathBuf,
    ui: Sender<UiMsg>,
    mut rx: mpsc::UnboundedReceiver<(Request, ReplyTag)>,
) {
    // 本轮断线周期内是否已拉起过 lanmsgd / 已通知界面
    let mut launched = false;
    let mut reported = false;
    loop {
        match IpcClient::connect(&endpoint).await {
            Ok(client) => {
                launched = false;
                if ui.send(UiMsg::Connected).is_err() {
                    return;
                }
                let reason = serve(&client, &mut rx, &ui).await;
                if ui.send(UiMsg::Disconnected(reason)).is_err() {
                    return;
                }
                reported = true;
            }
            Err(e) => {
                if !launched {
                    launched = true;
                    if let Err(le) = spawn_daemon(&data_dir) {
                        tracing::warn!("拉起 lanmsgd 失败: {le}");
                    }
                }
                if !reported {
                    reported = true;
                    if ui.send(UiMsg::Disconnected(e.to_string())).is_err() {
                        return;
                    }
                }
            }
        }
        // 断开期间的请求不排队：立刻以“未连接”回复，避免重连后重放过期操作
        let retry = tokio::time::sleep(RETRY_INTERVAL);
        tokio::pin!(retry);
        loop {
            tokio::select! {
                _ = &mut retry => break,
                cmd = rx.recv() => {
                    let Some((_, tag)) = cmd else { return };
                    if ui.send(UiMsg::Reply { tag, result: Err(not_connected()) }).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

/// 在已连接状态下处理请求与事件，返回断开原因。
async fn serve(
    client: &IpcClient,
    rx: &mut mpsc::UnboundedReceiver<(Request, ReplyTag)>,
    ui: &Sender<UiMsg>,
) -> String {
    let mut events = client.events();
    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some((req, tag)) = cmd else {
                    return "界面已退出".to_string();
                };
                let client = client.clone();
                let ui = ui.clone();
                tokio::spawn(async move {
                    let result = client.call(req).await;
                    let _ = ui.send(UiMsg::Reply { tag, result });
                });
            }
            ev = events.recv() => match ev {
                Ok(e) => {
                    if ui.send(UiMsg::Event(e)).is_err() {
                        return "界面已退出".to_string();
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if ui.send(UiMsg::Resync).is_err() {
                        return "界面已退出".to_string();
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return "事件流已关闭".to_string();
                }
            },
        }
    }
}

/// 拉起同目录的 lanmsgd（分离进程，`--data-dir <dir>`）。
fn spawn_daemon(data_dir: &Path) -> std::io::Result<()> {
    let name = if cfg!(windows) {
        "lanmsgd.exe"
    } else {
        "lanmsgd"
    };
    let exe = std::env::current_exe()?
        .parent()
        .map(|d| d.join(name))
        .unwrap_or_else(|| PathBuf::from(name));
    let mut cmd = Command::new(exe);
    cmd.arg("--data-dir")
        .arg(data_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组，界面退出或终端关闭不连带杀掉守护进程
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()?;
    // 后台回收，避免 Unix 下留僵尸进程
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
