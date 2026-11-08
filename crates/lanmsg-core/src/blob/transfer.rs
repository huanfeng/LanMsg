// P2 Task 12：传输引擎——分段并行、多源、断点续传。
//
// download 流程（计划原文）：查消息行 → 计算目标路径 → 候选源（holders ∩ 已连接，
// 空则 BlobQuery + 等 BLOB_QUERY_WAIT）→ GetMeta 拿分段表（本地优先）→ 创建 .part
// → 工作者（min(BLOB_PARALLEL_CONNS, 剩余分段)条，按路径优先级绑定候选源）
// 逐段 GetRange + blake3 校验 → 全部完成后整文件复核 → rename 落地 → set_local
// → add_holder → 向自己设备发 BlobHave → Done 事件 → MessageUpdated。
// 取消经 CancellationToken：任务静默退出，行保持 Queued/Running 与 .part，
// 重启后 resume_all 续传；显式 cancel() 才置 Cancelled 并删 .part。
// 阻塞 IO（hash_file）一律 spawn_blocking。注释中文，标识符英文。

use std::collections::{HashMap, VecDeque};
use std::io::{Seek, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
// AsyncRead/AsyncSeek/AsyncWrite 仅被 cfg(test) 的 read_segment 使用。
#[cfg(test)]
use tokio::io::{AsyncRead, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use lanmsg_api::types::{Event, SettingsDto, TransferFailReason};
use lanmsg_proto::consts::{
    BLOB_PARALLEL_CONNS, BLOB_QUERY_WAIT, BLOB_SEGMENT_LEN, PROGRESS_INTERVAL,
};
use lanmsg_proto::data::{read_data_frame, write_data_frame, DataFrame};
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{BlobHash, ConvId, DeviceId};
use lanmsg_proto::types::{BlobHave, BlobMeta, BlobQuery, MsgBody, PathClass};

use crate::blob::hash::hash_file;
use crate::error::CoreError;
use crate::identity::Identity;
use crate::net::conn::DataStream;
use crate::net::ConnManager;
use crate::service::views::message_view;
use crate::settings::load_settings;
use crate::store::{
    BlobLocalKind, BlobLocalRow, ConvKindRow, MessageBlobRow, Store, TransferRow, TransferState,
};

/// 下载来源：Blob 元数据，加上用来向对端询问持有者的会话（中转站条目没有会话）。
#[derive(Debug, Clone)]
struct DlSource {
    meta: BlobMeta,
    conv: Option<ConvId>,
}

impl From<MessageBlobRow> for DlSource {
    fn from(mb: MessageBlobRow) -> Self {
        DlSource {
            meta: mb.meta,
            conv: Some(mb.conv),
        }
    }
}

/// 传输失败：界面依据的结构化原因 + 英文排障信息。
type Failure = (TransferFailReason, String);

/// 单段交换超时（含连接建立）：空闲超时的 2 倍。
const SEGMENT_TIMEOUT: Duration = Duration::from_secs(30 * 2);

/// 工作者阶段的最大轮数（候选耗尽后再查询重试的上限）。
const MAX_SOURCE_ROUNDS: usize = 3;

/// 读分段缓冲（1 MiB）。
const SEG_BUF: usize = 1024 * 1024;

/// 传输引擎。
pub struct TransferEngine {
    store: Arc<Store>,
    conns: ConnManager,
    identity: Arc<RwLock<Identity>>,
    events: broadcast::Sender<Event>,
    /// 数据目录（下载目录缺省回退 `<data_dir>/downloads`）。
    data_dir: PathBuf,
    /// 活动任务表：transfer_id → 取消令牌。
    tasks: StdMutex<HashMap<String, CancellationToken>>,
    /// 串行化 download 的查重与建行（download 全程无 await，可安全持有）。
    download_lock: StdMutex<()>,
}

/// 单个分段读取失败的性质。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchFail {
    /// 数据损坏 / 对端无此 Blob：计源失败一次。
    Source,
    /// 连接层错误：源立即判死。
    Conn,
    /// 本地写盘错误：与源无关，不计源失败，整个传输以 Io 终止。
    Local,
}

/// 工作者共享状态。
struct Shared {
    /// 待下载分段队列。
    queue: StdMutex<VecDeque<usize>>,
    /// 已取出、未落定的分段数。
    in_flight: AtomicUsize,
    /// 串行化 mark_segment_done（存储层读改写非原子）。
    mark_lock: StdMutex<()>,
    /// 各源累计失败次数。
    failures: StdMutex<HashMap<DeviceId, u32>>,
    /// 已耗尽的源。
    dead: StdMutex<Vec<DeviceId>>,
    /// 任一工作者宣告全部源不可用时的错误信息（仅作停止标志与排障）。
    failed_out: StdMutex<Option<Failure>>,
    /// 进度窗口。
    progress: StdMutex<Progress>,
}

/// 进度状态：最近 1 秒滑动窗口 + 发送节流。
struct Progress {
    last_emit: Option<Instant>,
    done_bytes: u64,
    window: VecDeque<(Instant, u64)>,
}

/// 单个工作者任务的参数包。
struct WorkerJob {
    id: String,
    hash: BlobHash,
    total: u64,
    part: PathBuf,
    segs: Vec<[u8; 32]>,
    candidates: Vec<DeviceId>,
    start_cand: usize,
    token: CancellationToken,
}

impl TransferEngine {
    /// 构造传输引擎。
    pub fn new(
        store: Arc<Store>,
        conns: ConnManager,
        identity: Arc<RwLock<Identity>>,
        events: broadcast::Sender<Event>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            store,
            conns,
            identity,
            events,
            data_dir,
            tasks: StdMutex::new(HashMap::new()),
            download_lock: StdMutex::new(()),
        }
    }

    /// 发起下载：返回 transfer_id；已有活动传输则返回其 id；
    /// 已在本地 → InvalidArgument("已下载")。后台任务完成剩余流程。
    pub async fn download(self: Arc<Self>, hash: BlobHash) -> Result<String, CoreError> {
        // download 全程无 await，持锁安全；并发同 hash 请求在此串行。
        let _guard = self.download_lock.lock().unwrap();
        if self.store.get_local(&hash)?.is_some() {
            return Err(CoreError::InvalidArgument("already downloaded".into()));
        }
        if let Some(row) = self.store.transfer_for_hash(&hash)? {
            return Ok(row.transfer_id);
        }
        let Some(mb) = self.store.message_blobs_by_hash(&hash)?.into_iter().next() else {
            return Err(CoreError::NotFound(
                "no message references this file".into(),
            ));
        };
        let settings = load_settings(&self.store)?;
        let title = self.conv_title(&mb.conv);
        // 文件夹条目：wire/库中的 rel_path 是含文件名的条目路径（相对文件夹根），
        // 落地目录 = <文件夹名>/<条目所在子目录>（文件名由 dest_path_for 追加）。
        let rel_dir = match mb.rel_path.as_deref() {
            Some(rel) => {
                let folder_name = match self.store.get_message(&mb.msg)?.map(|m| m.env.body) {
                    Some(MsgBody::Folder { name, .. }) => name,
                    _ => String::new(),
                };
                let sub = rel.rsplit_once('/').map_or("", |(dir, _)| dir);
                Some(if sub.is_empty() {
                    folder_name
                } else {
                    format!("{folder_name}/{sub}")
                })
            }
            None => None,
        };
        let dest = self.dest_path_for(&settings, &title, &mb.meta.name, rel_dir.as_deref());
        self.start(mb.into(), dest)
    }

    /// 以显式目录为落地位置下载（中转站使用）：文件名经 `sanitize_component` 清洗后
    /// 放在 `dest_dir` 下。不检查本地是否已有该 Blob（调用方明确要一份副本）；
    /// 同哈希已有活动传输则返回其 id。`size` 以调用方给定的元数据为准。
    pub async fn download_to(
        self: Arc<Self>,
        meta: BlobMeta,
        dest_dir: &Path,
    ) -> Result<String, CoreError> {
        let _guard = self.download_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(row) = self.store.transfer_for_hash(&meta.hash)? {
            return Ok(row.transfer_id);
        }
        let dest = dest_dir.join(sanitize_component(&meta.name));
        self.start(DlSource { meta, conv: None }, dest)
    }

    /// 中转站条目的下载目录：`<下载目录>/Station/<站点名>/<相对目录各段>`。
    pub fn station_dest_dir(
        &self,
        settings: &SettingsDto,
        station_name: &str,
        sub_dirs: &[String],
    ) -> PathBuf {
        let base = if settings.download_dir.trim().is_empty() {
            self.data_dir.join("downloads")
        } else {
            PathBuf::from(&settings.download_dir)
        };
        let mut p = base.join("Station").join(sanitize_component(station_name));
        for seg in sub_dirs {
            p = p.join(sanitize_component(seg));
        }
        p
    }

    /// 建传输行并启动后台任务（调用方已持有 `download_lock`）。
    fn start(self: &Arc<Self>, src: DlSource, dest: PathBuf) -> Result<String, CoreError> {
        let mut part = dest.clone().into_os_string();
        part.push(".part");
        let row = TransferRow {
            transfer_id: new_transfer_id(),
            hash: src.meta.hash,
            state: TransferState::Queued,
            part_path: PathBuf::from(part),
            dest_path: dest,
            total: src.meta.size,
            segment_hashes: Vec::new(),
            done_segments: Vec::new(),
            error: None,
        };
        self.store.upsert_transfer(&row)?;
        let id = row.transfer_id.clone();
        self.spawn_download(row, src);
        Ok(id)
    }

    /// 取消传输：停止工作者、删除 .part、状态 Cancelled 并发事件。
    /// 已结束的传输幂等返回 Ok；不存在 → NotFound。
    pub async fn cancel(&self, transfer_id: &str) -> Result<(), CoreError> {
        let token = self.tasks.lock().unwrap().get(transfer_id).cloned();
        if let Some(token) = token {
            token.cancel();
            // 等任务退出（任务只做静默退出，不写库），避免与工作者写库竞争。
            let start = Instant::now();
            while self.tasks.lock().unwrap().contains_key(transfer_id) {
                if start.elapsed() > Duration::from_secs(5) {
                    tracing::warn!(transfer = %transfer_id, "timed out waiting for task to exit on cancel");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        let Some(mut row) = self.store.get_transfer(transfer_id)? else {
            return Err(CoreError::NotFound(format!(
                "transfer not found: {transfer_id}"
            )));
        };
        if !matches!(row.state, TransferState::Queued | TransferState::Running) {
            return Ok(());
        }
        row.state = TransferState::Cancelled;
        row.error = Some("cancelled".into());
        self.store.upsert_transfer(&row)?;
        match std::fs::remove_file(&row.part_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let _ = self.events.send(Event::TransferFinished {
            transfer: row.transfer_id.clone(),
            hash: row.hash.to_hex(),
            ok: false,
            error: Some("cancelled".into()),
            reason: Some(TransferFailReason::Cancelled),
        });
        Ok(())
    }

    /// 恢复遗留的活动传输（Service 启动后调用）：.part 存在且长度 == total
    /// 时沿用 done_segments 继续，否则在后台任务里重置后重新开始。
    pub async fn resume_all(self: Arc<Self>) -> Result<(), CoreError> {
        for row in self.store.active_transfers()? {
            let src = match self
                .store
                .message_blobs_by_hash(&row.hash)?
                .into_iter()
                .next()
            {
                Some(mb) => DlSource::from(mb),
                None => match self.store.station_blob(&row.hash)? {
                    Some((_, meta, _)) => DlSource { meta, conv: None },
                    None => continue,
                },
            };
            self.spawn_download(row, src);
        }
        Ok(())
    }

    /// 目标路径：download_dir/<会话标题 sanitize>/<rel_path 各段 sanitize>/<name>。
    pub fn dest_path_for(
        &self,
        settings: &SettingsDto,
        conv_title: &str,
        name: &str,
        rel_path: Option<&str>,
    ) -> PathBuf {
        let base = if settings.download_dir.trim().is_empty() {
            self.data_dir.join("downloads")
        } else {
            PathBuf::from(&settings.download_dir)
        };
        let mut p = base.join(sanitize_component(conv_title));
        if let Some(rel) = rel_path {
            for seg in rel.split('/') {
                p = p.join(sanitize_component(seg));
            }
        }
        p.join(sanitize_component(name))
    }

    /// Service 关闭时调用：取消全部活动任务。任务静默退出，行保持
    /// Queued/Running，重启后由 resume_all 续传。
    pub fn shutdown_all(&self) {
        for token in self.tasks.lock().unwrap().values() {
            token.cancel();
        }
    }

    // ---- 内部 ----

    /// 启动后台传输任务。
    fn spawn_download(self: &Arc<Self>, row: TransferRow, mb: DlSource) {
        let token = CancellationToken::new();
        self.tasks
            .lock()
            .unwrap()
            .insert(row.transfer_id.clone(), token.clone());
        let engine = self.clone();
        tokio::spawn(async move {
            engine.run_transfer(row, mb, token).await;
        });
    }

    /// 传输任务主体：分段下载 → 取消静默退出 / 失败写行 → 成功落地。
    async fn run_transfer(
        self: Arc<Self>,
        mut row: TransferRow,
        mb: DlSource,
        token: CancellationToken,
    ) {
        let id = row.transfer_id.clone();
        let engine = self.clone();
        let mut err = if row.total == 0 {
            // 大小 0：跳过网络，直接建空文件（整文件复核比对 blake3(b"")）。
            Self::create_part(&row.part_path, 0)
                .await
                .err()
                .map(|e| (TransferFailReason::Io, e.to_string()))
        } else {
            engine.download_segments(&mut row, &mb, &token).await
        };
        self.tasks.lock().unwrap().remove(&id);
        if token.is_cancelled() {
            // cancel() 负责删除 .part、置 Cancelled 并发事件。
            return;
        }
        match err.take() {
            None => {
                let self_device = self.identity.read().unwrap().device_id();
                if let Some((reason, msg)) = self.finalize(&row, &mb.meta, &self_device).await {
                    self.fail(&row, reason, &msg);
                }
            }
            Some((reason, msg)) => self.fail(&row, reason, &msg),
        }
    }

    /// 分段下载主流程：候选源 → GetMeta → .part 准备 → 工作者并行取段。
    /// 返回 None 表示全部分段完成；Some(错误信息) 表示失败。
    async fn download_segments(
        self: Arc<Self>,
        row: &mut TransferRow,
        mb: &DlSource,
        token: &CancellationToken,
    ) -> Option<Failure> {
        let mut candidates = self.candidates(&row.hash);
        if candidates.is_empty() {
            self.solicit(&row.hash, mb.conv.as_ref(), token).await;
            if token.is_cancelled() {
                return None;
            }
            candidates = self.candidates(&row.hash);
            if candidates.is_empty() {
                return Some((TransferFailReason::SourceOffline, "source offline".into()));
            }
        }

        // 分段表：本地 blob_segments 优先，否则向候选源取 GetMeta（并持久化，
        // 便于本机将来作为源服务他人）。
        let segs = match self.store.blob_segments(&row.hash) {
            Ok(Some(s)) if !s.is_empty() => s,
            _ => match self.fetch_meta(&candidates, &row.hash).await {
                Some(s) => {
                    if let Err(e) = self.store.put_blob(&mb.meta, &s) {
                        tracing::warn!(error = %e, "failed to persist segment table");
                    }
                    s
                }
                None => return Some((TransferFailReason::SourceOffline, "source offline".into())),
            },
        };

        // .part 一致性：存在且长度 == total 才沿用，否则重建并重置分段进度。
        let n = segs.len();
        let part_ok = std::fs::metadata(&row.part_path)
            .map(|m| m.len() == row.total)
            .unwrap_or(false);
        if !part_ok {
            if let Err(e) = Self::create_part(&row.part_path, row.total).await {
                return Some((
                    TransferFailReason::Io,
                    format!("failed to create .part file: {e}"),
                ));
            }
            row.done_segments = vec![false; n];
        }
        if row.done_segments.len() != n {
            row.done_segments = vec![false; n];
        }
        row.state = TransferState::Running;
        if let Err(e) = self.store.upsert_transfer(row) {
            return Some((
                TransferFailReason::Io,
                format!("failed to write transfer record: {e}"),
            ));
        }

        // 工作者阶段：候选全部耗尽时再做一轮 BlobQuery 等待重试（最多
        // MAX_SOURCE_ROUNDS 轮，覆盖持有者应答迟到 / 候选上线的情况）。
        for _round in 0..MAX_SOURCE_ROUNDS {
            // 从存储重读分段进度（上一轮工作者可能已标记完成）。
            if let Ok(Some(fresh)) = self.store.get_transfer(&row.transfer_id) {
                row.done_segments = fresh.done_segments;
            }
            let remaining = n - row.done_segments.iter().filter(|&&b| b).count();
            if remaining == 0 {
                return None;
            }
            let n_workers = BLOB_PARALLEL_CONNS.min(remaining).max(1);
            let start_done = row
                .done_segments
                .iter()
                .enumerate()
                .filter(|(_, &b)| b)
                .map(|(i, _)| segment_len(row.total, i))
                .sum();
            let shared = Arc::new(Shared {
                queue: StdMutex::new((0..n).filter(|&i| !row.done_segments[i]).collect()),
                in_flight: AtomicUsize::new(0),
                mark_lock: StdMutex::new(()),
                failures: StdMutex::new(HashMap::new()),
                dead: StdMutex::new(Vec::new()),
                failed_out: StdMutex::new(None),
                progress: StdMutex::new(Progress {
                    last_emit: None,
                    done_bytes: start_done,
                    window: VecDeque::new(),
                }),
            });
            let n_cand = candidates.len();
            let mut handles = Vec::with_capacity(n_workers);
            for i in 0..n_workers {
                let job = WorkerJob {
                    id: row.transfer_id.clone(),
                    hash: row.hash,
                    total: row.total,
                    part: row.part_path.clone(),
                    segs: segs.clone(),
                    candidates: candidates.clone(),
                    start_cand: i % n_cand,
                    token: token.clone(),
                };
                handles.push(tokio::spawn(Self::worker(
                    self.clone(),
                    shared.clone(),
                    job,
                )));
            }
            for h in handles {
                if let Err(e) = h.await {
                    tracing::warn!(error = %e, "transfer worker exited abnormally");
                }
            }
            let failed = shared.failed_out.lock().unwrap().clone();
            if failed.is_none() || token.is_cancelled() {
                return None;
            }
            // 本地写错误：换源无济于事，立即以 Io 终止，不再等持有者应答。
            if let Some(f @ (TransferFailReason::Io, _)) = failed {
                return Some(f);
            }
            // 全部候选耗尽：等一轮持有者应答，出现新候选则重试。
            self.solicit(&row.hash, mb.conv.as_ref(), token).await;
            if token.is_cancelled() {
                return None;
            }
            let fresh = self.candidates(&row.hash);
            if fresh.is_empty() {
                return Some((TransferFailReason::SourceOffline, "source offline".into()));
            }
            candidates = fresh;
        }
        Some((TransferFailReason::SourceOffline, "source offline".into()))
    }

    /// 工作者：从共享队列取分段，绑定候选源下载，失败换绑下一个候选。
    async fn worker(self: Arc<Self>, shared: Arc<Shared>, job: WorkerJob) {
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&job.part)
        {
            Ok(f) => Arc::new(StdMutex::new(f)),
            Err(e) => {
                *shared.failed_out.lock().unwrap() = Some((
                    TransferFailReason::Io,
                    format!("failed to open .part file: {e}"),
                ));
                return;
            }
        };
        let n = job.candidates.len();
        let mut cand = job.start_cand % n;
        // 工作者私有数据连接：跨分段复用，出错即弃、下次重开。
        let mut conn: Option<DataStream> = None;
        loop {
            if job.token.is_cancelled() || shared.failed_out.lock().unwrap().is_some() {
                return;
            }
            let seg = shared.queue.lock().unwrap().pop_front();
            let Some(idx) = seg else {
                // 队列空：全部落定即退出，否则等待在途分段失败回队。
                if shared.in_flight.load(Ordering::SeqCst) == 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            };
            shared.in_flight.fetch_add(1, Ordering::SeqCst);
            let offset = idx as u64 * BLOB_SEGMENT_LEN;
            let len = segment_len(job.total, idx);
            match self
                .fetch_segment(
                    &job.candidates[cand],
                    &job.hash,
                    offset,
                    len,
                    &job.segs[idx],
                    &file,
                    &mut conn,
                )
                .await
            {
                Ok(()) => {
                    {
                        let _guard = shared.mark_lock.lock().unwrap();
                        if let Err(e) = self.store.mark_segment_done(&job.id, idx) {
                            tracing::warn!(error = %e, "failed to mark segment done");
                        }
                    }
                    self.record_progress(&shared, &job, len);
                    shared.in_flight.fetch_sub(1, Ordering::SeqCst);
                }
                Err(fail) => {
                    shared.queue.lock().unwrap().push_back(idx);
                    shared.in_flight.fetch_sub(1, Ordering::SeqCst);
                    let dead_now = match fail {
                        FetchFail::Source => {
                            let mut f = shared.failures.lock().unwrap();
                            let c = f.entry(job.candidates[cand]).or_insert(0);
                            *c += 1;
                            *c >= 3
                        }
                        FetchFail::Conn => true,
                        FetchFail::Local => {
                            *shared.failed_out.lock().unwrap() = Some((
                                TransferFailReason::Io,
                                "failed to write segment to .part file".into(),
                            ));
                            return;
                        }
                    };
                    if dead_now {
                        shared.dead.lock().unwrap().push(job.candidates[cand]);
                    }
                    match Self::next_candidate(&shared, &job, cand) {
                        Some(next) => cand = next,
                        None => {
                            *shared.failed_out.lock().unwrap() =
                                Some((TransferFailReason::SourceOffline, "source offline".into()));
                            return;
                        }
                    }
                }
            }
        }
    }

    /// 取分段：复用工作者私有空闲连接或新开数据连接，GetRange → read_segment。
    /// 连接保存在 `conn`（None = 无空闲），失败时置 None 丢弃。
    #[allow(clippy::too_many_arguments)] // 取段参数（定位、校验、落盘、连接）各自独立
    async fn fetch_segment(
        &self,
        dev: &DeviceId,
        hash: &BlobHash,
        offset: u64,
        len: u64,
        expect: &[u8; 32],
        file: &Arc<StdMutex<std::fs::File>>,
        conn: &mut Option<DataStream>,
    ) -> Result<(), FetchFail> {
        if conn.is_none() {
            *conn = Some(
                self.conns
                    .open_data(dev)
                    .await
                    .map_err(|_| FetchFail::Conn)?,
            );
        }
        let stream = conn.as_mut().expect("conn was just filled");
        let res = tokio::time::timeout(SEGMENT_TIMEOUT, async {
            write_data_frame(
                stream,
                &DataFrame::GetRange {
                    hash: *hash,
                    offset,
                    len,
                },
            )
            .await
            .map_err(|_| FetchFail::Conn)?;
            let frame = read_data_frame(stream).await.map_err(|_| FetchFail::Conn)?;
            match frame {
                DataFrame::RangeStart { .. } => {
                    read_segment_to_file(stream, len, expect, file.clone(), offset).await
                }
                other => {
                    tracing::debug!(?other, offset, "unexpected reply frame to GetRange");
                    Err(FetchFail::Source)
                }
            }
        })
        .await;
        match res {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                // 连接不可信：丢弃，下次取段重开。
                *conn = None;
                Err(e)
            }
            Err(_) => {
                // 超时：连接状态未知，一并丢弃。
                *conn = None;
                Err(FetchFail::Conn)
            }
        }
    }

    /// 向候选源取 GetMeta 分段表。
    async fn fetch_meta(&self, candidates: &[DeviceId], hash: &BlobHash) -> Option<Vec<[u8; 32]>> {
        for dev in candidates {
            let Ok(mut stream) = self.conns.open_data(dev).await else {
                continue;
            };
            let got = tokio::time::timeout(SEGMENT_TIMEOUT, async {
                if write_data_frame(&mut stream, &DataFrame::GetMeta { hash: *hash })
                    .await
                    .is_err()
                {
                    return None;
                }
                read_data_frame(&mut stream).await.ok()
            })
            .await
            .ok()
            .flatten();
            if let Some(DataFrame::Meta { segment_hashes, .. }) = got {
                if !segment_hashes.is_empty() {
                    return Some(segment_hashes);
                }
            }
        }
        None
    }

    /// 候选源：holders ∩ 已连接，按路径优先级排序（Lan < Vpn < Internet < Relay）。
    fn candidates(&self, hash: &BlobHash) -> Vec<DeviceId> {
        let mut cands: Vec<DeviceId> = self
            .store
            .holders(hash)
            .unwrap_or_default()
            .into_iter()
            .filter(|d| self.conns.is_connected(d))
            .collect();
        cands.sort_by_key(|d| self.conns.peer(d).map(|p| path_rank(&p.path)).unwrap_or(4));
        cands
    }

    /// 候选为空时：向已连接的自己设备与该会话对端设备发 BlobQuery，
    /// 等待 BLOB_QUERY_WAIT 让持有者应答回填。
    async fn solicit(&self, hash: &BlobHash, conv: Option<&ConvId>, token: &CancellationToken) {
        let self_user = self.identity.read().unwrap().user_id();
        let peer_user = conv
            .and_then(|c| self.store.get_conversation(c).ok().flatten())
            .and_then(|c| c.peer_user);
        let targets: Vec<DeviceId> = self
            .conns
            .peers()
            .into_iter()
            .filter(|p| solicit_wants(p.cert.as_ref().map(|c| c.user), self_user, peer_user))
            .map(|p| p.device)
            .collect();
        for t in targets {
            if let Err(e) = self
                .conns
                .send(
                    &t,
                    Frame::BlobQuery(BlobQuery {
                        hashes: vec![*hash],
                    }),
                )
                .await
            {
                tracing::warn!(error = %e, "failed to send BlobQuery");
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(BLOB_QUERY_WAIT) => {}
            _ = token.cancelled() => {}
        }
    }

    /// 记录一个分段完成并发送节流后的进度事件。
    fn record_progress(&self, shared: &Shared, job: &WorkerJob, added: u64) {
        let emit = {
            let mut p = shared.progress.lock().unwrap();
            p.done_bytes += added;
            let now = Instant::now();
            while let Some((t, _)) = p.window.front() {
                if now.duration_since(*t) > Duration::from_secs(1) {
                    p.window.pop_front();
                } else {
                    break;
                }
            }
            let done = p.done_bytes;
            p.window.push_back((now, done));
            if !p.last_emit.is_some_and(|t| t.elapsed() < PROGRESS_INTERVAL) {
                p.last_emit = Some(now);
                let (t0, b0) = p.window.front().copied().unwrap_or((now, p.done_bytes));
                let bps = if p.window.len() >= 2 && now > t0 {
                    (p.done_bytes - b0) * 1000 / now.duration_since(t0).as_millis().max(1) as u64
                } else {
                    0
                };
                Some((p.done_bytes, bps))
            } else {
                None
            }
        };
        if let Some((done, bytes_per_sec)) = emit {
            let _ = self.events.send(Event::TransferProgress {
                transfer: job.id.clone(),
                hash: job.hash.to_hex(),
                done,
                total: job.total,
                bytes_per_sec,
            });
        }
    }

    /// 下一个未耗尽的候选（从 cur+1 环形扫描）。
    fn next_candidate(shared: &Shared, job: &WorkerJob, cur: usize) -> Option<usize> {
        let n = job.candidates.len();
        let mut i = cur;
        for _ in 0..n {
            i = (i + 1) % n;
            if !shared.dead.lock().unwrap().contains(&job.candidates[i]) {
                return Some(i);
            }
        }
        None
    }

    /// 完成落地：整文件复核 → rename 到最终路径 → set_local → add_holder →
    /// BlobHave 给自己设备 → Done 行 → TransferFinished → MessageUpdated。
    /// 返回 None 表示成功，Some(错误信息) 表示失败（行置 Failed）。
    async fn finalize(
        &self,
        row: &TransferRow,
        meta: &BlobMeta,
        self_device: &DeviceId,
    ) -> Option<Failure> {
        let part = row.part_path.clone();
        let hashed = match tokio::task::spawn_blocking(move || hash_file(&part)).await {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                return Some((
                    TransferFailReason::Io,
                    format!("failed to read downloaded file: {e}"),
                ))
            }
            Err(e) => {
                return Some((
                    TransferFailReason::Io,
                    format!("background task failed: {e}"),
                ))
            }
        };
        if hashed.hash.0 != meta.hash.0 {
            if let Err(e) = std::fs::remove_file(&row.part_path) {
                tracing::warn!(error = %e, "failed to delete .part file after verification failure");
            }
            return Some((
                TransferFailReason::VerifyFailed,
                "verification failed".into(),
            ));
        }
        let final_path = unique_path(&row.dest_path);
        if let Err(e) = tokio::fs::rename(&row.part_path, &final_path).await {
            return Some((
                TransferFailReason::Io,
                format!("failed to finalize file: {e}"),
            ));
        }
        let local = BlobLocalRow {
            hash: row.hash,
            path: final_path,
            kind: BlobLocalKind::Downloaded,
            size: hashed.size,
            mtime: hashed.mtime,
        };
        // 已有仍可读的本地行时不覆盖（例如中转站副本不应抢走消息附件的来源路径）。
        let existing_ok = self
            .store
            .get_local(&row.hash)
            .ok()
            .flatten()
            .is_some_and(|l| {
                std::fs::metadata(&l.path).is_ok_and(|m| m.is_file() && m.len() == l.size)
            });
        if existing_ok {
            // 保留既有行。
        } else if let Err(e) = self.store.set_local(&local) {
            return Some((
                TransferFailReason::Io,
                format!("failed to write local record: {e}"),
            ));
        }
        if let Err(e) = self
            .store
            .add_holder(&row.hash, self_device, crate::service::now_millis())
        {
            tracing::warn!(error = %e, "failed to record this device as holder");
        }
        // 向已连接的自己设备与消息作者的设备发 BlobHave：自己的设备据此知道本机持有，
        // 作者据此知道“对方已下载”（供“已送达”判定）。
        let self_user = self.identity.read().unwrap().user_id();
        let authors: Vec<DeviceId> = self
            .store
            .message_blobs_by_hash(&row.hash)
            .map(|v| v.into_iter().map(|mb| mb.msg.author).collect())
            .unwrap_or_default();
        for p in self.conns.peers() {
            let is_own = p.cert.as_ref().is_some_and(|c| Some(c.user) == self_user);
            if is_own || authors.contains(&p.device) {
                if let Err(e) = self
                    .conns
                    .send(
                        &p.device,
                        Frame::BlobHave(BlobHave {
                            hashes: vec![row.hash],
                        }),
                    )
                    .await
                {
                    tracing::warn!(error = %e, "failed to send BlobHave");
                }
            }
        }
        let mut done_row = row.clone();
        done_row.state = TransferState::Done;
        // 下载走到这里即全部分段完成；内存副本的分段进度是过时的，全置 true。
        done_row.done_segments = vec![true; row.done_segments.len()];
        if let Err(e) = self.store.upsert_transfer(&done_row) {
            return Some((
                TransferFailReason::Io,
                format!("failed to write transfer record: {e}"),
            ));
        }
        let _ = self.events.send(Event::TransferFinished {
            transfer: row.transfer_id.clone(),
            hash: row.hash.to_hex(),
            ok: true,
            error: None,
            reason: None,
        });
        // 对引用该 blob 的每条消息发 MessageUpdated（视图状态随下载完成变化）。
        let peers = self.conns.peers();
        for mb in self
            .store
            .message_blobs_by_hash(&row.hash)
            .unwrap_or_default()
        {
            if let Ok(Some(sm)) = self.store.get_message(&mb.msg) {
                let _ = self.events.send(Event::MessageUpdated {
                    msg: message_view(self.store.as_ref(), self_device, &peers, &sm),
                });
            }
        }
        // 引用该 blob 的中转站：通知界面刷新条目状态。
        let mut stations: Vec<ConvId> = self
            .store
            .station_items_with_blob(&row.hash)
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.station)
            .collect();
        stations.dedup();
        for station in stations {
            let _ = self.events.send(Event::StationUpdated {
                station: station.to_hex(),
            });
        }
        None
    }

    /// 失败收尾：写行 Failed 并发 TransferFinished{ok:false}。
    fn fail(&self, row: &TransferRow, reason: TransferFailReason, msg: &str) {
        let mut r = row.clone();
        r.state = TransferState::Failed;
        r.error = Some(msg.to_string());
        if let Err(e) = self.store.upsert_transfer(&r) {
            tracing::warn!(error = %e, "failed to write failed state");
        }
        let _ = self.events.send(Event::TransferFinished {
            transfer: row.transfer_id.clone(),
            hash: row.hash.to_hex(),
            ok: false,
            error: Some(msg.to_string()),
            reason: Some(reason),
        });
    }

    /// 会话标题（与 views::conv_summary 同规则）：DM 用对端显示名，
    /// 我的设备会话用固定名，其余用存储标题。
    fn conv_title(&self, conv: &ConvId) -> String {
        let Ok(Some(row)) = self.store.get_conversation(conv) else {
            return conv.to_hex();
        };
        match row.kind {
            ConvKindRow::Dm => {
                let named = row
                    .peer_user
                    .as_ref()
                    .and_then(|u| self.store.get_user(u).ok().flatten())
                    .map(|u| u.display_name);
                named.unwrap_or_else(|| {
                    row.peer_user
                        .as_ref()
                        .map(|u| u.short())
                        .unwrap_or_default()
                })
            }
            // 仅用作落地目录名，保持语言中立。
            ConvKindRow::SelfDevices => "My devices".into(),
            _ => row.title.clone().unwrap_or_default(),
        }
    }

    /// 创建 .part（含父目录）：File::create 截断 + set_len 预分配。
    async fn create_part(path: &Path, size: u64) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let f = tokio::fs::File::create(path).await?;
        f.set_len(size).await?;
        Ok(())
    }
}

