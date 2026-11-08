// lanmsgd 进程运行时（P3 Task 8）：线程模型、无头判定、缩略图缓存清理。
// 线程模型：tokio 多线程运行时固定 2 个 worker（控制常驻内存），由本模块持有；
// 主线程留给托盘/界面消息循环（Task 9），无头时阻塞等待退出信号。
// 注释中文，标识符英文。

use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use lanmsg_api::ipc::{endpoint_name, IpcListener};
use lanmsg_core::discovery::DiscoveryMode;
use lanmsg_core::keystore_os::os_keystore;
use lanmsg_core::thumb_cache::enforce_thumb_cache_limit;
use lanmsg_core::{PlatformHooks, Service, ServiceConfig};
use lanmsg_proto::consts::{DAEMON_QUIT_MARKER, LOCALSEND_PORT, THUMB_CACHE_LIMIT};

use crate::thumbnailer::SubprocessThumbnailer;
use crate::{ipc_server, Args};

/// 缩略图缓存清理间隔。
const THUMB_CACHE_SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

/// IPC 端点已被另一个 lanmsgd 占用。
#[derive(Debug)]
pub struct AlreadyRunning;

impl std::fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lanmsgd is already running")
    }
}

impl std::error::Error for AlreadyRunning {}

/// 守护进程运行时：Service + 持有它的 tokio 运行时。
pub struct DaemonRuntime {
    pub service: Service,
    rt: tokio::runtime::Runtime,
    /// 退出通知：IPC `Shutdown`、信号或托盘退出时置 true。
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// IPC 接入循环任务；退出时等它结束以便监听器 Drop 清理 socket。
    accept_task: tokio::task::JoinHandle<()>,
}

/// 写“主动退出”标记（托盘“退出”或 IPC Shutdown 路径调用）：界面进程见到它就不再自动拉起后台。
/// SIGTERM/kill 路径刻意不写：那是系统关机等场景，下次开机自启不应被标记拦住。
pub(crate) fn write_quit_marker(data_dir: &Path) {
    if let Err(e) = std::fs::write(data_dir.join(DAEMON_QUIT_MARKER), b"") {
        tracing::warn!("failed to write quit marker: {e}");
    }
}

/// 建 tokio 运行时、启动 Service 与 IPC 服务；返回时 IPC 已可用。
/// IPC 端点被占用时返回 `AlreadyRunning`；系统密钥库不可用等错误带上下文返回。
pub fn start_runtime(args: &Args) -> Result<DaemonRuntime> {
    let data_dir = args.resolved_data_dir();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("failed to create tokio runtime")?;

    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let (service, accept_task) = rt.block_on(async {
        // 信号处理器最先安装（早于 IPC 绑定），并由常驻任务统一转成退出通知，
        // 无头与托盘两种模式共用：收到信号后走正常收尾（清理 IPC socket）。
        let signals = SignalSet::register();
        let sig_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            signals.wait().await;
            tracing::info!("termination signal received, shutting down");
            sig_tx.send_replace(true);
        });

        // 先占 IPC 端点：已有实例在运行时尽早退出，不去动数据库与端口。
        let endpoint = endpoint_name(&data_dir);
        let listener = match IpcListener::bind(&endpoint) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                return Err(anyhow::Error::new(AlreadyRunning))
            }
            Err(e) => return Err(anyhow::Error::new(e).context("failed to bind IPC endpoint")),
        };

        // IPC 已绑定即视为启动成功：删除上次主动退出留下的标记。
        let _ = std::fs::remove_file(data_dir.join(DAEMON_QUIT_MARKER));

        let cache_dir = data_dir.join("cache");
        let service = Service::start(ServiceConfig {
            data_dir: data_dir.clone(),
            listen_port: args.port,
            discovery: if args.no_multicast {
                DiscoveryMode::Disabled
            } else {
                DiscoveryMode::Multicast
            },
            keystore: os_keystore(&data_dir),
            static_peers: args.peers.clone(),
            platform: PlatformHooks {
                thumbnailer: Some(Arc::new(SubprocessThumbnailer::new(cache_dir.clone()))),
                ..PlatformHooks::default()
            },
            localsend_port: LOCALSEND_PORT,
        })
        .await
        .context("failed to start service (if the data directory marker selects the system keystore but it is unavailable, check the keystore service)")?;
        tracing::info!("lanmsgd started, IPC endpoint: {endpoint}");

        // IPC 接入循环。
        let svc = service.clone();
        let tx = shutdown_tx.clone();
        let mut stop = shutdown_tx.subscribe();
        let focus_owner = Arc::new(AtomicU64::new(0));
        let accept_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = async { let _ = stop.wait_for(|v| *v).await; } => break,
                    res = listener.accept() => match res {
                        Ok(stream) => {
                            tokio::spawn(ipc_server::serve_connection(
                                svc.clone(),
                                stream,
                                tx.clone(),
                                data_dir.clone(),
                                focus_owner.clone(),
                            ));
                        }
                        Err(e) => {
                            tracing::warn!("IPC accept failed: {e}");
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    },
                }
            }
            // listener 在此 Drop，Unix 下清理 socket 文件。
        });

        // 缩略图缓存清理：启动一次，之后每小时一次。
        let thumbs_dir = cache_dir.join("thumbs");
        tokio::spawn(async move {
            loop {
                let dir = thumbs_dir.clone();
                match tokio::task::spawn_blocking(move || {
                    enforce_thumb_cache_limit(&dir, THUMB_CACHE_LIMIT)
                })
                .await
                {
                    Ok(Ok(freed)) if freed > 0 => tracing::info!("thumbnail cache cleaned, {freed} bytes freed"),
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => tracing::warn!("thumbnail cache cleanup failed: {e}"),
                    Err(e) => tracing::warn!("thumbnail cache cleanup task failed: {e}"),
                }
                tokio::time::sleep(THUMB_CACHE_SWEEP_INTERVAL).await;
            }
        });

        Ok((service, accept_task))
    })?;

    Ok(DaemonRuntime {
        service,
        rt,
        shutdown_tx,
        accept_task,
    })
}

