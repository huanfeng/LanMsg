// 中转站主机端：站点创建、条目放入（引用 / 复制）、删除、置顶、过期与
// 引用文件可用性巡检。所有文件读写走 spawn_blocking + std::fs。
// 注释中文，标识符英文。

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use lanmsg_proto::consts::MAX_FOLDER_ENTRIES;
use lanmsg_proto::ids::{BlobHash, ConvId, DeviceId};
use lanmsg_proto::station::{ItemId, StationAcl, StationContent, StationInfo, StationItem};
use lanmsg_proto::types::{BlobMeta, DeviceType, FolderEntry};
use tokio::sync::broadcast;
use tracing::warn;

use crate::blob::hash::file_mtime;
use crate::blob::register::register_local_file;
use crate::blob::transfer::sanitize_component;
use crate::error::CoreError;
use crate::identity::Identity;
use crate::net::PeerInfo;
use crate::platform::ThumbnailProvider;
use crate::store::{
    BlobLocalKind, BlobLocalRow, ConvKindRow, ConvRow, StationItemRow, Store, StoreModeRow,
};

/// 站点名称最大字符数。
const MAX_NAME_CHARS: usize = 64;
/// 文字条目最大字节数。
const MAX_TEXT_BYTES: usize = 64 * 1024;
/// 复制模式在数据目录下的根目录名（磁盘目录名保持英文，与 `My devices` 同一处理）。
pub const STATION_DIR: &str = "Station";

/// 复制模式条目的存放目录：`<data_dir>/Station/<station hex>/<item hex>`。
pub fn item_dir(data_dir: &Path, station: &ConvId, item: &ItemId) -> PathBuf {
    data_dir
        .join(STATION_DIR)
        .join(station.to_hex())
        .join(item.to_hex())
}

/// 中转站主机端。
pub struct StationHost {
    store: Arc<Store>,
    identity: Arc<RwLock<Identity>>,
    data_dir: PathBuf,
    thumbnailer: Option<Arc<dyn ThumbnailProvider>>,
    changes: broadcast::Sender<(ConvId, Vec<StationItem>)>,
}

impl StationHost {
    pub fn new(
        store: Arc<Store>,
        identity: Arc<RwLock<Identity>>,
        data_dir: PathBuf,
        thumbnailer: Option<Arc<dyn ThumbnailProvider>>,
    ) -> StationHost {
        let (changes, _) = broadcast::channel(64);
        StationHost {
            store,
            identity,
            data_dir,
            thumbnailer,
            changes,
        }
    }

    /// 订阅条目变化（站点, 变化的条目）。
    pub fn subscribe_changes(&self) -> broadcast::Receiver<(ConvId, Vec<StationItem>)> {
        self.changes.subscribe()
    }

    pub(crate) fn self_device(&self) -> DeviceId {
        self.identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .device_id()
    }

    fn notify(&self, station: &ConvId, items: Vec<StationItem>) {
        if !items.is_empty() {
            // 无订阅者时 send 返回 Err，属正常情况。
            let _ = self.changes.send((*station, items));
        }
    }

    /// 取本机托管的站点；不存在、已删除或不在本机 → InvalidArgument。
    pub(crate) fn hosted(&self, station: &ConvId) -> Result<StationInfo, CoreError> {
        match self.store.get_station(station)? {
            Some(info) if !info.deleted && info.host == self.self_device() => Ok(info),
            _ => Err(CoreError::InvalidArgument(
                "station is not hosted on this device".into(),
            )),
        }
    }