/// 段 idx 的实际长度（末段可能不足整段）。
fn segment_len(total: u64, idx: usize) -> u64 {
    let start = idx as u64 * BLOB_SEGMENT_LEN;
    BLOB_SEGMENT_LEN.min(total.saturating_sub(start))
}

/// 读取 `len` 字节分段：seek 到 `offset` 边读边写 `out`，同时计算 blake3；
/// 读完与 `expect` 比对，不符返回错误（数据已写入对应偏移，等待重传覆盖，
/// 整文件复核兜底）。
/// 注：正式路径走 read_segment_to_file（spawn_blocking 落盘）；本函数仅服务单元测试。
#[cfg(test)]
pub(crate) async fn read_segment<R, W>(
    r: &mut R,
    len: u64,
    expect: &[u8; 32],
    out: &mut W,
    offset: u64,
) -> Result<(), CoreError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + AsyncSeek + Unpin,
{
    out.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; SEG_BUF];
    let mut remaining = len;
    while remaining > 0 {
        let want = buf.len().min(remaining as usize);
        let n = r.read(&mut buf[..want]).await?;
        if n == 0 {
            return Err(CoreError::Proto("segment data ended early".into()));
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n]).await?;
        remaining -= n as u64;
    }
    if hasher.finalize().as_bytes() != expect {
        return Err(CoreError::Rejected("segment verification failed".into()));
    }
    Ok(())
}

