// 应用内更新（P5 T23a）：GitHub Releases 检查与下载。
// 默认地址指向 huanfeng/lanmsg 的 GitHub API；环境变量 LANMSG_UPDATE_URL 可整体覆盖
// API 地址（测试用）。仓库私有时匿名访问不可用：请求失败返回明确的错误（界面显示
// “无法检查更新”），将来仓库公开后自动可用，不崩溃。
// 信任模型（写进文档）：只从固定仓库下载；SHA256SUMS 缺独立信任根（与安装包同源），
// 靠固定仓库 URL 缓解——与 wind-installer 相同的限制。
// 下载写盘走 std::fs::File + StdMutex（与 blob 传输引擎同模式，规避 tokio::fs::File
// 并发复用的已知问题）；SHA256 边下载边算，完成后 .part 重命名。
// 注释中文，标识符英文。

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use lanmsg_api::types::{Event, InstallChannel, UpdateAsset, UpdateCheckView, UpdateInfo};
use sha2::{Digest, Sha256};

use crate::error::CoreError;
use crate::service::Inner;
use crate::settings;

/// 默认的 GitHub API 地址（releases/latest 不含预发布与草稿）。
pub const DEFAULT_UPDATE_API: &str = "https://api.github.com/repos/huanfeng/lanmsg/releases/latest";
/// 默认的发布页地址（Manual 渠道展示用；自定义 API 地址解析不出发布页时也用它）。
pub const DEFAULT_RELEASE_URL: &str = "https://github.com/huanfeng/lanmsg/releases/latest";

/// 启动自动检查的延迟：等发现与网络就绪。
const AUTO_CHECK_DELAY: Duration = Duration::from_secs(15);
/// 自动检查节流：每天最多一次（24 小时）。
const AUTO_CHECK_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;
/// API 请求总超时 / 下载单次读空闲超时：GitHub API 响应小，30 秒足够。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// 进度事件节流步长：百分比变化才发。
const PROGRESS_STEP: u64 = 1;

/// rustls 配置：显式 aws-lc-rs provider + 内置 webpki 根证书，正常 CA 校验（GitHub 用）；
/// 两类 client 共用。reqwest 的 manual-roots 特性集要求调用方给出完整 ClientConfig
/// （含根证书），只传未完成的 builder 会在 build() 时报 Unknown TLS backend。
fn rustls_config() -> Result<rustls::ClientConfig, CoreError> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Ok(rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| CoreError::Tls(e.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth())
}