impl DaemonRuntime {
    /// 阻塞当前线程直到收到 Ctrl-C / SIGTERM（Unix）/ `Request::Shutdown`（信号已由
    /// `start_runtime` 注册的任务转成退出通知），然后关闭服务。
    /// 返回后 IPC 监听器已 Drop（socket 文件被清理）。
    pub fn block_until_shutdown(self) {
        let service = self.service.clone();
        let tx = self.shutdown_tx.clone();
        let mut rx = self.shutdown_tx.subscribe();
        self.rt.block_on(async move {
            tokio::select! {
                _ = async { let _ = rx.wait_for(|v| *v).await; } => {}
            }
            tx.send_replace(true);
            service.shutdown().await;
        });
        self.finish();
    }

    /// tokio 运行时句柄：界面线程用 `handle.block_on` 调用 `Service::handle`。
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.rt.handle().clone()
    }

    /// 退出通知触发时在运行时线程上执行一次 `f`（托盘据此经 channel 回界面线程退出）。
    pub fn on_shutdown(&self, f: impl FnOnce() + Send + 'static) {
        let mut rx = self.shutdown_tx.subscribe();
        self.rt.spawn(async move {
            let _ = rx.wait_for(|v| *v).await;
            f();
        });
    }

    /// 把核心事件转发给回调（在运行时线程上调用 `f`，回调需自行跨线程投递到界面线程）。
    pub fn spawn_event_forwarder(&self, f: impl Fn(lanmsg_api::Event) + Send + 'static) {
        let mut events = self.service.subscribe();
        self.rt.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(e) => f(e),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    /// 关闭服务并停止运行时（托盘退出后使用）。
    pub fn shutdown(self) {
        let service = self.service.clone();
        self.shutdown_tx.send_replace(true);
        self.rt.block_on(async move { service.shutdown().await });
        self.finish();
    }

    /// 等 IPC 接入循环结束（监听器 Drop），再停运行时。
    fn finish(self) {
        let _ = self.rt.block_on(self.accept_task);
        self.rt.shutdown_background();
    }
}

/// 终止信号的监听器：Unix 为 SIGTERM + SIGINT，其他平台在等待时才监听 Ctrl-C。
/// 必须在绑定 IPC 之前创建：IPC 一开始接受连接，外部就可能立刻发信号，
/// 处理器晚于此安装的话，信号会落在默认动作上（进程被杀、socket 文件不清理）。
struct SignalSet {
    #[cfg(unix)]
    term: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    int: Option<tokio::signal::unix::Signal>,
}

impl SignalSet {
    /// 注册信号处理器（需在 tokio 运行时上下文中调用）。
    fn register() -> SignalSet {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let reg = |kind: SignalKind, name: &str| match signal(kind) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!("failed to listen for {name}: {e}");
                    None
                }
            };
            SignalSet {
                term: reg(SignalKind::terminate(), "SIGTERM"),
                int: reg(SignalKind::interrupt(), "SIGINT"),
            }
        }
        #[cfg(not(unix))]
        {
            SignalSet {}
        }
    }

    /// 等待任一终止信号。监听失败的信号永不触发（进程继续常驻）。
    async fn wait(self) {
        #[cfg(unix)]
        {
            let SignalSet { term, int } = self;
            let wait_one = |s: Option<tokio::signal::unix::Signal>| async move {
                match s {
                    Some(mut s) => {
                        s.recv().await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = wait_one(term) => {}
                _ = wait_one(int) => {}
            }
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = tokio::signal::ctrl_c().await {
                tracing::warn!("failed to listen for Ctrl-C: {e}");
                std::future::pending::<()>().await;
            }
        }
    }
}

/// 是否应以无头模式运行。
/// `force`（`--headless`）为真则无头；Windows/macOS 恒有图形会话；
/// Linux 在 `DISPLAY` 与 `WAYLAND_DISPLAY` 都为空时无头。
/// 注意：只看环境变量，不保证显示服务真的可用；连不上时由 Task 9 的 `run_tray` 兜底降级。
pub fn is_headless(env: &dyn Fn(&str) -> Option<String>, force: bool) -> bool {
    if force {
        return true;
    }
    if !cfg!(target_os = "linux") {
        return false;
    }
    let unset = |k: &str| env(k).is_none_or(|v| v.is_empty());
    unset("DISPLAY") && unset("WAYLAND_DISPLAY")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_detection_without_display_env() {
        let none = |_: &str| None;
        let x11 = |k: &str| (k == "DISPLAY").then(|| ":0".to_string());
        #[cfg(target_os = "linux")]
        {
            let empty = |_: &str| Some(String::new());
            assert!(is_headless(&none, false));
            assert!(!is_headless(&x11, false));
            assert!(is_headless(&empty, false));
        }
        assert!(is_headless(&x11, true));
        assert!(is_headless(&none, true));
    }
}