    /// 创建站点。只有桌面、笔记本、无界面设备能当主机。
    pub fn create(&self, name: &str, now: i64) -> Result<StationInfo, CoreError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(CoreError::InvalidArgument(
                "station name must be 1 to 64 characters".into(),
            ));
        }
        let (device, owner, device_type) = {
            let id = self.identity.read().unwrap_or_else(|e| e.into_inner());
            let owner = id.user_id().ok_or(CoreError::NotInitialized)?;
            let device_type = id.cert.as_ref().map(|c| c.device_type);
            (id.device_id(), owner, device_type)
        };
        if !matches!(
            device_type,
            Some(DeviceType::Desktop | DeviceType::Laptop | DeviceType::Headless)
        ) {
            return Err(CoreError::InvalidArgument(
                "this device type cannot host a station".into(),
            ));
        }
        let info = StationInfo {
            station: ConvId::random(),
            name: name.to_string(),
            host: device,
            owner,
            acl: StationAcl::OwnerOnly,
            created_at: now,
            deleted: false,
        };
        self.store.upsert_station(&info)?;
        self.store.ensure_conversation(&ConvRow {
            conv: info.station,
            kind: ConvKindRow::Station,
            peer_user: None,
            title: Some(info.name.clone()),
            created_at: now,
        })?;
        Ok(info)
    }

    fn check_expiry(expires_at: Option<i64>, now: i64) -> Result<(), CoreError> {
        match expires_at {
            Some(t) if t <= now => Err(CoreError::InvalidArgument(
                "expiry must be later than now".into(),
            )),
            _ => Ok(()),
        }
    }

    fn new_row(
        station: &ConvId,
        content: StationContent,
        expires_at: Option<i64>,
        added_by: DeviceId,
        now: i64,
    ) -> StationItemRow {
        StationItemRow {
            item: StationItem {
                item: ItemId::random(),
                added_by,
                added_at: now,
                expires_at,
                content,
                pinned: false,
                removed: false,
                available: true,
                updated_at: now,
            },
            station: *station,
            store_mode: None,
            local_path: None,
            local_size: None,
            local_mtime: None,
        }
    }

    /// 放入本机路径（文件或文件夹），每个路径一个条目。
    pub async fn put_paths(
        &self,
        station: &ConvId,
        paths: &[PathBuf],
        mode: StoreModeRow,
        expires_at: Option<i64>,
        added_by: DeviceId,
        now: i64,
    ) -> Result<Vec<StationItem>, CoreError> {
        self.hosted(station)?;
        Self::check_expiry(expires_at, now)?;
        if paths.is_empty() {
            return Err(CoreError::InvalidArgument("path list is empty".into()));
        }
        // 先全部登记完再一次性写库：中途失败时清理已复制的副本，不留半截结果。
        let mut rows = Vec::with_capacity(paths.len());
        for path in paths {
            match self
                .put_one_path(station, path, mode, expires_at, added_by, now)
                .await
            {
                Ok(row) => rows.push(row),
                Err(e) => {
                    for r in &rows {
                        if r.store_mode == Some(StoreModeRow::Copy) {
                            remove_dir_blocking(item_dir(&self.data_dir, station, &r.item.item))
                                .await;
                        }
                    }
                    return Err(e);
                }
            }
        }
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            if let Err(e) = self.store.upsert_station_item(row) {
                self.notify(station, out);
                return Err(e.into());
            }
            out.push(row.item.clone());
        }
        self.notify(station, out.clone());
        Ok(out)
    }

    async fn put_one_path(
        &self,
        station: &ConvId,
        path: &Path,
        mode: StoreModeRow,
        expires_at: Option<i64>,
        added_by: DeviceId,
        now: i64,
    ) -> Result<StationItemRow, CoreError> {
        let (abs, is_dir, name) = resolve_path(path.to_path_buf()).await?;

        let item_id = ItemId::random();
        let dir = item_dir(&self.data_dir, station, &item_id);
        let kind = match mode {
            StoreModeRow::Reference => BlobLocalKind::Source,
            StoreModeRow::Copy => BlobLocalKind::Downloaded,
        };
        let self_device = self.self_device();

        // 逐文件登记；失败时清理已复制的副本。
        let result = async {
            // (原文件, rel_path)；File 条目 rel_path 为 None。
            let sources: Vec<(PathBuf, Option<String>)> = if is_dir {
                let root = abs.clone();
                let (_, files) =
                    tokio::task::spawn_blocking(move || crate::blob::collect_folder_sources(&root))
                        .await
                        .map_err(|e| CoreError::Proto(format!("walk task failed: {e}")))??;
                if files.is_empty() {
                    return Err(CoreError::InvalidArgument("folder is empty".into()));
                }
                files.into_iter().map(|f| (f.path, f.rel_path)).collect()
            } else {
                vec![(abs.clone(), None)]
            };

            let mut entries = Vec::new();
            let mut total: u64 = 0;
            let mut max_mtime: i64 = 0;
            let mut single: Option<BlobMeta> = None;
            for (src, rel) in sources {
                let target = match mode {
                    StoreModeRow::Reference => src.clone(),
                    StoreModeRow::Copy => {
                        let base = dir.join(sanitize_component(&name));
                        let dest = match &rel {
                            Some(r) => join_sanitized(base, r),
                            None => base,
                        };
                        // 防御：副本必须落在条目目录内且不能覆盖源文件。
                        if dest == src || !dest.starts_with(&dir) {
                            return Err(CoreError::InvalidArgument(
                                "copy destination is not inside the item directory".into(),
                            ));
                        }
                        let (from, to) = (src.clone(), dest.clone());
                        tokio::task::spawn_blocking(move || {
                            if let Some(parent) = to.parent() {
                                std::fs::create_dir_all(parent)?;
                            }
                            std::fs::copy(&from, &to).map(|_| ())
                        })
                        .await
                        .map_err(|e| CoreError::Proto(format!("copy task failed: {e}")))??;
                        dest
                    }
                };
                let meta = register_local_file(
                    self.store.clone(),
                    self_device,
                    self.thumbnailer.clone(),
                    target,
                    kind,
                    now,
                    // 复制登记不抢走仍可读的既有本地行（例如消息附件）。
                    mode == StoreModeRow::Copy,
                )
                .await?;
                // 登记刚写入的本地行就是这次哈希时的 (size, mtime)。
                if let Some(local) = self.store.get_local(&meta.hash)? {
                    total += local.size;
                    max_mtime = max_mtime.max(local.mtime);
                }
                match rel {
                    Some(rel_path) => entries.push(FolderEntry {
                        rel_path,
                        blob: meta,
                    }),
                    None => single = Some(meta),
                }
            }
            let content = match single {
                Some(blob) => StationContent::File { blob },
                None => StationContent::Folder { name, entries },
            };
            Ok((content, total, max_mtime))
        }
        .await;

        let (content, total, max_mtime) = match result {
            Ok(v) => v,
            Err(e) => {
                if mode == StoreModeRow::Copy {
                    remove_dir_blocking(dir).await;
                }
                return Err(e);
            }
        };
        let mut row = Self::new_row(station, content, expires_at, added_by, now);
        row.item.item = item_id;
        row.store_mode = Some(mode);
        if mode == StoreModeRow::Reference {
            row.local_path = Some(abs);
            row.local_size = Some(total);
            row.local_mtime = Some(max_mtime);
        } else {
            row.local_path = Some(dir);
        }
        Ok(row)
    }

    /// 放入文字。
    pub fn put_text(
        &self,
        station: &ConvId,
        text: &str,
        expires_at: Option<i64>,
        added_by: DeviceId,
        now: i64,
    ) -> Result<StationItem, CoreError> {
        self.hosted(station)?;
        Self::check_expiry(expires_at, now)?;
        if text.len() > MAX_TEXT_BYTES {
            return Err(CoreError::InvalidArgument("text is too long".into()));
        }
        let row = Self::new_row(
            station,
            StationContent::Text {
                text: text.to_string(),
            },
            expires_at,
            added_by,
            now,
        );
        self.store.upsert_station_item(&row)?;
        self.notify(station, vec![row.item.clone()]);
        Ok(row.item)
    }

    /// 放入已知 Blob（不复制、不下载，内容由已知持有者提供）。
    pub fn put_blobs(
        &self,
        station: &ConvId,
        metas: Vec<BlobMeta>,
        expires_at: Option<i64>,
        added_by: DeviceId,
        now: i64,
    ) -> Result<Vec<StationItem>, CoreError> {
        self.hosted(station)?;
        Self::check_expiry(expires_at, now)?;
        if metas.is_empty() {
            return Err(CoreError::InvalidArgument("blob list is empty".into()));
        }
        let mut out = Vec::with_capacity(metas.len());
        for blob in metas {
            let row = Self::new_row(
                station,
                StationContent::File { blob },
                expires_at,
                added_by,
                now,
            );
            self.store.upsert_station_item(&row)?;
            out.push(row.item);
        }
        self.notify(station, out.clone());
        Ok(out)
    }

    /// 接收自己另一台设备经连接发来的条目；`mode_copy` 为 true 时本机需要拉取内容。
    pub fn accept_remote(
        &self,
        from: &PeerInfo,
        station: &ConvId,
        item: StationItem,
        mode_copy: bool,
        now: i64,
    ) -> Result<StationItem, CoreError> {
        let info = self.hosted(station)?;
        let from_user = from.cert.as_ref().map(|c| c.user);
        if from_user != Some(info.owner) {
            return Err(CoreError::Rejected("access denied".into()));
        }
        Self::check_expiry(item.expires_at, now)?;
        let existing = self.store.get_station_item(&item.item)?;
        if let Some(existing) = &existing {
            if existing.station != *station {
                return Err(CoreError::InvalidArgument(
                    "item belongs to another station".into(),
                ));
            }
            // 墓碑：已删除的条目不因迟到的旧投递复活。
            if existing.item.removed {
                return Ok(existing.item.clone());
            }
        }
        let blobs: Vec<BlobHash> = match &item.content {
            StationContent::Text { text } => {
                if text.len() > MAX_TEXT_BYTES {
                    return Err(CoreError::InvalidArgument("text is too long".into()));
                }
                Vec::new()
            }
            StationContent::File { blob } => vec![blob.hash],
            StationContent::Folder { entries, .. } => {
                if entries.is_empty() || entries.len() > MAX_FOLDER_ENTRIES {
                    return Err(CoreError::InvalidArgument(
                        "folder entry count out of range".into(),
                    ));
                }
                entries.iter().map(|e| e.blob.hash).collect()
            }
        };
        let is_text = matches!(item.content, StationContent::Text { .. });
        let copy = mode_copy && !is_text;
        let mut row = Self::new_row(station, item.content, item.expires_at, from.device, now);
        row.item.item = item.item;
        row.item.pinned = item.pinned;
        row.item.available = !copy;
        row.store_mode = copy.then_some(StoreModeRow::Copy);
        // 重复投递：已拉取完成的复制条目保留本地状态，避免退回「待拉取」。
        if let Some(old) = existing.filter(|o| {
            copy && !o.item.removed
                && o.store_mode == Some(StoreModeRow::Copy)
                && o.item.available
                && o.item.content == row.item.content
        }) {
            row.item.available = true;
            row.item.added_at = old.item.added_at;
            row.local_path = old.local_path;
            row.local_size = old.local_size;
            row.local_mtime = old.local_mtime;
        }
        self.store.upsert_station_item(&row)?;
        if copy {
            for hash in &blobs {
                self.store.add_holder(hash, &from.device, now)?;
            }
        }
        self.notify(station, vec![row.item.clone()]);
        Ok(row.item)
    }

    /// 复制模式条目拉取完成：记录本地目录并标为可用。
    pub fn mark_available(
        &self,
        item: &ItemId,
        local_path: &Path,
        now: i64,
    ) -> Result<(), CoreError> {
        let mut row = self
            .store
            .get_station_item(item)?
            .ok_or_else(|| CoreError::NotFound(item.to_hex()))?;
        row.item.available = true;
        row.item.updated_at = now;
        row.local_path = Some(local_path.to_path_buf());
        self.store.upsert_station_item(&row)?;
        self.notify(&row.station, vec![row.item]);
        Ok(())
    }

    /// 标记条目已删除；复制模式同时删除副本目录。
    pub async fn remove(&self, station: &ConvId, item: &ItemId, now: i64) -> Result<(), CoreError> {
        self.hosted(station)?;
        let mut row = self
            .store
            .get_station_item(item)?
            .filter(|r| r.station == *station)
            .ok_or_else(|| CoreError::NotFound(item.to_hex()))?;
        let dir = self.remove_row(&mut row, now)?;
        self.notify(station, vec![row.item]);
        if let Some(dir) = dir {
            remove_dir_blocking(dir).await;
        }
        Ok(())
    }

    /// 写库标记删除并整理本地行，返回待删除的副本目录（由调用方在阻塞线程里删）。
    fn remove_row(&self, row: &mut StationItemRow, now: i64) -> Result<Option<PathBuf>, CoreError> {
        row.item.removed = true;
        row.item.updated_at = now;
        self.store.upsert_station_item(row)?;
        if row.store_mode != Some(StoreModeRow::Copy) {
            return Ok(None);
        }
        let dir = item_dir(&self.data_dir, &row.station, &row.item.item);
        // 本地行指向即将删除的副本时，改由仍存活的其他条目接手（否则同内容的
        // 引用条目会因本地行悬空而无法下载）。
        for (meta, _) in content_blobs(&row.item.content) {
            let Some(local) = self.store.get_local(&meta.hash)? else {
                continue;
            };
            if !local.path.starts_with(&dir) {
                continue;
            }
            self.store.remove_local(&meta.hash)?;
            for other in self.store.station_items_with_blob(&meta.hash)? {
                if other.item.available {
                    self.restore_local(&other)?;
                }
            }
        }
        Ok(Some(dir))
    }

    /// 本地行缺失时按条目记录补登记。大小取元数据、mtime 记 0，
    /// 让数据通道首次服务前重新哈希校验，不信任条目记录。
    fn restore_local(&self, row: &StationItemRow) -> Result<(), CoreError> {
        let Some(root) = &row.local_path else {
            return Ok(());
        };
        let kind = match row.store_mode {
            Some(StoreModeRow::Reference) => BlobLocalKind::Source,
            Some(StoreModeRow::Copy) => BlobLocalKind::Downloaded,
            None => return Ok(()),
        };
        let copy_paths = copy_layout(&row.item.content);
        for (i, (meta, rel)) in content_blobs(&row.item.content).into_iter().enumerate() {
            if self.store.get_local(&meta.hash)?.is_some() {
                continue;
            }
            let path = match row.store_mode {
                Some(StoreModeRow::Reference) => match rel {
                    Some(rel) => root.join(rel),
                    None => root.clone(),
                },
                _ => match copy_paths.get(i) {
                    Some((_, p)) => root.join(p),
                    None => continue,
                },
            };
            if !std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == meta.size) {
                continue;
            }
            self.store.set_local(&BlobLocalRow {
                hash: meta.hash,
                path,
                kind,
                size: meta.size,
                mtime: 0,
            })?;
        }
        Ok(())
    }

    /// 置顶 / 取消置顶。
    pub fn set_pinned(
        &self,
        station: &ConvId,
        item: &ItemId,
        pinned: bool,
        now: i64,
    ) -> Result<(), CoreError> {
        self.hosted(station)?;
        let mut row = self
            .store
            .get_station_item(item)?
            .filter(|r| r.station == *station)
            .ok_or_else(|| CoreError::NotFound(item.to_hex()))?;
        row.item.pinned = pinned;
        row.item.updated_at = now;
        self.store.upsert_station_item(&row)?;
        self.notify(station, vec![row.item]);
        Ok(())
    }

    /// 巡检：到期条目按删除处理；引用模式条目的原文件被移动 / 删除 / 修改时
    /// 标为不可用（恢复一致后重新可用，并补回被清掉的本地行）。返回变化条数。
    /// 阻塞函数，调用方在 spawn_blocking 中执行。
    pub fn sweep(&self, now: i64) -> Result<usize, CoreError> {
        let mut changed: std::collections::HashMap<ConvId, Vec<StationItem>> = Default::default();
        let mut count = 0;
        for mut row in self.store.station_items_expiring(now)? {
            if let Some(dir) = self.remove_row(&mut row, now)? {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        warn!(error = %e, dir = %dir.display(), "removing station copy failed");
                    }
                }
            }
            changed.entry(row.station).or_default().push(row.item);
            count += 1;
        }
        for mut row in self.store.station_reference_items()? {
            let ok = reference_unchanged(&row);
            if ok {
                self.restore_local(&row)?;
            }
            if ok == row.item.available {
                continue;
            }
            row.item.available = ok;
            row.item.updated_at = now;
            self.store.upsert_station_item(&row)?;
            changed.entry(row.station).or_default().push(row.item);
            count += 1;
        }
        for (station, items) in changed {
            self.notify(&station, items);
        }
        self.sweep_orphan_dirs();
        Ok(count)
    }

    /// 清理没有条目（或条目已删除）的复制目录，例如放入中途失败留下的副本。
    /// 一小时内改动过的目录不动，避免误删正在写入的副本。
    fn sweep_orphan_dirs(&self) {
        let root = self.data_dir.join(STATION_DIR);
        let Ok(stations) = std::fs::read_dir(&root) else {
            return;
        };
        let min_age = std::time::Duration::from_secs(3600);
        for station in stations.flatten() {
            let Ok(items) = std::fs::read_dir(station.path()) else {
                continue;
            };
            for dir in items.flatten() {
                let Some(id) = dir
                    .file_name()
                    .to_str()
                    .and_then(|n| ItemId::from_hex(n).ok())
                else {
                    continue;
                };
                let live = matches!(
                    self.store.get_station_item(&id),
                    Ok(Some(r)) if !r.item.removed
                );
                let old = dir
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age >= min_age);
                if !live && old {
                    if let Err(e) = std::fs::remove_dir_all(dir.path()) {
                        warn!(error = %e, dir = %dir.path().display(), "removing orphan station copy failed");
                    }
                }
            }
        }
    }
}