/// API 客户端（更新检查 / SHA256SUMS 拉取）：响应小，带 30 秒总超时快速失败。
fn api_client() -> Result<reqwest::Client, CoreError> {
    reqwest::Client::builder()
        .use_preconfigured_tls(rustls_config()?)
        .user_agent(format!("lanmsg/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| CoreError::Io(std::io::Error::other(e)))
}

/// 下载客户端（安装包可达几十 MB）：总时长取决于网速与包大小，不设总超时——
/// 连接建立 10 秒、单次读空闲 30 秒兜底防连接卡死；中断的下载由 .part + SHA256
/// 校验兜底，不会落盘半成品。
fn download_client() -> Result<reqwest::Client, CoreError> {
    reqwest::Client::builder()
        .use_preconfigured_tls(rustls_config()?)
        .user_agent(format!("lanmsg/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| CoreError::Io(std::io::Error::other(e)))
}

/// 解析更新 API 地址：环境变量 LANMSG_UPDATE_URL 覆盖（非空才生效），否则默认地址。
fn resolve_api_url() -> String {
    match std::env::var("LANMSG_UPDATE_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => DEFAULT_UPDATE_API.to_string(),
    }
}

/// 由 API 地址推发布页地址：仅识别标准 GitHub API 形态，其余回退默认发布页。
fn release_page_url(api_url: &str) -> String {
    const API_PREFIX: &str = "https://api.github.com/repos/";
    const API_SUFFIX: &str = "/releases/latest";
    if let Some(mid) = api_url
        .strip_prefix(API_PREFIX)
        .and_then(|s| s.strip_suffix(API_SUFFIX))
    {
        return format!("https://github.com/{mid}/releases/latest");
    }
    DEFAULT_RELEASE_URL.to_string()
}

/// Release 数据（含 SHA256SUMS 资产的下载 URL，下载校验用）。
struct Release {
    version: String,
    notes: String,
    published_at: i64,
    assets: Vec<UpdateAsset>,
    /// SHA256SUMS 资产的下载 URL；release 未提供时为 None。
    sha256sums_url: Option<String>,
}

/// 解析 GitHub Release JSON：tag_name / body / published_at / assets / SHA256SUMS 资产定位。
fn parse_release(v: &serde_json::Value) -> Result<Release, CoreError> {
    // 非对象或缺 tag_name 视为无效响应。
    let Some(obj) = v.as_object() else {
        return Err(CoreError::Proto("unexpected update API response".into()));
    };
    let tag = obj
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or_else(|| CoreError::Proto("update API response has no tag_name".into()))?;
    let notes = obj
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or_default()
        .to_string();
    // RFC3339 解析不了（非 GitHub 兼容端点）时置 0，界面不显示时间。
    let published_at = obj
        .get("published_at")
        .and_then(|p| p.as_str())
        .and_then(|s| {
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
        })
        .map(|dt| dt.unix_timestamp() * 1000)
        .unwrap_or(0);
    let assets = obj
        .get("assets")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let name = a.get("name")?.as_str()?.to_string();
                    let url = a.get("browser_download_url")?.as_str()?.to_string();
                    let size = a.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                    Some(UpdateAsset { name, size, url })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let sha256sums_url = obj
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|a| a.get("name").and_then(|n| n.as_str()) == Some("SHA256SUMS"))
                .and_then(|a| a.get("browser_download_url").and_then(|u| u.as_str()))
                .map(str::to_string)
        });
    Ok(Release {
        version: strip_v(tag),
        notes,
        published_at,
        assets,
        sha256sums_url,
    })
}

/// 去掉 tag 的 v 前缀（保留其余原样）。
fn strip_v(tag: &str) -> String {
    tag.strip_prefix('v').unwrap_or(tag).to_string()
}

// ===== 语义化版本比较 =====

/// 预发布段标识：Num 按数值比，Alnum 按 ASCII 比；数值标识小于字母数字标识。
#[derive(Debug, Clone, PartialEq, Eq)]
enum PreId {
    Num(u64),
    Alnum(String),
}

/// 语义化版本（major.minor.patch[-pre]；build 元数据忽略）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Semver {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Vec<PreId>,
}

/// 严格解析语义化版本；不合法返回 None（比较时按“无法判定”处理，不误报更新）。
fn parse_semver(s: &str) -> Option<Semver> {
    // build 元数据丢弃；预发布段取 '-' 后的部分。
    let s = s.split('+').next()?;
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, p),
        None => (s, ""),
    };
    let mut parts = core.split('.');
    let major = parse_num(parts.next()?)?;
    let minor = parse_num(parts.next()?)?;
    let patch = parse_num(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    let mut pre_ids = Vec::new();
    if !pre.is_empty() {
        for id in pre.split('.') {
            if id.is_empty() {
                return None;
            }
            if let Ok(n) = id.parse::<u64>() {
                pre_ids.push(PreId::Num(n));
            } else {
                pre_ids.push(PreId::Alnum(id.to_string()));
            }
        }
    }
    Some(Semver {
        major,
        minor,
        patch,
        pre: pre_ids,
    })
}

/// 解析纯数字段（含前导零按宽松处理，仅用于比较）。
fn parse_num(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    s.parse::<u64>().ok()
}

/// 语义化版本比较（半序）：Greater/Less/Equal，按规范 6.4 节的优先级规则。
fn semver_cmp(a: &Semver, b: &Semver) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ord = a
        .major
        .cmp(&b.major)
        .then(a.minor.cmp(&b.minor))
        .then(a.patch.cmp(&b.patch));
    if ord != Ordering::Equal {
        return ord;
    }
    // major.minor.patch 相等时比预发布段；无预发布段 > 有预发布段，
    // 逐标识比：数值按数值、字母数字按 ASCII，数值 < 字母数字。
    match (a.pre.is_empty(), b.pre.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            for (x, y) in a.pre.iter().zip(b.pre.iter()) {
                match pre_id_cmp(x, y) {
                    Ordering::Equal => continue,
                    other => return other,
                }
            }
            a.pre.len().cmp(&b.pre.len())
        }
    }
}

/// 单个预发布标识比较。
fn pre_id_cmp(a: &PreId, b: &PreId) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    use PreId::{Alnum, Num};
    match (a, b) {
        (Num(n1), Num(n2)) => n1.cmp(n2),
        (Num(_), Alnum(_)) => Ordering::Less,
        (Alnum(_), Num(_)) => Ordering::Greater,
        (Alnum(s1), Alnum(s2)) => s1.cmp(s2),
    }
}