/// 同 read_segment，但落盘走 `spawn_blocking` + 同步文件（分段块粒度），
/// 避免在异步读循环中混用 tokio 文件状态机。
async fn read_segment_to_file(
    r: &mut DataStream,
    len: u64,
    expect: &[u8; 32],
    file: Arc<StdMutex<std::fs::File>>,
    offset: u64,
) -> Result<(), FetchFail> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; SEG_BUF];
    let mut remaining = len;
    let mut pos = offset;
    while remaining > 0 {
        let want = buf.len().min(remaining as usize);
        let n = r
            .read(&mut buf[..want])
            .await
            .map_err(|_| FetchFail::Source)?;
        if n == 0 {
            return Err(FetchFail::Source);
        }
        hasher.update(&buf[..n]);
        let chunk = buf[..n].to_vec();
        let f = file.clone();
        let at = pos;
        tokio::task::spawn_blocking(move || {
            let mut f = f.lock().unwrap();
            f.seek(std::io::SeekFrom::Start(at))?;
            f.write_all(&chunk)?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| FetchFail::Local)?
        .map_err(|_| FetchFail::Local)?;
        pos += n as u64;
        remaining -= n as u64;
    }
    if hasher.finalize().as_bytes() != expect {
        return Err(FetchFail::Source);
    }
    Ok(())
}