/// 在阻塞线程里删除目录，不存在不算错误，其他失败只记警告。
async fn remove_dir_blocking(dir: PathBuf) {
    let d = dir.clone();
    match tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&d)).await {
        Ok(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => {
            warn!(error = %e, dir = %dir.display(), "removing station copy failed");
        }
        Err(e) => warn!(error = %e, "removing station copy task failed"),
        _ => {}
    }
}

/// 规整用户给的路径并取出可用名字，返回 (规整路径, 是否目录, 名字)。
///
/// 先规整（解析 `..` 与符号链接）再取名字：否则以 `..` 或 `/` 结尾的路径取不到
/// 文件名，复制目标会退化成源路径本身。名字为空、`.`、`..` 或含分隔符一律拒绝。
pub(crate) async fn resolve_path(p: PathBuf) -> Result<(PathBuf, bool, String), CoreError> {
    tokio::task::spawn_blocking(move || {
        let abs = std::fs::canonicalize(&p).map_err(|_| {
            CoreError::InvalidArgument(format!("path does not exist: {}", p.display()))
        })?;
        let is_dir = std::fs::metadata(&abs).map_err(CoreError::Io)?.is_dir();
        let name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty() && n != "." && n != ".." && !n.contains(['/', '\\']))
            .ok_or_else(|| {
                CoreError::InvalidArgument(format!("path has no usable name: {}", p.display()))
            })?;
        Ok::<_, CoreError>((abs, is_dir, name))
    })
    .await
    .map_err(|e| CoreError::Proto(format!("path task failed: {e}")))?
}