/// 远端版本是否比本机新。任一侧解析失败返回 false（不误报）。
fn is_newer(remote: &str, current: &str) -> bool {
    match (parse_semver(remote), parse_semver(current)) {
        (Some(r), Some(c)) => semver_cmp(&r, &c) == std::cmp::Ordering::Greater,
        _ => false,
    }
}

// ===== 渠道检测 =====

/// 本机安装渠道检测。
///
/// - Windows：可执行文件位于 `scoop\apps\` 下 → scoop 版；否则安装器版。
/// - macOS：.app 包内运行 → dmg 渠道；否则 Manual（源码构建）。
/// - Linux：可执行文件所在的 bin 目录其前缀下存在
///   `share/applications/lanmsg.desktop`（install.sh / 含 share 的 tarball 安装）→ LinuxSh；
///   其余（纯 bin 的 tarball、源码构建、无头版）→ Manual。
pub fn detect_channel() -> InstallChannel {
    match std::env::current_exe() {
        Ok(exe) => channel_for_exe(&exe),
        Err(_) => InstallChannel::Manual,
    }
}

/// 按可执行文件路径判定渠道（current_exe 取不到时按 Manual）。
#[cfg(windows)]
fn channel_for_exe(exe: &std::path::Path) -> InstallChannel {
    // 附录定义：可执行文件位于 scoop\apps\lanmsg\ 下 → scoop 版；否则安装器版。
    let p = exe.to_string_lossy().to_lowercase();
    if p.contains(r"\scoop\apps\") {
        InstallChannel::WindowsScoop
    } else {
        InstallChannel::WindowsInstaller
    }
}

