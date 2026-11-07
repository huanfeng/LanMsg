// IPC 桥：后台线程里的 IpcClient。
// UI 线程经 `Bridge::call` 投递请求，结果与事件经 windui 通道（`UiMsg`）回到 UI 线程。
// 连接失败时拉起 lanmsgd 一次（数据目录有“主动退出”标记则不拉起），之后每秒重试；任何阻塞都发生在后台线程。
// 断线只靠 `IpcClient::events()` 的 Closed 判定（读任务退出即关闭），不对单个调用的 Io 错误做断线推断。
// 注释中文，标识符英文。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc as std_mpsc;
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
    /// 后台服务是用户主动退出的（数据目录里有退出标记），不自动拉起。
    DaemonQuit,
    /// 拉起 lanmsgd 失败；内容是可直接展示给用户的原因。
    LaunchFailed(String),
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
    /// 文件页查询；`epoch` 是发出时 `Model.files_epoch` 的值，条件变了之后迟到的回复据此丢弃。
    Files {
        append: bool,
        epoch: u64,
    },
    /// 文件页静默刷新的一批：已加载条数超过单次上限时分多批取回，凑够 `target` 条再整体替换；
    /// `asked` 是本批请求的条数（回复满批才说明后面还有）。
    FilesRefresh {
        epoch: u64,
        target: u32,
        asked: u32,
    },
    Settings,
    /// `SetSettings` 的回复：成功后提示并重新读取。
    SettingsSaved,
    /// `SetProfile` 的回复：成功后刷新本人信息。
    ProfileSaved,
    Init,
    /// 发起方的 `PairStart`；`nonce` 是发出时 `Model.pair_nonce` 的值，用于丢弃取消后迟到的回复。
    Pair {
        nonce: u64,
    },
    /// 加入方的 `PairJoin`：失败时回到选择界面（发起方的 `Pair` 失败则关闭对话框）。
    PairJoin {
        nonce: u64,
    },
    /// `OpenDm` 的回复：成功后切到会话页并打开该会话。
    Dm,
    /// 请求处理（接收/拒绝/屏蔽…）：成功后才从列表移除。
    Responded {
        request: String,
    },
    /// 发出联系人请求：成功后提示。
    Requested {
        user: String,
    },
    /// 联系人策略保存：失败后以服务端为准重新拉取。
    Policy,
    /// 移除本人设备：成功后刷新设备列表。
    DeviceRemoved,
    /// 手动添加对端地址。
    ManualPeer,
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

/// 界面发给桥线程的指令。
enum Cmd {
    /// 普通请求，回复经 `UiMsg::Reply` 回到界面。
    Call(Request, ReplyTag),
    /// 同步请求：处理完（或确认当前未连接）后向 `ack` 发一个信号，供退出路径限时等待。
    Sync(Request, std_mpsc::Sender<()>),
    /// 用户点“启动后台服务”：清除退出标记并拉起 lanmsgd。
    StartDaemon,
}

/// 界面侧的桥句柄（可克隆，随处投递请求）。
#[derive(Clone)]
pub struct Bridge {
    tx: Option<mpsc::UnboundedSender<Cmd>>,
}

impl Bridge {
    /// 启动后台线程并开始连接。
    pub fn start(endpoint: String, data_dir: PathBuf, ui: Sender<UiMsg>) -> Bridge {
        let (tx, rx) = mpsc::unbounded_channel::<Cmd>();
        let spawned = std::thread::Builder::new()
            .name("lanmsg-bridge".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!("bridge thread failed to create runtime: {e}");
                        let _ = ui.send(UiMsg::Disconnected(e.to_string()));
                        return;
                    }
                };
                rt.block_on(run(endpoint, data_dir, ui, rx));
            });
        if let Err(e) = spawned {
            tracing::error!("failed to start bridge thread: {e}");
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
            let _ = tx.send(Cmd::Call(req, tag));
        }
    }

    /// 同步投递请求并最多等 `timeout`（界面退出路径用）；返回是否在限时内完成。
    pub fn call_sync(&self, req: Request, timeout: Duration) -> bool {
        let Some(tx) = &self.tx else { return false };
        let (ack_tx, ack_rx) = std_mpsc::channel();
        if tx.send(Cmd::Sync(req, ack_tx)).is_err() {
            return false;
        }
        // 桥线程丢弃 ack 发送端（未连接）时 recv_timeout 立即返回 Err，同样不再等待
        ack_rx.recv_timeout(timeout).is_ok()
    }

    /// 清除退出标记并拉起后台服务（“启动后台服务”按钮）。
    pub fn start_daemon(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Cmd::StartDaemon);
        }
    }
}

/// 连接失败后下一步做什么。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FailAction {
    /// 拉起 lanmsgd。
    Launch,
    /// 后台服务是主动退出的：不拉起，提示用户手动启动。
    ShowQuit,
    /// 什么也不做，等待下次重试。
    Wait,
}

/// 纯判定：连接失败时，依据“有无退出标记”和“本轮是否已拉起过”决定动作。
pub fn connect_failure_action(quit_marker: bool, launched: bool) -> FailAction {
    if quit_marker {
        FailAction::ShowQuit
    } else if launched {
        FailAction::Wait
    } else {
        FailAction::Launch
    }
}