/// 路径优先级：Lan < Vpn < Internet < Relay。
fn path_rank(p: &PathClass) -> u8 {
    match p {
        PathClass::Lan => 0,
        PathClass::Vpn => 1,
        PathClass::Internet => 2,
        PathClass::Relay => 3,
    }
}

/// 已存在则依次尝试 "名 (1).扩展名"、"名 (2).扩展名"…
pub fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = match name.rsplit_once('.') {
        // ".txt" 这类隐藏文件没有可用主干，整体作为 stem。
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), Some(ext.to_string())),
        _ => (name, None),
    };
    let dir = p.parent().unwrap_or(Path::new("."));
    for i in 1..u64::MAX {
        let cand = match &ext {
            Some(e) => dir.join(format!("{stem} ({i}).{e}")),
            None => dir.join(format!("{stem} ({i})")),
        };
        if !cand.exists() {
            return cand;
        }
    }
    p.to_path_buf()
}

/// solicit 的目标判定：对端必须有证书，且用户是自己或会话对端用户。
/// SelfDevices 会话 peer_user 为 None，此时只匹配自己的设备（无证书对端不会因 None == None 误中）。
fn solicit_wants(
    peer: Option<lanmsg_proto::ids::UserId>,
    self_user: Option<lanmsg_proto::ids::UserId>,
    conv_peer: Option<lanmsg_proto::ids::UserId>,
) -> bool {
    peer.is_some_and(|u| Some(u) == self_user || Some(u) == conv_peer)
}