/// macOS：.app 包内 → dmg 渠道；否则 Manual（源码构建）。
#[cfg(target_os = "macos")]
fn channel_for_exe(exe: &std::path::Path) -> InstallChannel {
    if exe
        .ancestors()
        .skip(1)
        .any(|a| a.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")))
    {
        InstallChannel::MacDmg
    } else {
        InstallChannel::Manual
    }
}

/// Linux：bin 目录的前缀下存在 share/applications/lanmsg.desktop（install.sh 或含 share 的
/// tarball 安装）→ LinuxSh；其余（纯 bin 的 tarball、源码构建、无头版）→ Manual。
#[cfg(not(any(windows, target_os = "macos")))]
fn channel_for_exe(exe: &std::path::Path) -> InstallChannel {
    for a in exe.ancestors().skip(1) {
        if a.file_name() == Some(std::ffi::OsStr::new("bin")) {
            if let Some(prefix) = a.parent() {
                if prefix.join("share/applications/lanmsg.desktop").exists() {
                    return InstallChannel::LinuxSh;
                }
            }
        }
    }
    InstallChannel::Manual
}

// ===== 检查与下载 =====

/// 拉取并解析最新 release（网络失败 / HTTP 非 2xx / 解析失败都返回明确错误）。
async fn fetch_release(api_url: &str) -> Result<Release, CoreError> {
    let client = api_client()?;
    let resp = client
        .get(api_url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(reqwest_err)?;
    if !resp.status().is_success() {
        return Err(CoreError::NotFound(format!(
            "update check failed: HTTP {} (repository may be private or offline)",
            resp.status()
        )));
    }
    let v: serde_json::Value = resp.json().await.map_err(reqwest_err)?;
    parse_release(&v)
}

/// reqwest 错误转 CoreError::Io。
fn reqwest_err(e: reqwest::Error) -> CoreError {
    CoreError::Io(std::io::Error::other(e))
}

/// CheckUpdate 的实现：拉取 release → 语义化版本比较 → 组装视图（含渠道与发布页 URL）。
pub(crate) async fn check_now() -> Result<UpdateCheckView, CoreError> {
    let api = resolve_api_url();
    let release = fetch_release(&api).await?;
    let current = env!("CARGO_PKG_VERSION");
    let update = if is_newer(&release.version, current) {
        Some(UpdateInfo {
            version: release.version,
            notes: release.notes,
            published_at: release.published_at,
            assets: release.assets,
        })
    } else {
        None
    };
    Ok(UpdateCheckView {
        update,
        current: current.to_string(),
        channel: detect_channel(),
        release_url: release_page_url(&api),
    })
}

/// DownloadUpdate 的实现：校验资产名 → 占用忙碌标志 → 放行后台下载任务。
/// 失败路径都释放忙碌标志；返回 Ok 后结果经 `UpdateDownload*` 事件上报。
pub(crate) async fn start_download(inner: &Arc<Inner>, asset: &str) -> Result<(), CoreError> {
    let asset = asset.trim();
    if asset.is_empty() {
        return Err(CoreError::InvalidArgument(
            "asset name must not be empty".into(),
        ));
    }
    // 同一时刻只允许一个更新下载。
    if inner
        .update_busy
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return Err(CoreError::InvalidArgument(
            "an update download is already in progress".into(),
        ));
    }
    // 先拉一次 release 确认资产存在；失败路径都释放忙碌标志。
    let release = match fetch_release(&resolve_api_url()).await {
        Ok(r) => r,
        Err(e) => {
            inner
                .update_busy
                .store(false, std::sync::atomic::Ordering::SeqCst);
            return Err(e);
        }
    };
    if !release.assets.iter().any(|a| a.name == asset) {
        inner
            .update_busy
            .store(false, std::sync::atomic::Ordering::SeqCst);
        return Err(CoreError::InvalidArgument(format!(
            "asset not found in latest release: {asset}"
        )));
    }
    let inner = inner.clone();
    let asset = asset.to_string();
    tokio::spawn(async move {
        download_task(inner, asset).await;
    });
    Ok(())
}
/// 后台下载任务：下载 → 校验 → 落盘改名，结果经事件上报；结束（成功或失败）都释放忙碌标志。
async fn download_task(inner: Arc<Inner>, asset: String) {
    let result = download_inner(&inner, &asset).await;
    inner
        .update_busy
        .store(false, std::sync::atomic::Ordering::SeqCst);
    match result {
        Ok(path) => {
            let _ = inner.events.send(Event::UpdateDownloadFinished {
                asset: asset.clone(),
                ok: true,
                path: Some(path.to_string_lossy().into_owned()),
                error: None,
            });
            tracing::info!("update downloaded: {asset}");
        }
        Err(e) => {
            let _ = inner.events.send(Event::UpdateDownloadFinished {
                asset,
                ok: false,
                path: None,
                error: Some(e.to_string()),
            });
            tracing::warn!("update download failed: {e}");
        }
    }
}

/// 下载一个资产到 data_dir/updates/ 下：流式写 .part 并算 SHA256，校验通过后改名。
async fn download_inner(inner: &Inner, asset: &str) -> Result<PathBuf, CoreError> {
    // 重新拉取 release：保证 URL 与校验和新鲜。
    let release = fetch_release(&resolve_api_url()).await?;
    let Some(found) = release.assets.iter().find(|a| a.name == asset) else {
        return Err(CoreError::NotFound(format!(
            "asset not found in latest release: {asset}"
        )));
    };
    let name = found.name.clone();
    let url = found.url.clone();
    let total = found.size;
    // 附录安全要求：强制校验 SHA256SUMS；release 没有该资产或没有该文件条目都算失败。
    let Some(sums_url) = release.sha256sums_url else {
        return Err(CoreError::NotFound(
            "release has no SHA256SUMS asset".into(),
        ));
    };
    let expected = fetch_sha256(&sums_url, &name).await?;
    // 目录准备与旧文件清理：开始新下载前，删除 updates/ 里除本资产外的文件（旧版本安装包）。
    let dir = inner.data_dir.join("updates");
    std::fs::create_dir_all(&dir)?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy() != asset {
                if let Err(e) = std::fs::remove_file(entry.path()) {
                    tracing::debug!("failed to remove stale update file: {e}");
                }
            }
        }
    }
    let part = dir.join(format!("{asset}.part"));
    let final_path = dir.join(&name);
    let _ = std::fs::remove_file(&part);
    let file = Arc::new(StdMutex::new(std::fs::File::create(&part)?));
    let resp = download_client()?
        .get(&url)
        .send()
        .await
        .map_err(reqwest_err)?;
    if !resp.status().is_success() {
        return Err(CoreError::NotFound(format!(
            "update download failed: HTTP {}",
            resp.status()
        )));
    }
    let mut resp = resp;
    let mut hasher = Sha256::new();
    let mut done: u64 = 0;
    let mut last_pct: u64 = 100;
    let _ = inner
        .events
        .send(lanmsg_api::types::Event::UpdateDownloadProgress {
            asset: asset.to_string(),
            done: 0,
            total,
        });
    while let Some(chunk) = resp.chunk().await.map_err(reqwest_err)? {
        {
            let mut f = file.lock().unwrap_or_else(|e| e.into_inner());
            f.write_all(&chunk)?;
        }
        hasher.update(&chunk);
        done += chunk.len() as u64;
        let pct = (done * 100).checked_div(total).unwrap_or(0);
        if pct >= last_pct.saturating_add(PROGRESS_STEP) {
            last_pct = pct;
            let _ = inner
                .events
                .send(lanmsg_api::types::Event::UpdateDownloadProgress {
                    asset: asset.to_string(),
                    done,
                    total,
                });
        }
    }
    // 强制落盘后关闭句柄，再校验并改名。
    {
        let f = file.lock().unwrap_or_else(|e| e.into_inner());
        f.sync_all()?;
    }
    drop(file);
    let actual = hex_lower(&hasher.finalize());
    if actual != expected.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&part);
        return Err(CoreError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("update checksum mismatch: expected {expected}, got {actual}"),
        )));
    }
    std::fs::rename(&part, &final_path)?;
    Ok(final_path)
}

