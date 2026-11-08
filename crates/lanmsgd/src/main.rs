// Windows 发布版用 GUI 子系统：开机自启/界面拉起时不弹控制台窗口。
// 无控制台时 stdout/stderr 句柄无效：Rust 标准库对无效句柄的写入按成功处理（不 panic），
// 且 Tee 本就忽略 stdout 写错误，日志仍落到文件。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// lanmsgd 守护进程入口（P1 Task 12；P3 Task 8 起线程模型见 runtime.rs）。
// 无头时阻塞等待 Ctrl-C / SIGTERM / Request::Shutdown；有图形会话时走托盘（tray.rs）。
// 注释中文，标识符英文。

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::fmt::writer::MakeWriter;

use lanmsg_proto::consts::SERVICE_PORT;

mod firewall_cmd;
mod ipc_server;
#[cfg(feature = "desktop")]
mod launch;
#[cfg(feature = "desktop")]
mod notify;
mod runtime;
#[cfg(feature = "desktop")]
mod strings;
mod thumbnailer;
#[cfg(feature = "desktop")]
mod tray;

/// 同时写 stdout 与日志文件的 tracing writer。
struct MakeTee(Arc<Mutex<std::fs::File>>);

impl<'a> MakeWriter<'a> for MakeTee {
    type Writer = Tee;

    fn make_writer(&'a self) -> Self::Writer {
        Tee(self.0.clone())
    }
}

/// 每次写入同时落到 stdout 与日志文件。
struct Tee(Arc<Mutex<std::fs::File>>);

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stdout().write_all(buf);
        let _ = self.0.lock().unwrap().write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stdout().flush();
        let _ = self.0.lock().unwrap().flush();
        Ok(())
    }
}

/// 初始化 tracing：stdout + `<data_dir>/logs/lanmsgd.log` 单文件追加。
fn init_tracing(data_dir: &std::path::Path, level: &str) -> Result<()> {
    let logs = data_dir.join("logs");
    std::fs::create_dir_all(&logs)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join("lanmsgd.log"))?;
    let level = level.parse().unwrap_or(tracing::Level::INFO);
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(MakeTee(Arc::new(Mutex::new(file))))
        .init();
    Ok(())
}

#[derive(Parser)]
#[command(name = "lanmsgd", about = "LanMsg daemon")]
pub(crate) struct Args {
    /// 数据目录（默认平台数据目录 C8）
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// 服务监听端口，0 = 系统分配
    #[arg(long, default_value_t = SERVICE_PORT)]
    pub(crate) port: u16,
    /// 启动即尝试直连的对端地址，可重复
    #[arg(long = "peer")]
    pub(crate) peers: Vec<std::net::SocketAddr>,
    /// 禁用组播发现
    #[arg(long)]
    pub(crate) no_multicast: bool,
    /// 日志级别（error/warn/info/debug/trace）
    #[arg(long, default_value = "info")]
    log: String,
    /// 强制无头模式（不显示托盘与界面）
    #[arg(long)]
    headless: bool,
    /// 添加 Windows 防火墙入站放行规则后退出（需要管理员权限；其他平台无操作）
    #[arg(long)]
    add_firewall_rule: bool,
}

impl Args {
    /// 数据目录：命令行指定，否则平台默认。
    pub(crate) fn resolved_data_dir(&self) -> PathBuf {
        self.data_dir
            .clone()
            .unwrap_or_else(lanmsg_core::paths::default_data_dir)
    }
}

fn main() {
    if let Err(e) = run() {
        if e.downcast_ref::<runtime::AlreadyRunning>().is_some() {
            println!("lanmsgd is already running");
            std::process::exit(2);
        }
        eprintln!("lanmsgd failed to start: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.add_firewall_rule {
        std::process::exit(firewall_cmd::run_add_rule());
    }
    let data_dir = args.resolved_data_dir();
    std::fs::create_dir_all(&data_dir)?;
    init_tracing(&data_dir, &args.log)?;

    let rt = runtime::start_runtime(&args)?;
    let env = |k: &str| std::env::var(k).ok();
    if runtime::is_headless(&env, args.headless) {
        tracing::info!("running in headless mode");
        // 规则缺失只记日志：无头模式没有托盘入口，IPC 客户端会收到事件
        #[cfg(windows)]
        firewall_cmd::spawn_monitor(rt.firewall_tx(), |missing| {
            if missing {
                tracing::warn!("firewall rule for lanmsgd is missing");
            }
        });
        rt.block_until_shutdown();
    } else {
        #[cfg(feature = "desktop")]
        tray::run_tray(rt, data_dir);
    }
    Ok(())
}
