// P1 核心骨架：lanmsgd 无头守护进程（Task 12）。
// 启动 Service 与 IPC 监听；Ctrl-C 或 Request::Shutdown 时关闭退出。
// 注释中文，标识符英文。

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::fmt::writer::MakeWriter;

use lanmsg_api::ipc::{endpoint_name, IpcListener};
use lanmsg_core::discovery::DiscoveryMode;
use lanmsg_core::keystore::FileKeyStore;
use lanmsg_core::{PlatformHooks, Service, ServiceConfig};
use lanmsg_proto::consts::{LOCALSEND_PORT, SERVICE_PORT};

mod ipc_server;

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
#[command(name = "lanmsgd", about = "LanMsg 无头守护进程")]
struct Args {
    /// 数据目录（默认平台数据目录 C8）
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// 服务监听端口，0 = 系统分配
    #[arg(long, default_value_t = SERVICE_PORT)]
    port: u16,
    /// 启动即尝试直连的对端地址，可重复
    #[arg(long = "peer")]
    peers: Vec<std::net::SocketAddr>,
    /// 禁用组播发现
    #[arg(long)]
    no_multicast: bool,
    /// 日志级别（error/warn/info/debug/trace）
    #[arg(long, default_value = "info")]
    log: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let data_dir = args
        .data_dir
        .clone()
        .unwrap_or_else(lanmsg_core::paths::default_data_dir);
    std::fs::create_dir_all(&data_dir)?;
    init_tracing(&data_dir, &args.log)?;

    let service = Service::start(ServiceConfig {
        data_dir: data_dir.clone(),
        listen_port: args.port,
        discovery: if args.no_multicast {
            DiscoveryMode::Disabled
        } else {
            DiscoveryMode::Multicast
        },
        keystore: Arc::new(FileKeyStore::new(data_dir.join("keys"))),
        static_peers: args.peers,
        platform: PlatformHooks::default(),
        localsend_port: LOCALSEND_PORT,
    })
    .await?;

    let endpoint = endpoint_name(&data_dir);
    let listener = match IpcListener::bind(&endpoint) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            println!("lanmsgd 已在运行");
            std::process::exit(2);
        }
        Err(e) => return Err(e.into()),
    };
    tracing::info!("lanmsgd 已启动，IPC 端点: {endpoint}");

    // Ctrl-C：关闭服务后退出 0。
    let svc_for_signal = service.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            svc_for_signal.shutdown().await;
            std::process::exit(0);
        }
    });

    loop {
        match listener.accept().await {
            Ok(stream) => {
                let svc = service.clone();
                tokio::spawn(ipc_server::serve_connection(svc, stream));
            }
            Err(e) => {
                tracing::warn!("IPC accept 失败: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}