/// 把相对路径（'/' 分隔）逐段清洗后接到 base 下。清洗同时消除 `..`、盘符、反斜杠。
pub(crate) fn join_sanitized(mut base: PathBuf, rel: &str) -> PathBuf {
    for seg in rel.split('/') {
        base = base.join(sanitize_component(seg));
    }
    base
}

/// 复制模式下条目各 Blob 相对条目目录的落地路径（与 `content_blobs` 同序，逐段清洗）。
pub(crate) fn copy_layout(content: &StationContent) -> Vec<(BlobMeta, PathBuf)> {
    match content {
        StationContent::Text { .. } => Vec::new(),
        StationContent::File { blob } => {
            vec![(blob.clone(), PathBuf::from(sanitize_component(&blob.name)))]
        }
        StationContent::Folder { name, entries } => entries
            .iter()
            .map(|e| {
                (
                    e.blob.clone(),
                    join_sanitized(PathBuf::from(sanitize_component(name)), &e.rel_path),
                )
            })
            .collect(),
    }
}

/// 条目内容引用的 Blob 与 rel_path（File 为 None）。
fn content_blobs(content: &StationContent) -> Vec<(&BlobMeta, Option<&String>)> {
    match content {
        StationContent::Text { .. } => Vec::new(),
        StationContent::File { blob } => vec![(blob, None)],
        StationContent::Folder { entries, .. } => entries
            .iter()
            .map(|e| (&e.blob, Some(&e.rel_path)))
            .collect(),
    }
}

/// 引用模式条目的原文件是否与登记时一致：File 比较 (size, mtime)；
/// Folder 要求每个文件都在且大小一致，并比较大小总和与最大 mtime。
fn reference_unchanged(row: &StationItemRow) -> bool {
    let (Some(path), Some(size), Some(mtime)) = (&row.local_path, row.local_size, row.local_mtime)
    else {
        return false;
    };
    let stat = |p: &Path| std::fs::metadata(p).ok().filter(|m| m.is_file());
    match &row.item.content {
        StationContent::Text { .. } => true,
        StationContent::File { .. } => {
            stat(path).is_some_and(|m| m.len() == size && file_mtime(&m) == mtime)
        }
        StationContent::Folder { entries, .. } => {
            // 登记时文件夹根路径即 local_path；条目路径为根下的相对路径。
            let mut total = 0u64;
            let mut newest = 0i64;
            for e in entries {
                let Some(m) = stat(&path.join(&e.rel_path)) else {
                    return false;
                };
                if m.len() != e.blob.size {
                    return false;
                }
                total += m.len();
                newest = newest.max(file_mtime(&m));
            }
            total == size && newest == mtime
        }
    }
}

