// 中转站同步：主机向自己的设备通告 / 推送，拉取端订阅并镜像清单，
// 拉取端经主机代理写入（Put / Remove / SetPinned），主机拉取手机放入的内容。
// 内容写盘前的路径清洗见 `host::copy_layout`。
// 注释中文，标识符英文。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lanmsg_api::types::{Event, StationPutItem};
use lanmsg_proto::consts::MAX_FOLDER_ENTRIES;
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId, UserId};
use lanmsg_proto::station::{ItemId, StationContent, StationInfo, StationItem, StationMsg};
use lanmsg_proto::types::{BlobMeta, FolderEntry};
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::blob::register::register_local_file;
use crate::error::CoreError;
use crate::net::PeerInfo;
use crate::service::{now_millis, Inner};
use crate::station::host::{copy_layout, item_dir, resolve_path};
use crate::store::{BlobLocalKind, ConvKindRow, ConvRow, StationItemRow, TransferState};

/// 等待主机应答的上限。
const RESULT_TIMEOUT: Duration = Duration::from_secs(30);
/// 文字条目字节上限（与主机端一致）。
const MAX_TEXT_BYTES: usize = 64 * 1024;
/// 单次 Snapshot / Delta 最多接受的条目数。
const MAX_ITEMS_PER_MSG: usize = 5_000;

type ResultTx = oneshot::Sender<(bool, Option<String>)>;

/// 拉取端状态：请求编号、等待表，以及主机端正在拉取的条目集合。
#[derive(Default)]
pub(crate) struct StationClient {
    next_req: AtomicU64,
    pending: Mutex<HashMap<u64, ResultTx>>,
    pulling: Mutex<HashSet<ItemId>>,
}

impl StationClient {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn register(&self) -> (u64, oneshot::Receiver<(bool, Option<String>)>) {
        // 0 留给没有请求编号的 Subscribe 拒绝应答。
        let req = self.next_req.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(req, tx);
        (req, rx)
    }

    fn forget(&self, req: u64) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&req);
    }

    fn complete(&self, req: u64, ok: bool, error: Option<String>) {
        // 先取出 sender 再发送，不在持锁时做其他事。
        let tx = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&req);
        if let Some(tx) = tx {
            let _ = tx.send((ok, error));
        }
    }
}

fn self_user(inner: &Inner) -> Option<UserId> {
    inner
        .identity
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .user_id()
}

fn self_device(inner: &Inner) -> DeviceId {
    inner
        .identity
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .device_id()
}

fn is_own(inner: &Inner, peer: &PeerInfo) -> bool {
    let me = self_user(inner);
    me.is_some() && peer.cert.as_ref().map(|c| c.user) == me
}

fn own_peers(inner: &Inner) -> Vec<PeerInfo> {
    inner
        .conns
        .peers()
        .into_iter()
        .filter(|p| is_own(inner, p))
        .collect()
}

fn station_updated(inner: &Inner, station: &ConvId) {
    let _ = inner.events.send(Event::StationUpdated {
        station: station.to_hex(),
    });
}

/// 本机托管的站点。
fn hosted_stations(inner: &Inner) -> Vec<StationInfo> {
    let me = self_device(inner);
    inner
        .store
        .list_stations()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.host == me && !s.deleted)
        .collect()
}

async fn send_station(inner: &Inner, to: &DeviceId, msg: StationMsg) {
    if let Err(e) = inner.conns.send(to, Frame::Station(msg)).await {
        debug!(peer = %to, error = %e, "sending station frame failed");
    }
}

/// 自己的设备连接建立：通告本机托管的站点。
pub(crate) async fn on_peer_up(inner: &Arc<Inner>, peer: &PeerInfo) {
    if !is_own(inner, peer) {
        return;
    }
    let stations = hosted_stations(inner);
    if stations.is_empty() {
        return;
    }
    send_station(inner, &peer.device, StationMsg::Announce { stations }).await;
}

/// 向全部在线的自己设备通告（新建站点后调用）。
pub(crate) async fn announce_all(inner: &Arc<Inner>) {
    let stations = hosted_stations(inner);
    if stations.is_empty() {
        return;
    }
    for p in own_peers(inner) {
        send_station(
            inner,
            &p.device,
            StationMsg::Announce {
                stations: stations.clone(),
            },
        )
        .await;
    }
}