/// Windows 保留设备名（不区分大小写，且带扩展名也保留）。
fn is_windows_reserved(stem: &str) -> bool {
    let up = stem.to_ascii_uppercase();
    matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((up.starts_with("COM") || up.starts_with("LPT"))
            && up.len() == 4
            && matches!(up.as_bytes()[3], b'1'..=b'9'))
}

/// 文件名段清洗：替换 \ / : * ? " < > | 与控制字符为 _，去掉首尾空格与点，
/// 空则 "_"（同时天然阻断了 "." / ".." 路径穿越）；Windows 保留名
/// （CON、PRN、AUX、NUL、COM1-9、LPT1-9，含 `con.txt`）前加下划线。
pub fn sanitize_component(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed: String = replaced.trim_matches(|c| c == ' ' || c == '.').to_string();
    if trimmed.is_empty() {
        "_".into()
    } else if is_windows_reserved(trimmed.split('.').next().unwrap_or("").trim_end()) {
        format!("_{trimmed}")
    } else {
        trimmed
    }
}

/// 新 transfer_id：16 字节随机十六进制。
fn new_transfer_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    crate::identity::hex_encode(&b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// 正确数据 → Ok 且写入位置正确（offset 处内容一致，游标停在 offset+len）。
    #[tokio::test]
    async fn read_segment_accepts_correct_data() {
        let data: Vec<u8> = (0..100u8).collect();
        let expect = *blake3::hash(&data).as_bytes();
        let mut r = Cursor::new(data.clone());
        let mut out = Cursor::new(vec![0u8; 150]);
        read_segment(&mut r, 100, &expect, &mut out, 40)
            .await
            .unwrap();
        assert_eq!(&out.get_ref()[40..140], &data[..]);
        assert_eq!(out.position(), 140);
    }

    /// 错误数据 → Err。
    #[tokio::test]
    async fn read_segment_rejects_corrupt_data() {
        let expect = *blake3::hash(&(0..100u8).collect::<Vec<u8>>()).as_bytes();
        let mut bad: Vec<u8> = (0..100u8).collect();
        bad[7] ^= 0xFF;
        let mut r = Cursor::new(bad);
        let mut out = Cursor::new(vec![0u8; 100]);
        assert!(read_segment(&mut r, 100, &expect, &mut out, 0)
            .await
            .is_err());
    }

    #[test]
    fn sanitize_windows_reserved_names() {
        assert_eq!(sanitize_component("CON"), "_CON");
        assert_eq!(sanitize_component("con.txt"), "_con.txt");
        assert_eq!(sanitize_component("Nul.tar.gz"), "_Nul.tar.gz");
        assert_eq!(sanitize_component("com1"), "_com1");
        assert_eq!(sanitize_component("LPT9.log"), "_LPT9.log");
        assert_eq!(sanitize_component("COM0"), "COM0");
        assert_eq!(sanitize_component("COM10"), "COM10");
        assert_eq!(sanitize_component("console.txt"), "console.txt");
        assert_eq!(sanitize_component("a.con"), "a.con");
        // 文件夹相对路径段与会话标题经同一函数。
        let segs: Vec<String> = "aux/ok/PRN.d".split('/').map(sanitize_component).collect();
        assert_eq!(segs, ["_aux", "ok", "_PRN.d"]);
    }

    #[test]
    fn solicit_skips_certless_and_strangers() {
        use lanmsg_proto::ids::UserId;
        let me = Some(UserId([1u8; 32]));
        let other = Some(UserId([2u8; 32]));
        let stranger = Some(UserId([3u8; 32]));
        // SelfDevices 会话（conv_peer = None）：仅自己的设备。
        assert!(solicit_wants(me, me, None));
        assert!(!solicit_wants(None, me, None));
        assert!(!solicit_wants(other, me, None));
        // Dm 会话：自己与对端用户，陌生人与无证书对端排除。
        assert!(solicit_wants(other, me, other));
        assert!(!solicit_wants(stranger, me, other));
        assert!(!solicit_wants(None, me, other));
        // 自身未初始化时无证书对端同样不命中。
        assert!(!solicit_wants(None, None, None));
    }

    #[test]
    fn segment_len_edges() {
        assert_eq!(segment_len(0, 0), 0);
        assert_eq!(segment_len(BLOB_SEGMENT_LEN, 0), BLOB_SEGMENT_LEN);
        assert_eq!(segment_len(BLOB_SEGMENT_LEN + 10, 1), 10);
        assert_eq!(segment_len(5, 1), 0);
    }
}