/// 拉起失败时展示给用户的原因。
pub fn launch_failure_text(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        crate::strings::LAUNCH_NOT_FOUND.to_string()
    } else {
        format!("{}{e}", crate::strings::LAUNCH_FAILED_PREFIX)
    }
}

fn not_connected() -> ApiError {
    ApiError {
        code: ErrorCode::Io,
        message: "not connected to lanmsgd".to_string(),
    }
}

/// 桥主循环：连接 → 服务 → 断开 → 重试。
async fn run(
    endpoint: String,
    data_dir: PathBuf,
    ui: Sender<UiMsg>,
    mut rx: mpsc::UnboundedReceiver<Cmd>,
) {
    // 本轮断线周期内是否已拉起过 lanmsgd / 已通知界面 / 已提示“后台服务已退出”
    let mut launched = false;
    let mut reported = false;
    let mut quit_shown = false;
    loop {
        match IpcClient::connect(&endpoint).await {
            Ok(client) => {
                launched = false;
                quit_shown = false;
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
                let marker = data_dir
                    .join(lanmsg_proto::consts::DAEMON_QUIT_MARKER)
                    .exists();
                match connect_failure_action(marker, launched) {
                    FailAction::Launch => {
                        launched = true;
                        if quit_shown {
                            // 标记已被清除：离开“已退出”提示，回到普通重连
                            quit_shown = false;
                            reported = false;
                        }
                        if let Err(le) = spawn_daemon(&data_dir) {
                            tracing::warn!("failed to launch lanmsgd: {le}");
                            reported = true;
                            if ui
                                .send(UiMsg::LaunchFailed(launch_failure_text(&le)))
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    FailAction::ShowQuit => {
                        if !quit_shown {
                            quit_shown = true;
                            reported = true;
                            if ui.send(UiMsg::DaemonQuit).is_err() {
                                return;
                            }
                        }
                    }
                    FailAction::Wait => {}
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
                cmd = rx.recv() => match cmd {
                    None => return,
                    Some(Cmd::Call(_, tag)) => {
                        if ui.send(UiMsg::Reply { tag, result: Err(not_connected()) }).is_err() {
                            return;
                        }
                    }
                    // 未连接：丢弃 ack 发送端，等待方立即返回
                    Some(Cmd::Sync(..)) => {}
                    Some(Cmd::StartDaemon) => {
                        let _ = std::fs::remove_file(data_dir.join(lanmsg_proto::consts::DAEMON_QUIT_MARKER));
                        launched = true;
                        quit_shown = false;
                        let msg = match spawn_daemon(&data_dir) {
                            Ok(()) => UiMsg::Disconnected(crate::strings::DAEMON_STARTING.to_string()),
                            Err(le) => {
                                tracing::warn!("failed to launch lanmsgd: {le}");
                                UiMsg::LaunchFailed(launch_failure_text(&le))
                            }
                        };
                        reported = true;
                        if ui.send(msg).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// 在已连接状态下处理请求与事件，返回断开原因。
async fn serve(
    client: &IpcClient,
    rx: &mut mpsc::UnboundedReceiver<Cmd>,
    ui: &Sender<UiMsg>,
) -> String {
    let mut events = client.events();
    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else {
                    return "UI exited".to_string();
                };
                let client = client.clone();
                let ui = ui.clone();
                match cmd {
                    Cmd::Call(req, tag) => {
                        tokio::spawn(async move {
                            let result = client.call(req).await;
                            let _ = ui.send(UiMsg::Reply { tag, result });
                        });
                    }
                    Cmd::Sync(req, ack) => {
                        tokio::spawn(async move {
                            let _ = client.call(req).await;
                            let _ = ack.send(());
                        });
                    }
                    // 已连接，后台服务在运行，无需处理
                    Cmd::StartDaemon => {}
                }
            }
            ev = events.recv() => match ev {
                Ok(e) => {
                    if ui.send(UiMsg::Event(e)).is_err() {
                        return "UI exited".to_string();
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if ui.send(UiMsg::Resync).is_err() {
                        return "UI exited".to_string();
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return "event stream closed".to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_marker_blocks_launch() {
        assert_eq!(connect_failure_action(true, false), FailAction::ShowQuit);
        assert_eq!(connect_failure_action(true, true), FailAction::ShowQuit);
    }

    #[test]
    fn launches_once_without_marker() {
        assert_eq!(connect_failure_action(false, false), FailAction::Launch);
        assert_eq!(connect_failure_action(false, true), FailAction::Wait);
    }

    #[test]
    fn launch_failure_text_names_the_cause() {
        let nf = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(launch_failure_text(&nf), crate::strings::LAUNCH_NOT_FOUND);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(launch_failure_text(&denied).starts_with(crate::strings::LAUNCH_FAILED_PREFIX));
    }

    #[test]
    fn call_sync_on_detached_bridge_returns_immediately() {
        let b = Bridge::detached();
        assert!(!b.call_sync(Request::GetSelf, Duration::from_secs(5)));
    }
}