/// 把条目变化推送给在线的自己设备。
pub(crate) async fn push_delta(inner: &Arc<Inner>, station: ConvId, items: Vec<StationItem>) {
    if items.is_empty() {
        return;
    }
    for p in own_peers(inner) {
        send_station(
            inner,
            &p.device,
            StationMsg::Delta {
                station,
                items: items.clone(),
            },
        )
        .await;
    }
}

/// 处理对端发来的 Station 帧。
pub(crate) async fn on_message(inner: Arc<Inner>, from: DeviceId, msg: StationMsg) {
    let Some(peer) = inner.conns.peer(&from) else {
        debug!(peer = %from, "station frame from unregistered connection dropped");
        return;
    };
    match msg {
        StationMsg::Announce { stations } => on_announce(&inner, &peer, stations).await,
        StationMsg::Subscribe { station, since } => {
            on_subscribe(&inner, &peer, station, since).await
        }
        StationMsg::Snapshot { info, items, .. } => on_snapshot(&inner, &peer, info, items),
        StationMsg::Delta { station, items } => on_delta(&inner, &peer, station, items),
        StationMsg::Put {
            station,
            req,
            item,
            mode_copy,
        } => on_put(&inner, &peer, station, req, item, mode_copy).await,
        StationMsg::Remove { station, req, item } => {
            let r = match owned_station(&inner, &peer, &station) {
                Ok(_) => inner.station.remove(&station, &item, now_millis()).await,
                Err(e) => Err(e),
            };
            reply(&inner, &peer.device, req, r).await;
        }
        StationMsg::SetPinned {
            station,
            req,
            item,
            pinned,
        } => {
            let r = match owned_station(&inner, &peer, &station) {
                Ok(_) => inner
                    .station
                    .set_pinned(&station, &item, pinned, now_millis()),
                Err(e) => Err(e),
            };
            reply(&inner, &peer.device, req, r).await;
        }
        StationMsg::Result { req, ok, error } => {
            // 只接受自己的设备的应答，避免他人抢答。
            if is_own(&inner, &peer) {
                inner.station_client.complete(req, ok, error);
            }
        }
    }
}

/// 校验对端是站点主人且站点在本机托管。
fn owned_station(
    inner: &Inner,
    peer: &PeerInfo,
    station: &ConvId,
) -> Result<StationInfo, CoreError> {
    let info = inner.station.hosted(station)?;
    if peer.cert.as_ref().map(|c| c.user) != Some(info.owner) {
        return Err(CoreError::Rejected("access denied".into()));
    }
    Ok(info)
}