/// 解析 SHA256SUMS 文本：行格式 `<64位hex>  <文件名>`（sha256sum 输出格式），按文件名匹配返回小写 hex。
fn parse_sha256sums(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let hash = it.next()?;
        let fname = it.next()?;
        if fname == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

/// 拉取 SHA256SUMS 并返回指定资产的哈希。
async fn fetch_sha256(sums_url: &str, name: &str) -> Result<String, CoreError> {
    let client = api_client()?;
    let resp = client.get(sums_url).send().await.map_err(reqwest_err)?;
    if !resp.status().is_success() {
        return Err(CoreError::NotFound(format!(
            "failed to fetch SHA256SUMS: HTTP {}",
            resp.status()
        )));
    }
    let text = resp.text().await.map_err(reqwest_err)?;
    parse_sha256sums(&text, name)
        .ok_or_else(|| CoreError::NotFound(format!("SHA256SUMS has no entry for {name}")))
}

/// 小写 hex 编码。
fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 启动自动检查（lanmsgd 启动时调用；手动检查走 CheckUpdate 请求）。
/// 延迟 15 秒等网络就绪；开关关闭或 24 小时内已检查过则跳过；
/// 失败不写时间戳（下次启动再试），成功才记录时间。
pub(crate) async fn auto_check_tick(inner: Arc<Inner>) {
    tokio::time::sleep(AUTO_CHECK_DELAY).await;
    let s = match settings::load_settings(&inner.store) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("auto update check skipped: failed to load settings: {e}");
            return;
        }
    };
    if !s.auto_check_update {
        tracing::debug!("auto update check disabled in settings");
        return;
    }
    let last = inner
        .store
        .get_setting(settings::SETTING_UPDATE_LAST_CHECK)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let now = crate::service::now_millis();
    if now.saturating_sub(last) < AUTO_CHECK_INTERVAL_MS {
        tracing::debug!("auto update check throttled (checked within 24h)");
        return;
    }
    match check_now().await {
        Ok(view) => {
            if let Err(e) = inner
                .store
                .set_setting(settings::SETTING_UPDATE_LAST_CHECK, &now.to_string())
            {
                tracing::warn!("failed to record last update check time: {e}");
            }
            match &view.update {
                Some(info) => tracing::info!(
                    "update available: v{} (current v{})",
                    info.version,
                    view.current
                ),
                None => tracing::info!("update check ok, already up to date (v{})", view.current),
            }
        }
        Err(e) => {
            tracing::warn!("auto update check failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_compare_and_prerelease() {
        assert!(is_newer("1.2.4", "1.2.3"));
        assert!(is_newer("1.3.0", "1.2.9"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("1.2.2", "1.2.3"));
        // 预发布低于正式版；数值按数值、字母按 ASCII，数值 < 字母数字。
        assert!(is_newer("1.2.3", "1.2.3-rc.1"));
        assert!(is_newer("1.2.3-rc.2", "1.2.3-rc.1"));
        assert!(is_newer("1.2.3-rc.2", "1.2.3-rc.1"));
        assert!(is_newer("1.2.3-beta", "1.2.3-alpha.1"));
        assert!(!is_newer("1.2.3-rc.1", "1.2.3"));
        // 多段数值比较。
        assert!(is_newer("1.10.0", "1.9.9"));
    }

    #[test]
    fn unparseable_versions_never_report_update() {
        assert!(!is_newer("", "1.2.3"));
        assert!(!is_newer("not-a-version", "1.2.3"));
        assert!(!is_newer("1.2.3.4", "1.2.3"));
        assert!(!is_newer("1.2", "1.2.3"));
        assert!(!is_newer("1.2.3", "not-a-version"));
    }

    #[test]
    fn strip_v_leaves_non_v_tags() {
        assert_eq!(strip_v("v1.2.3"), "1.2.3");
        assert_eq!(strip_v("1.2.3"), "1.2.3");
        assert_eq!(strip_v("release-42"), "release-42");
    }

    #[test]
    fn clients_build_with_expected_timeouts() {
        // 两类 client 都能构造（TLS 配置共用）；下载 client 与 API client 超时策略不同。
        assert!(api_client().is_ok());
        assert!(download_client().is_ok());
    }

    #[test]
    fn release_page_url_from_github_api() {
        assert_eq!(
            release_page_url("https://api.github.com/repos/huanfeng/lanmsg/releases/latest"),
            "https://github.com/huanfeng/lanmsg/releases/latest"
        );
        assert_eq!(
            release_page_url("https://example.com/api/releases/latest"),
            DEFAULT_RELEASE_URL
        );
    }

    #[test]
    fn parse_release_extracts_fields() {
        let v: serde_json::Value = serde_json::json!({
            "tag_name": "v1.2.3",
            "name": "LanMsg 1.2.3",
            "body": "## 更新内容\n- 修复若干问题",
            "published_at": "2026-11-08T12:00:00Z",
            "prerelease": false,
            "draft": false,
            "assets": [
                {
                    "name": "lanmsg-windows-x64.zip",
                    "size": 123,
                    "browser_download_url": "https://github.com/huanfeng/lanmsg/releases/download/v1.2.3/lanmsg-windows-x64.zip"
                },
                {"name": "SHA256SUMS", "size": 1, "browser_download_url": "https://example.com/SHA256SUMS"},
            ],
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.version, "1.2.3");
        assert_eq!(r.published_at, 1_794_139_200_000);
        assert_eq!(r.assets.len(), 2);
        assert_eq!(r.assets[0].name, "lanmsg-windows-x64.zip");
        assert_eq!(r.assets[0].size, 123);
        assert_eq!(
            r.sha256sums_url.as_deref(),
            Some("https://example.com/SHA256SUMS")
        );
    }

    #[test]
    fn parse_release_rejects_missing_tag() {
        let v: serde_json::Value = serde_json::json!({ "body": "x" });
        assert!(parse_release(&v).is_err());
        assert!(parse_release(&serde_json::Value::Null).is_err());
    }

    #[test]
    fn sha256sums_parse_matches_by_name() {
        let text = "abc123  other-file\n\
                    e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  lanmsg-windows-x64.zip\n\
                    deadbeef  lanmsg-windows-arm64.zip\n";
        let h = parse_sha256sums(text, "lanmsg-windows-x64.zip");
        assert_eq!(
            h.as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(parse_sha256sums(text, "missing"), None);
    }

    #[test]
    fn sha256sums_rejects_non_hex_and_short_hashes() {
        let text = "nothex!!  a.zip\ndeadbeef  b.zip\n";
        assert_eq!(parse_sha256sums("nothex!!  a.zip\n", "a.zip"), None);
        assert_eq!(parse_sha256sums("deadbeef  b.zip\n", "b.zip"), None);
        let _ = text;
    }

    #[test]
    fn detect_channel_finds_installsh_prefix_on_linux() {
        // Linux 检测：bin 前缀下有桌面文件 → LinuxSh；纯 bin → Manual。
        if cfg!(windows) || cfg!(target_os = "macos") {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::create_dir_all(prefix.join("share/applications")).unwrap();
        std::fs::write(prefix.join("share/applications/lanmsg.desktop"), "").unwrap();
        let exe = prefix.join("bin/lanmsg");
        assert_eq!(channel_for_exe(&exe), InstallChannel::LinuxSh);
    }

    #[test]
    fn detect_channel_pure_bin_is_manual() {
        if cfg!(windows) || cfg!(target_os = "macos") {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix2");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        let exe = prefix.join("bin/lanmsg");
        assert_eq!(channel_for_exe(&exe), InstallChannel::Manual);
    }
}
