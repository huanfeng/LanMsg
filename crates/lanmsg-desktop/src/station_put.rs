// `--station-put <PATH>...`：不开窗口，把路径放进本机托管的中转站后退出。
// 供 Windows“发送到”快捷方式使用。注释中文，标识符英文。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lanmsg_api::ipc::{endpoint_name, IpcClient};
use lanmsg_api::types::{Request, Response, StationPutItem, StoreMode};

use crate::bridge::spawn_daemon;
use crate::error_text::error_text;
use crate::strings;

/// 连不上后台服务时，拉起后等待它就绪的上限。
const CONNECT_WAIT: Duration = Duration::from_secs(5);
const CONNECT_RETRY: Duration = Duration::from_millis(200);

/// 放入失败时的错误：已经是可记录到日志的文字。
type Failure = String;

/// 入口：成功返回 Ok；任何一步失败返回错误文字（由 main 写日志并以退出码 1 结束）。
pub fn run_station_put(data_dir: &Path, paths: Vec<PathBuf>) -> Result<(), Failure> {
    if paths.is_empty() {
        return Err("no paths given".into());
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    rt.block_on(put(data_dir, paths))
}

async fn connect(data_dir: &Path) -> Result<IpcClient, Failure> {
    let endpoint = endpoint_name(data_dir);
    if let Ok(c) = IpcClient::connect(&endpoint).await {
        return Ok(c);
    }
    // 与界面桥一致：连不上就拉起 lanmsgd 一次，再重试。
    spawn_daemon(data_dir).map_err(|e| format!("cannot launch lanmsgd: {e}"))?;
    let start = Instant::now();
    loop {
        match IpcClient::connect(&endpoint).await {
            Ok(c) => return Ok(c),
            Err(e) if start.elapsed() >= CONNECT_WAIT => {
                return Err(format!("cannot connect to lanmsgd: {e}"))
            }
            Err(_) => tokio::time::sleep(CONNECT_RETRY).await,
        }
    }
}

async fn put(data_dir: &Path, paths: Vec<PathBuf>) -> Result<(), Failure> {
    let client = connect(data_dir).await?;
    let call = |r: Request| {
        let client = &client;
        async move {
            client
                .call(r)
                .await
                .map_err(|e| format!("{:?}: {}", e.code, error_text(&e.code)))
        }
    };
    let me = match call(Request::GetSelf).await? {
        Response::SelfInfo { value } if value.initialized => value,
        _ => return Err("this device is not initialized".into()),
    };
    // 本机托管的站点：主机名等于本机设备名（StationView 给的是主机设备名）。
    let stations = match call(Request::StationList).await? {
        Response::Stations { value } => value,
        _ => return Err("unexpected reply to StationList".into()),
    };
    let station = match stations
        .into_iter()
        .find(|s| Some(&s.host_device) == me.device_name.as_ref())
    {
        Some(s) => s.station,
        None => match call(Request::StationCreate {
            name: strings::STATION_DEFAULT_NAME.to_string(),
        })
        .await?
        {
            Response::ConvId { value } => value,
            _ => return Err("unexpected reply to StationCreate".into()),
        },
    };
    call(Request::StationPut {
        station,
        item: StationPutItem::Paths {
            paths: paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        },
        mode: StoreMode::Reference,
        expires_at: None,
    })
    .await?;
    Ok(())
}

/// 把失败原因追加到 `<data_dir>/logs/lanmsg.log`（尽力而为）。
pub fn log_failure(data_dir: &Path, msg: &str) {
    use std::io::Write;
    let dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("lanmsg.log"))
    {
        let _ = writeln!(f, "station-put failed: {msg}");
    }
}