async fn reply(inner: &Inner, to: &DeviceId, req: u64, r: Result<(), CoreError>) {
    let (ok, error) = match r {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    send_station(inner, to, StationMsg::Result { req, ok, error }).await;
}

// ---- 拉取端：通告、快照、增量 ----

async fn on_announce(inner: &Arc<Inner>, peer: &PeerInfo, stations: Vec<StationInfo>) {
    let (Some(me), true) = (self_user(inner), is_own(inner, peer)) else {
        return;
    };
    let my_device = self_device(inner);
    for info in stations.into_iter().take(64) {
        // 只接受对端自己托管、主人是本用户的站点。
        if info.host != peer.device || info.owner != me || info.host == my_device {
            continue;
        }
        let name = info.name.chars().take(64).collect::<String>();
        let info = StationInfo { name, ..info };
        if let Err(e) = inner.store.upsert_station(&info) {
            warn!(error = %e, "storing announced station failed");
            continue;
        }
        let _ = inner.store.ensure_conversation(&ConvRow {
            conv: info.station,
            kind: ConvKindRow::Station,
            peer_user: None,
            title: Some(info.name.clone()),
            created_at: info.created_at,
        });
        let since = inner.store.synced_until(&info.station).unwrap_or(0);
        send_station(
            inner,
            &peer.device,
            StationMsg::Subscribe {
                station: info.station,
                since,
            },
        )
        .await;
        station_updated(inner, &info.station);
    }
}

fn on_snapshot(inner: &Arc<Inner>, peer: &PeerInfo, info: StationInfo, items: Vec<StationItem>) {
    if !is_own(inner, peer) || info.host != peer.device {
        return;
    }
    // 只更新已通告过的站点。
    match inner.store.get_station(&info.station) {
        Ok(Some(old)) if old.host == peer.device => {}
        _ => return,
    }
    let info = StationInfo {
        name: info.name.chars().take(64).collect(),
        ..info
    };
    if inner.store.upsert_station(&info).is_err() {
        return;
    }
    apply_items(inner, peer.device, &info.station, items);
}

fn on_delta(inner: &Arc<Inner>, peer: &PeerInfo, station: ConvId, items: Vec<StationItem>) {
    if !is_own(inner, peer) {
        return;
    }
    match inner.store.get_station(&station) {
        Ok(Some(old)) if old.host == peer.device => {}
        _ => return,
    }
    apply_items(inner, peer.device, &station, items);
}

/// 镜像主机发来的条目：按 updated_at 取新者（迟到的旧版本和墓碑都不会倒退），
/// 并记录主机为文件的持有者。
fn apply_items(inner: &Arc<Inner>, host: DeviceId, station: &ConvId, items: Vec<StationItem>) {
    let now = now_millis();
    let mut newest = inner.store.synced_until(station).unwrap_or(0);
    for item in items.into_iter().take(MAX_ITEMS_PER_MSG) {
        if !content_ok(&item.content) {
            continue;
        }
        match inner.store.get_station_item(&item.item) {
            Ok(Some(old)) if old.station != *station || old.item.updated_at > item.updated_at => {
                continue
            }
            Err(_) => continue,
            _ => {}
        }
        newest = newest.max(item.updated_at);
        let row = StationItemRow {
            item,
            station: *station,
            store_mode: None,
            local_path: None,
            local_size: None,
            local_mtime: None,
        };
        if let Err(e) = inner.store.upsert_station_item(&row) {
            warn!(error = %e, "storing station item failed");
            continue;
        }
        if !row.item.removed {
            for (meta, _) in content_metas(&row.item.content) {
                let _ = inner.store.add_holder(&meta.hash, &host, now);
            }
        }
    }
    let _ = inner.store.set_synced_until(station, newest);
    station_updated(inner, station);
}

fn content_metas(content: &StationContent) -> Vec<(&BlobMeta, Option<&String>)> {
    match content {
        StationContent::Text { .. } => Vec::new(),
        StationContent::File { blob } => vec![(blob, None)],
        StationContent::Folder { entries, .. } => entries
            .iter()
            .map(|e| (&e.blob, Some(&e.rel_path)))
            .collect(),
    }
}

fn content_ok(content: &StationContent) -> bool {
    match content {
        StationContent::Text { text } => text.len() <= MAX_TEXT_BYTES,
        StationContent::File { .. } => true,
        StationContent::Folder { entries, .. } => {
            !entries.is_empty() && entries.len() <= MAX_FOLDER_ENTRIES
        }
    }
}

// ---- 主机端 ----

async fn on_subscribe(inner: &Arc<Inner>, peer: &PeerInfo, station: ConvId, since: i64) {
    let info = match owned_station(inner, peer, &station) {
        Ok(i) => i,
        Err(e) => {
            reply(inner, &peer.device, 0, Err(e)).await;
            return;
        }
    };
    let store = inner.store.clone();
    let rows =
        tokio::task::spawn_blocking(move || store.station_items(&station, since, true)).await;
    let items = match rows {
        Ok(Ok(rows)) => rows.into_iter().map(|r| r.item).collect(),
        _ => return,
    };
    send_station(
        inner,
        &peer.device,
        StationMsg::Snapshot {
            info,
            items,
            now: now_millis(),
        },
    )
    .await;
}

async fn on_put(
    inner: &Arc<Inner>,
    peer: &PeerInfo,
    station: ConvId,
    req: u64,
    item: StationItem,
    mode_copy: bool,
) {
    let r = match owned_station(inner, peer, &station) {
        Ok(_) if !content_ok(&item.content) => Err(CoreError::InvalidArgument(
            "item content out of range".into(),
        )),
        Ok(_) => inner
            .station
            .accept_remote(peer, &station, item, mode_copy, now_millis()),
        Err(e) => Err(e),
    };
    match r {
        Ok(accepted) => {
            reply(inner, &peer.device, req, Ok(())).await;
            // 复制模式且内容尚未拉取：后台向放入者拉取。
            if !accepted.available && !accepted.removed {
                spawn_pull(inner.clone(), station, accepted.item);
            }
        }
        Err(e) => reply(inner, &peer.device, req, Err(e)).await,
    }
}

/// 后台拉取一个复制模式条目（同一条目同时只拉一个）。
pub(crate) fn spawn_pull(inner: Arc<Inner>, station: ConvId, item: ItemId) {
    {
        let mut set = inner
            .station_client
            .pulling
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !set.insert(item) {
            return;
        }
    }
    tokio::spawn(async move {
        let r = pull_item(&inner, &station, &item).await;
        inner
            .station_client
            .pulling
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&item);
        match r {
            Ok(()) => {}
            Err(e) => warn!(item = %item, error = %e, "pulling station item failed"),
        }
    });
}