/// 把十六进制哈希解析成元数据：先查消息引用，再查中转站条目。
pub(crate) fn resolve_blob_metas(
    store: &Store,
    hashes: &[String],
) -> Result<Vec<BlobMeta>, CoreError> {
    let mut out = Vec::with_capacity(hashes.len());
    for h in hashes {
        let hash = BlobHash::from_hex(h)
            .map_err(|_| CoreError::InvalidArgument(format!("invalid file hash: {h}")))?;
        if let Some(row) = store.message_blobs_by_hash(&hash)?.into_iter().next() {
            out.push(row.meta);
        } else if let Some((_, meta, _)) = store.station_blob(&hash)? {
            out.push(meta);
        } else {
            return Err(CoreError::NotFound(h.clone()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::FileKeyStore;
    use lanmsg_proto::ids::UserId;
    use lanmsg_proto::sign::UserKey;
    use lanmsg_proto::types::{DeviceCert, PathClass};

    struct Fixture {
        host: StationHost,
        store: Arc<Store>,
        me: DeviceId,
        user: UserId,
        station: ConvId,
        data: tempfile::TempDir,
        files: tempfile::TempDir,
        _keys: tempfile::TempDir,
    }

    fn identity(store: &Arc<Store>, keys: &Path, device_type: DeviceType) -> Arc<RwLock<Identity>> {
        let ks = FileKeyStore::new(keys.join("keys"));
        let mut id = Identity::load_or_create(&ks, store).unwrap();
        id.init_first_device(&ks, store, "dev", device_type, 1)
            .unwrap();
        Arc::new(RwLock::new(id))
    }

    fn fixture() -> Fixture {
        let keys = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let id = identity(&store, keys.path(), DeviceType::Desktop);
        let (me, user) = {
            let g = id.read().unwrap();
            (g.device_id(), g.user_id().unwrap())
        };
        let host = StationHost::new(store.clone(), id, data.path().to_path_buf(), None);
        let station = host.create("box", 1_000).unwrap().station;
        Fixture {
            host,
            store,
            me,
            user,
            station,
            data,
            files,
            _keys: keys,
        }
    }

    fn peer(user_key: &UserKey, device: DeviceId) -> PeerInfo {
        PeerInfo {
            device,
            cert: Some(DeviceCert::issue(
                user_key,
                device,
                "p".into(),
                DeviceType::Desktop,
                1,
            )),
            display_name: "p".into(),
            addr: "127.0.0.1:1".parse().unwrap(),
            dial_addr: "127.0.0.1:1".parse().unwrap(),
            path: PathClass::Lan,
            caps: Vec::new(),
            outbound: false,
            tls_exporter: [0u8; 32],
        }
    }

    fn write(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
        p
    }

    fn file_blob(item: &StationItem) -> &BlobMeta {
        match &item.content {
            StationContent::File { blob } => blob,
            other => panic!("expected file content: {other:?}"),
        }
    }

    #[test]
    fn create_validates_name_and_device_type() {
        let f = fixture();
        assert!(matches!(
            f.host.create("   ", 1),
            Err(CoreError::InvalidArgument(_))
        ));
        assert!(matches!(
            f.host.create(&"x".repeat(65), 1),
            Err(CoreError::InvalidArgument(_))
        ));
        let conv = f.store.get_conversation(&f.station).unwrap().unwrap();
        assert_eq!(conv.kind, ConvKindRow::Station);
        let info = f.store.get_station(&f.station).unwrap().unwrap();
        assert_eq!((info.host, info.owner), (f.me, f.user));

        for (t, ok) in [
            (DeviceType::Laptop, true),
            (DeviceType::Headless, true),
            (DeviceType::Phone, false),
            (DeviceType::Tablet, false),
            (DeviceType::Web, false),
        ] {
            let keys = tempfile::tempdir().unwrap();
            let store = Arc::new(Store::open_in_memory().unwrap());
            let id = identity(&store, keys.path(), t);
            let host = StationHost::new(store, id, keys.path().to_path_buf(), None);
            let r = host.create("box", 1);
            if ok {
                assert!(r.is_ok(), "{t:?}");
            } else {
                assert!(matches!(r, Err(CoreError::InvalidArgument(_))), "{t:?}");
            }
        }
    }

    #[tokio::test]
    async fn put_text_and_file_then_list() {
        let f = fixture();
        let path = write(f.files.path(), "a.txt", b"hello");
        f.host
            .put_text(&f.station, "hi", None, f.me, 2_000)
            .unwrap();
        let items = f
            .host
            .put_paths(
                &f.station,
                &[path],
                StoreModeRow::Reference,
                None,
                f.me,
                2_001,
            )
            .await
            .unwrap();
        let rows = f.store.station_items(&f.station, 0, false).unwrap();
        assert_eq!(rows.len(), 2);
        let hash = file_blob(&items[0]).hash;
        let (station, meta, rel) = f.store.station_blob(&hash).unwrap().unwrap();
        assert_eq!(
            (station, meta.name.as_str(), rel),
            (f.station, "a.txt", None)
        );
        // 引用模式：原路径登记为 Source，本机是持有者。
        assert_eq!(
            f.store.get_local(&hash).unwrap().unwrap().kind,
            BlobLocalKind::Source
        );
        assert_eq!(f.store.holders(&hash).unwrap(), vec![f.me]);
    }

    #[tokio::test]
    async fn folder_with_identical_files_can_be_added() {
        let f = fixture();
        let root = f.files.path().join("dir");
        write(&root, "a/same.txt", b"same");
        write(&root, "b/same.txt", b"same");
        write(&root, "c.txt", b"other");
        for mode in [StoreModeRow::Reference, StoreModeRow::Copy] {
            let items = f
                .host
                .put_paths(
                    &f.station,
                    std::slice::from_ref(&root),
                    mode,
                    None,
                    f.me,
                    2_000,
                )
                .await
                .unwrap();
            let StationContent::Folder { name, entries } = &items[0].content else {
                panic!("expected folder");
            };
            assert_eq!(name, "dir");
            let rels: Vec<_> = entries.iter().map(|e| e.rel_path.as_str()).collect();
            assert_eq!(rels, ["a/same.txt", "b/same.txt", "c.txt"]);
            assert_eq!(entries[0].blob.hash, entries[1].blob.hash);
            // 相同内容的两个文件在 station_blobs 里各有一行，按 rel_path 区分。
            let conn = f.store.conn().unwrap();
            let n: i64 = conn
                .query_row(
                    "SELECT count(*) FROM station_blobs WHERE item_id = ?1",
                    [&items[0].item.0[..]],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 3);
        }
    }

    #[tokio::test]
    async fn copy_mode_survives_original_deletion() {
        let f = fixture();
        let path = write(f.files.path(), "a.txt", b"hello");
        let items = f
            .host
            .put_paths(
                &f.station,
                std::slice::from_ref(&path),
                StoreModeRow::Copy,
                None,
                f.me,
                2_000,
            )
            .await
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(f.host.sweep(3_000).unwrap(), 0);
        let row = f.store.get_station_item(&items[0].item).unwrap().unwrap();
        assert!(row.item.available);
        let local = f
            .store
            .get_local(&file_blob(&items[0]).hash)
            .unwrap()
            .unwrap();
        let dir = f.data.path().join("Station");
        assert!(local.path.starts_with(&dir), "{:?}", local.path);
        assert_eq!(std::fs::read(&local.path).unwrap(), b"hello");

        // 删除条目时副本目录一并删除。
        f.host
            .remove(&f.station, &items[0].item, 4_000)
            .await
            .unwrap();
        assert!(!local.path.exists());
        assert!(f
            .store
            .station_blob(&file_blob(&items[0]).hash)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn reference_item_marked_unavailable_when_file_changes() {
        let f = fixture();
        let path = write(f.files.path(), "a.txt", b"hello");
        let items = f
            .host
            .put_paths(
                &f.station,
                std::slice::from_ref(&path),
                StoreModeRow::Reference,
                None,
                f.me,
                2_000,
            )
            .await
            .unwrap();
        let id = items[0].item;
        let available = |f: &Fixture| {
            f.store
                .get_station_item(&id)
                .unwrap()
                .unwrap()
                .item
                .available
        };
        assert_eq!(f.host.sweep(3_000).unwrap(), 0);
        assert!(available(&f));

        // 内容被改写（大小变化）。
        std::fs::write(&path, b"hello world").unwrap();
        assert_eq!(f.host.sweep(4_000).unwrap(), 1);
        assert!(!available(&f));
        // 再次巡检不重复计数。
        assert_eq!(f.host.sweep(5_000).unwrap(), 0);

        // 改回原内容与原修改时间后恢复可用。
        let mtime = f
            .store
            .get_station_item(&id)
            .unwrap()
            .unwrap()
            .local_mtime
            .unwrap();
        std::fs::write(&path, b"hello").unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_millis(mtime as u64))
            .unwrap();
        drop(file);
        assert_eq!(f.host.sweep(6_000).unwrap(), 1);
        assert!(available(&f));

        // 原文件被删除。
        std::fs::remove_file(&path).unwrap();
        assert_eq!(f.host.sweep(7_000).unwrap(), 1);
        assert!(!available(&f));
    }

    #[tokio::test]
    async fn reference_folder_unavailable_when_member_changes() {
        let f = fixture();
        let root = f.files.path().join("dir");
        write(&root, "a.txt", b"aaa");
        let b = write(&root, "sub/b.txt", b"bbb");
        let items = f
            .host
            .put_paths(
                &f.station,
                &[root],
                StoreModeRow::Reference,
                None,
                f.me,
                2_000,
            )
            .await
            .unwrap();
        assert_eq!(f.host.sweep(3_000).unwrap(), 0);
        std::fs::remove_file(&b).unwrap();
        assert_eq!(f.host.sweep(4_000).unwrap(), 1);
        let row = f.store.get_station_item(&items[0].item).unwrap().unwrap();
        assert!(!row.item.available);
    }

    #[test]
    fn expired_item_is_removed_by_sweep() {
        let f = fixture();
        let item = f
            .host
            .put_text(&f.station, "x", Some(5_000), f.me, 2_000)
            .unwrap();
        assert_eq!(f.host.sweep(4_999).unwrap(), 0);
        assert_eq!(f.host.sweep(5_000).unwrap(), 1);
        let row = f.store.get_station_item(&item.item).unwrap().unwrap();
        assert!(row.item.removed);
        assert_eq!(row.item.updated_at, 5_000);
    }

    #[test]
    fn put_with_expiry_records_expires_at() {
        let f = fixture();
        let item = f
            .host
            .put_text(&f.station, "x", Some(2_000 + 86_400_000), f.me, 2_000)
            .unwrap();
        let rows = f.store.station_items(&f.station, 0, false).unwrap();
        assert_eq!(rows[0].item.item, item.item);
        assert_eq!(rows[0].item.expires_at, Some(2_000 + 86_400_000));
    }

    #[tokio::test]
    async fn put_rejects_past_expiry_and_unhosted_station() {
        let f = fixture();
        assert!(matches!(
            f.host.put_text(&f.station, "x", Some(1_999), f.me, 2_000),
            Err(CoreError::InvalidArgument(_))
        ));
        assert!(matches!(
            f.host.put_text(&f.station, "x", Some(2_000), f.me, 2_000),
            Err(CoreError::InvalidArgument(_))
        ));
        assert!(matches!(
            f.host.put_text(&ConvId([9u8; 32]), "x", None, f.me, 2_000),
            Err(CoreError::InvalidArgument(_))
        ));
        let long = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(matches!(
            f.host.put_text(&f.station, &long, None, f.me, 2_000),
            Err(CoreError::InvalidArgument(_))
        ));
        let r = f
            .host
            .put_paths(
                &f.station,
                &[f.files.path().join("missing")],
                StoreModeRow::Reference,
                None,
                f.me,
                2_000,
            )
            .await;
        assert!(matches!(r, Err(CoreError::InvalidArgument(_))));
    }

    #[test]
    fn set_pinned_updates_row() {
        let f = fixture();
        let item = f.host.put_text(&f.station, "x", None, f.me, 2_000).unwrap();
        f.host
            .set_pinned(&f.station, &item.item, true, 2_005)
            .unwrap();
        let row = f.store.get_station_item(&item.item).unwrap().unwrap();
        assert!(row.item.pinned);
        assert_eq!(row.item.updated_at, 2_005);
        assert!(matches!(
            f.host.set_pinned(&f.station, &ItemId([3u8; 16]), true, 1),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn put_blobs_creates_items_without_local_copy() {
        let f = fixture();
        let meta = BlobMeta {
            hash: BlobHash([5u8; 32]),
            size: 9,
            name: "m.bin".into(),
            mime: "application/octet-stream".into(),
            media: None,
            thumb: None,
        };
        f.store.put_blob(&meta, &[[1u8; 32]]).unwrap();
        // 模拟一条消息里已知的文件。
        {
            let conn = f.store.conn().unwrap();
            conn.execute(
                "INSERT INTO message_blobs(author, seq, idx, conv_id, hash, name, size, mime, rel_path, hlc_millis, hlc_counter, meta)
                 VALUES(?3, 1, 0, ?3, ?1, 'm.bin', 9, 'application/octet-stream', NULL, 1, 0, ?2)",
                rusqlite::params![&meta.hash.0[..], lanmsg_proto::to_cbor(&meta), &[1u8; 32][..]],
            )
            .unwrap();
        }
        let metas = resolve_blob_metas(&f.store, &[meta.hash.to_hex()]).unwrap();
        assert_eq!(metas, vec![meta.clone()]);
        let items = f
            .host
            .put_blobs(&f.station, metas, None, f.me, 2_000)
            .unwrap();
        let row = f.store.get_station_item(&items[0].item).unwrap().unwrap();
        assert!(row.item.available);
        assert_eq!(row.local_path, None);
        assert_eq!(row.store_mode, None);
        assert!(!f.data.path().join("Station").exists());
        assert!(f.store.get_local(&meta.hash).unwrap().is_none());

        assert!(matches!(
            resolve_blob_metas(&f.store, &[BlobHash([0u8; 32]).to_hex()]),
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            resolve_blob_metas(&f.store, &["zz".into()]),
            Err(CoreError::InvalidArgument(_))
        ));
        assert!(matches!(
            f.host.put_blobs(&f.station, Vec::new(), None, f.me, 2_000),
            Err(CoreError::InvalidArgument(_))
        ));
    }

    #[test]
    fn accept_remote_rejects_other_users() {
        let f = fixture();
        let item = StationItem {
            item: ItemId::random(),
            added_by: DeviceId([8u8; 32]),
            added_at: 1,
            expires_at: None,
            content: StationContent::Text { text: "t".into() },
            pinned: false,
            removed: false,
            available: true,
            updated_at: 1,
        };
        let stranger = peer(&UserKey::generate(), DeviceId([8u8; 32]));
        assert!(matches!(
            f.host
                .accept_remote(&stranger, &f.station, item.clone(), false, 2_000),
            Err(CoreError::Rejected(_))
        ));
        assert!(f.store.get_station_item(&item.item).unwrap().is_none());
    }

    #[test]
    fn accept_remote_text_and_files_follow_mode() {
        let f = fixture();
        // 同一用户的另一台设备：用本机用户密钥签发证书只需 user 字段一致，
        // 这里直接构造 cert.user = 站点 owner。
        let mut p = peer(&UserKey::generate(), DeviceId([8u8; 32]));
        p.cert.as_mut().unwrap().user = f.user;
        let blob = BlobMeta {
            hash: BlobHash([6u8; 32]),
            size: 4,
            name: "r.bin".into(),
            mime: "application/octet-stream".into(),
            media: None,
            thumb: None,
        };
        let mk = |content| StationItem {
            item: ItemId::random(),
            added_by: DeviceId([1u8; 32]),
            added_at: 1,
            expires_at: None,
            content,
            pinned: false,
            removed: true,
            available: true,
            updated_at: 1,
        };
        let text = f
            .host
            .accept_remote(
                &p,
                &f.station,
                mk(StationContent::Text { text: "t".into() }),
                true,
                2_000,
            )
            .unwrap();
        assert!(text.available && !text.removed);
        assert_eq!(text.added_by, p.device);

        let copy = f
            .host
            .accept_remote(
                &p,
                &f.station,
                mk(StationContent::File { blob: blob.clone() }),
                true,
                2_001,
            )
            .unwrap();
        let row = f.store.get_station_item(&copy.item).unwrap().unwrap();
        assert!(!row.item.available);
        assert_eq!(row.store_mode, Some(StoreModeRow::Copy));
        assert_eq!(f.store.holders(&blob.hash).unwrap(), vec![p.device]);

        let blobs_mode = f
            .host
            .accept_remote(
                &p,
                &f.station,
                mk(StationContent::File { blob }),
                false,
                2_002,
            )
            .unwrap();
        let row = f.store.get_station_item(&blobs_mode.item).unwrap().unwrap();
        assert!(row.item.available);
        assert_eq!(row.store_mode, None);

        // 拉取完成后标为可用。
        f.host
            .mark_available(&copy.item, Path::new("/x/y"), 3_000)
            .unwrap();
        let row = f.store.get_station_item(&copy.item).unwrap().unwrap();
        assert!(row.item.available);
        assert_eq!(row.local_path, Some(PathBuf::from("/x/y")));
    }

    #[tokio::test]
    async fn station_files_visible_to_own_user_only() {
        let f = fixture();
        let path = write(f.files.path(), "a.txt", b"hello");
        let items = f
            .host
            .put_paths(
                &f.station,
                &[path],
                StoreModeRow::Reference,
                None,
                f.me,
                2_000,
            )
            .await
            .unwrap();
        let hash = file_blob(&items[0]).hash;
        let other = UserId([0xee; 32]);
        assert!(f.store.blob_visible_to(&hash, &f.user, &f.user).unwrap());
        assert!(!f.store.blob_visible_to(&hash, &other, &f.user).unwrap());
        // 条目删除后 Blob 行消失，且本地行被清掉后本人也不再可见。
        f.host
            .remove(&f.station, &items[0].item, 3_000)
            .await
            .unwrap();
        f.store.remove_local(&hash).unwrap();
        assert!(!f.store.blob_visible_to(&hash, &f.user, &f.user).unwrap());
    }

    #[tokio::test]
    async fn copy_never_overwrites_source_for_dotdot_and_root_paths() {
        let f = fixture();
        let root = f.files.path().join("dir");
        let a = write(&root, "a.txt", b"keep me");
        write(&root, "sub/b.txt", b"b");
        // `dir/sub/..` 规整后就是 `dir`。
        let tricky = root.join("sub").join("..");
        let items = f
            .host
            .put_paths(&f.station, &[tricky], StoreModeRow::Copy, None, f.me, 2_000)
            .await
            .unwrap();
        let StationContent::Folder { name, .. } = &items[0].content else {
            panic!("expected folder");
        };
        assert_eq!(name, "dir");
        assert_eq!(std::fs::read(&a).unwrap(), b"keep me");
        // 文件路径加 `/..` 不是有效路径；根目录没有可用名字。
        for p in [a.join(".."), PathBuf::from("/")] {
            let r = f
                .host
                .put_paths(&f.station, &[p], StoreModeRow::Copy, None, f.me, 2_000)
                .await;
            assert!(matches!(r, Err(CoreError::InvalidArgument(_))));
        }
        assert_eq!(std::fs::read(&a).unwrap(), b"keep me");
    }

    #[tokio::test]
    async fn failed_put_paths_leaves_no_items_or_copies() {
        let f = fixture();
        let ok = write(f.files.path(), "a.txt", b"x");
        let r = f
            .host
            .put_paths(
                &f.station,
                &[ok, f.files.path().join("missing")],
                StoreModeRow::Copy,
                None,
                f.me,
                2_000,
            )
            .await;
        assert!(matches!(r, Err(CoreError::InvalidArgument(_))));
        assert!(f
            .store
            .station_items(&f.station, 0, true)
            .unwrap()
            .is_empty());
        let station_dir = f.data.path().join("Station").join(f.station.to_hex());
        let leftovers = std::fs::read_dir(&station_dir)
            .map(|d| d.count())
            .unwrap_or(0);
        assert_eq!(leftovers, 0);
    }

    #[tokio::test]
    async fn removing_copy_hands_local_row_to_surviving_reference() {
        let f = fixture();
        let path = write(f.files.path(), "a.txt", b"same");
        let r = f
            .host
            .put_paths(
                &f.station,
                std::slice::from_ref(&path),
                StoreModeRow::Reference,
                None,
                f.me,
                2_000,
            )
            .await
            .unwrap();
        let c = f
            .host
            .put_paths(
                &f.station,
                std::slice::from_ref(&path),
                StoreModeRow::Copy,
                None,
                f.me,
                2_001,
            )
            .await
            .unwrap();
        let hash = file_blob(&r[0]).hash;
        // 复制登记不抢走仍可读的既有本地行。
        let before = f.store.get_local(&hash).unwrap().unwrap();
        assert_eq!(before.path, std::fs::canonicalize(&path).unwrap());
        assert_eq!(before.kind, BlobLocalKind::Source);
        f.host.remove(&f.station, &c[0].item, 3_000).await.unwrap();
        let local = f.store.get_local(&hash).unwrap().unwrap();
        assert_eq!(local.path, std::fs::canonicalize(&path).unwrap());
        assert_eq!(local.kind, BlobLocalKind::Source);
    }

    #[tokio::test]
    async fn sweep_restores_cleared_local_row() {
        let f = fixture();
        let path = write(f.files.path(), "a.txt", b"hello");
        let items = f
            .host
            .put_paths(
                &f.station,
                &[path],
                StoreModeRow::Reference,
                None,
                f.me,
                2_000,
            )
            .await
            .unwrap();
        let hash = file_blob(&items[0]).hash;
        f.store.remove_local(&hash).unwrap();
        assert_eq!(f.host.sweep(3_000).unwrap(), 0);
        assert!(f.store.get_local(&hash).unwrap().is_some());
    }

    #[tokio::test]
    async fn pin_and_remove_check_station() {
        let f = fixture();
        let other = f.host.create("other", 1_500).unwrap().station;
        let item = f.host.put_text(&f.station, "x", None, f.me, 2_000).unwrap();
        assert!(matches!(
            f.host.set_pinned(&other, &item.item, true, 3_000),
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            f.host.remove(&other, &item.item, 3_000).await,
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            f.host.remove(&ConvId([9u8; 32]), &item.item, 3_000).await,
            Err(CoreError::InvalidArgument(_))
        ));
    }

    #[test]
    fn accept_remote_redelivery_keeps_fetched_copy_state() {
        let f = fixture();
        let mut p = peer(&UserKey::generate(), DeviceId([8u8; 32]));
        p.cert.as_mut().unwrap().user = f.user;
        let blob = BlobMeta {
            hash: BlobHash([6u8; 32]),
            size: 4,
            name: "r.bin".into(),
            mime: "application/octet-stream".into(),
            media: None,
            thumb: None,
        };
        let item = StationItem {
            item: ItemId::random(),
            added_by: DeviceId([1u8; 32]),
            added_at: 1,
            expires_at: None,
            content: StationContent::File { blob },
            pinned: false,
            removed: false,
            available: false,
            updated_at: 1,
        };
        f.host
            .accept_remote(&p, &f.station, item.clone(), true, 2_000)
            .unwrap();
        f.host
            .mark_available(&item.item, Path::new("/x/y"), 2_100)
            .unwrap();
        f.host
            .accept_remote(&p, &f.station, item.clone(), true, 3_000)
            .unwrap();
        let row = f.store.get_station_item(&item.item).unwrap().unwrap();
        assert!(row.item.available);
        assert_eq!(row.local_path, Some(PathBuf::from("/x/y")));
        assert_eq!(row.item.added_at, 2_000);
    }

    #[tokio::test]
    async fn removed_item_is_not_resurrected_by_late_delivery() {
        let f = fixture();
        let mut p = peer(&UserKey::generate(), DeviceId([8u8; 32]));
        p.cert.as_mut().unwrap().user = f.user;
        let item = StationItem {
            item: ItemId::random(),
            added_by: DeviceId([1u8; 32]),
            added_at: 1,
            expires_at: None,
            content: StationContent::Text { text: "t".into() },
            pinned: false,
            removed: false,
            available: true,
            updated_at: 1,
        };
        f.host
            .accept_remote(&p, &f.station, item.clone(), false, 2_000)
            .unwrap();
        f.host.remove(&f.station, &item.item, 3_000).await.unwrap();
        let again = f
            .host
            .accept_remote(&p, &f.station, item.clone(), false, 4_000)
            .unwrap();
        assert!(again.removed);
        let row = f.store.get_station_item(&item.item).unwrap().unwrap();
        assert!(row.item.removed);
        assert_eq!(row.item.updated_at, 3_000);
    }

    #[test]
    fn copy_layout_neutralizes_hostile_names() {
        let blob = |name: &str| BlobMeta {
            hash: BlobHash([1u8; 32]),
            size: 1,
            name: name.into(),
            mime: "x/y".into(),
            media: None,
            thumb: None,
        };
        let content = StationContent::Folder {
            name: "../evil".into(),
            entries: vec![
                FolderEntry {
                    rel_path: "/etc/../passwd".into(),
                    blob: blob("passwd"),
                },
                FolderEntry {
                    rel_path: "C:\\x\\..\\y".into(),
                    blob: blob("y"),
                },
            ],
        };
        for (_, rel) in copy_layout(&content) {
            assert!(rel.is_relative());
            assert!(
                rel.components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
                "{rel:?}"
            );
        }
    }

    #[tokio::test]
    async fn sweep_removes_old_orphan_copy_dirs_only() {
        let f = fixture();
        let path = write(f.files.path(), "a.txt", b"x");
        let live = f
            .host
            .put_paths(
                &f.station,
                std::slice::from_ref(&path),
                StoreModeRow::Copy,
                None,
                f.me,
                2_000,
            )
            .await
            .unwrap();
        let base = f.data.path().join("Station").join(f.station.to_hex());
        let orphan_old = base.join(ItemId::random().to_hex());
        let orphan_new = base.join(ItemId::random().to_hex());
        for d in [&orphan_old, &orphan_new] {
            std::fs::create_dir_all(d).unwrap();
        }
        let old = std::fs::File::open(&orphan_old).unwrap();
        old.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(7200))
            .unwrap();
        drop(old);
        f.host.sweep(3_000).unwrap();
        assert!(!orphan_old.exists());
        assert!(orphan_new.exists());
        assert!(base.join(live[0].item.to_hex()).exists());
    }
}