/// 重试拉取失败或中断的复制条目（周期任务调用）。
pub(crate) fn retry_pulls(inner: &Arc<Inner>) {
    for info in hosted_stations(inner) {
        let rows = inner
            .store
            .station_items(&info.station, 0, false)
            .unwrap_or_default();
        for row in rows {
            if row.store_mode == Some(crate::store::StoreModeRow::Copy) && !row.item.available {
                spawn_pull(inner.clone(), info.station, row.item.item);
            }
        }
    }
}

async fn pull_item(inner: &Arc<Inner>, station: &ConvId, item: &ItemId) -> Result<(), CoreError> {
    let Some(row) = inner.store.get_station_item(item)? else {
        return Ok(());
    };
    if row.item.removed || row.item.available {
        return Ok(());
    }
    let dir = item_dir(&inner.data_dir, station, item);
    for (meta, rel) in copy_layout(&row.item.content) {
        let dest = dir.join(rel);
        let (Some(parent), Some(name)) = (dest.parent(), dest.file_name()) else {
            return Err(CoreError::InvalidArgument("bad copy destination".into()));
        };
        if !dest.starts_with(&dir) {
            return Err(CoreError::InvalidArgument(
                "copy destination escapes the item directory".into(),
            ));
        }
        if std::fs::metadata(&dest).is_ok_and(|m| m.is_file() && m.len() == meta.size) {
            continue;
        }
        let mut meta = meta;
        meta.name = name.to_string_lossy().into_owned();
        let id = inner
            .transfer
            .clone()
            .download_to(meta.clone(), parent)
            .await?;
        wait_transfer(inner, &id).await?;
        // 并入了同哈希的其他传输时，目标位置可能没有文件：从本地行补一份。
        if !dest.exists() {
            if let Some(local) = inner.store.get_local(&meta.hash)? {
                let (from, to) = (local.path, dest.clone());
                tokio::task::spawn_blocking(move || {
                    if let Some(p) = to.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    std::fs::copy(from, to).map(|_| ())
                })
                .await
                .map_err(|e| CoreError::Proto(format!("copy task failed: {e}")))??;
            }
        }
    }
    // 拉取期间条目可能被删除：墓碑优先。
    match inner.store.get_station_item(item)? {
        Some(r) if !r.item.removed => inner.station.mark_available(item, &dir, now_millis()),
        _ => Ok(()),
    }
}

async fn wait_transfer(inner: &Inner, id: &str) -> Result<(), CoreError> {
    loop {
        match inner.store.get_transfer(id)? {
            Some(r) if r.state == TransferState::Done => return Ok(()),
            Some(r) if matches!(r.state, TransferState::Failed | TransferState::Cancelled) => {
                return Err(CoreError::Rejected(
                    r.error.unwrap_or_else(|| "transfer failed".into()),
                ))
            }
            None => return Err(CoreError::NotFound(id.to_string())),
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

// ---- 拉取端：经主机代理写入 ----

/// 站点的主机设备；主机未连接 → PeerOffline。
fn connected_host(inner: &Inner, station: &ConvId) -> Result<DeviceId, CoreError> {
    let info = inner
        .store
        .get_station(station)?
        .ok_or_else(|| CoreError::NotFound(station.to_hex()))?;
    if !inner.conns.is_connected(&info.host) {
        return Err(CoreError::PeerOffline);
    }
    Ok(info.host)
}

/// 发一个带请求编号的帧并等待主机应答（最多 30 秒）。
async fn call_host(
    inner: &Arc<Inner>,
    host: &DeviceId,
    build: impl FnOnce(u64) -> StationMsg,
) -> Result<(), CoreError> {
    let (req, rx) = inner.station_client.register();
    if let Err(e) = inner.conns.send(host, Frame::Station(build(req))).await {
        inner.station_client.forget(req);
        return Err(e);
    }
    match tokio::time::timeout(RESULT_TIMEOUT, rx).await {
        Ok(Ok((true, _))) => Ok(()),
        Ok(Ok((false, error))) => {
            Err(CoreError::Rejected(error.unwrap_or_else(|| {
                "request rejected by station host".into()
            })))
        }
        Ok(Err(_)) => Err(CoreError::PeerOffline),
        Err(_) => {
            inner.station_client.forget(req);
            Err(CoreError::Rejected("station host did not respond".into()))
        }
    }
}

/// 远程站点放入：路径先登记为本机来源，由主机拉取；文字与已知 Blob 直接发元数据。
pub(crate) async fn remote_put(
    inner: &Arc<Inner>,
    station: &ConvId,
    item: StationPutItem,
    expires_at: Option<i64>,
) -> Result<(), CoreError> {
    let host = connected_host(inner, station)?;
    let me = self_device(inner);
    let now = now_millis();
    if let Some(t) = expires_at {
        if t <= now {
            return Err(CoreError::InvalidArgument(
                "expiry must be later than now".into(),
            ));
        }
    }
    let mk = |content: StationContent| StationItem {
        item: ItemId::random(),
        added_by: me,
        added_at: now,
        expires_at,
        content,
        pinned: false,
        removed: false,
        available: true,
        updated_at: now,
    };
    let mut puts: Vec<(StationItem, bool)> = Vec::new();
    match item {
        StationPutItem::Text { text } => {
            if text.len() > MAX_TEXT_BYTES {
                return Err(CoreError::InvalidArgument("text is too long".into()));
            }
            puts.push((mk(StationContent::Text { text }), true));
        }
        StationPutItem::Blobs { hashes } => {
            for meta in crate::station::host::resolve_blob_metas(&inner.store, &hashes)? {
                puts.push((mk(StationContent::File { blob: meta }), false));
            }
        }
        StationPutItem::Paths { paths } => {
            if paths.is_empty() {
                return Err(CoreError::InvalidArgument("path list is empty".into()));
            }
            for p in paths {
                let content = register_path(inner, std::path::PathBuf::from(p), me, now).await?;
                puts.push((mk(content), true));
            }
        }
    }
    for (item, mode_copy) in puts {
        let station = *station;
        call_host(inner, &host, move |req| StationMsg::Put {
            station,
            req,
            item,
            mode_copy,
        })
        .await?;
    }
    Ok(())
}

/// 把本机文件或文件夹登记为来源 Blob，得到条目内容。
async fn register_path(
    inner: &Arc<Inner>,
    path: std::path::PathBuf,
    me: DeviceId,
    now: i64,
) -> Result<StationContent, CoreError> {
    let (abs, is_dir, name) = resolve_path(path).await?;
    let reg = |p: std::path::PathBuf| {
        register_local_file(
            inner.store.clone(),
            me,
            inner.platform.thumbnailer.clone(),
            p,
            BlobLocalKind::Source,
            now,
            false,
        )
    };
    if !is_dir {
        return Ok(StationContent::File {
            blob: reg(abs).await?,
        });
    }
    let root = abs.clone();
    let (_, files) =
        tokio::task::spawn_blocking(move || crate::blob::collect_folder_sources(&root))
            .await
            .map_err(|e| CoreError::Proto(format!("walk task failed: {e}")))??;
    if files.is_empty() {
        return Err(CoreError::InvalidArgument("folder is empty".into()));
    }
    let mut entries = Vec::with_capacity(files.len());
    for f in files {
        let Some(rel_path) = f.rel_path else { continue };
        entries.push(FolderEntry {
            rel_path,
            blob: reg(f.path).await?,
        });
    }
    Ok(StationContent::Folder { name, entries })
}

pub(crate) async fn remote_remove(
    inner: &Arc<Inner>,
    station: &ConvId,
    item: ItemId,
) -> Result<(), CoreError> {
    let host = connected_host(inner, station)?;
    let station = *station;
    call_host(inner, &host, move |req| StationMsg::Remove {
        station,
        req,
        item,
    })
    .await
}

pub(crate) async fn remote_set_pinned(
    inner: &Arc<Inner>,
    station: &ConvId,
    item: ItemId,
    pinned: bool,
) -> Result<(), CoreError> {
    let host = connected_host(inner, station)?;
    let station = *station;
    call_host(inner, &host, move |req| StationMsg::SetPinned {
        station,
        req,
        item,
        pinned,
    })
    .await
}
