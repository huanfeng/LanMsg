// P2 存储扩展：Blob 元数据、本地文件、持有者、preparing、传输（Task 2）。
// paths/segment_hashes/done_segments 存 CBOR 或按段一字节；meta 存 CBOR。
// 注释中文，标识符英文。

use std::path::PathBuf;

use lanmsg_proto::ids::{BlobHash, ConvId, DeviceId, MsgId};
use lanmsg_proto::types::{BlobMeta, Envelope, Hlc};

use crate::store::{MsgStateRow, Store};

/// 本地 Blob 行。
#[derive(Debug, Clone, PartialEq)]
pub struct BlobLocalRow {
    pub hash: BlobHash,
    pub path: PathBuf,
    pub kind: BlobLocalKind,
    pub size: u64,
    pub mtime: i64,
}

/// 本地 Blob 来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobLocalKind {
    Source,
    Downloaded,
}

impl BlobLocalKind {
    fn as_str(&self) -> &'static str {
        match self {
            BlobLocalKind::Source => "source",
            BlobLocalKind::Downloaded => "downloaded",
        }
    }

    fn parse(s: &str) -> Result<Self, crate::store::StoreError> {
        match s {
            "source" => Ok(BlobLocalKind::Source),
            "downloaded" => Ok(BlobLocalKind::Downloaded),
            _ => Err(crate::store::StoreError::Codec(format!(
                "未知 blob 本地类型: {s}"
            ))),
        }
    }
}

/// preparing 行：文件消息已分配序号但信封未入库。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparingRow {
    pub author: DeviceId,
    pub seq: u64,
    pub conv: ConvId,
    pub conv_seq: u64,
    pub hlc: Hlc,
    pub kind: PreparingKind,
    pub paths: Vec<PathBuf>,
    pub created_at: i64,
}

/// preparing 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparingKind {
    Files,
    Folder,
}

impl PreparingKind {
    fn as_str(&self) -> &'static str {
        match self {
            PreparingKind::Files => "files",
            PreparingKind::Folder => "folder",
        }
    }

    fn parse(s: &str) -> Result<Self, crate::store::StoreError> {
        match s {
            "files" => Ok(PreparingKind::Files),
            "folder" => Ok(PreparingKind::Folder),
            _ => Err(crate::store::StoreError::Codec(format!(
                "未知 preparing 类型: {s}"
            ))),
        }
    }
}

/// 传输状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl TransferState {
    fn as_str(&self) -> &'static str {
        match self {
            TransferState::Queued => "queued",
            TransferState::Running => "running",
            TransferState::Done => "done",
            TransferState::Failed => "failed",
            TransferState::Cancelled => "cancelled",
        }
    }

    fn parse(s: &str) -> Result<Self, crate::store::StoreError> {
        match s {
            "queued" => Ok(TransferState::Queued),
            "running" => Ok(TransferState::Running),
            "done" => Ok(TransferState::Done),
            "failed" => Ok(TransferState::Failed),
            "cancelled" => Ok(TransferState::Cancelled),
            _ => Err(crate::store::StoreError::Codec(format!(
                "未知传输状态: {s}"
            ))),
        }
    }
}

/// 传输表行。
#[derive(Debug, Clone, PartialEq)]
pub struct TransferRow {
    pub transfer_id: String,
    pub hash: BlobHash,
    pub state: TransferState,
    pub part_path: PathBuf,
    pub dest_path: PathBuf,
    pub total: u64,
    pub segment_hashes: Vec<[u8; 32]>,
    pub done_segments: Vec<bool>,
    pub error: Option<String>,
}

/// 消息引用的 Blob 行。
#[derive(Debug, Clone, PartialEq)]
pub struct MessageBlobRow {
    pub msg: MsgId,
    pub idx: u32,
    pub conv: ConvId,
    pub meta: BlobMeta,
    pub rel_path: Option<String>,
    pub hlc: Hlc,
}

impl Store {
    /// 写入 Blob 元数据（已存在则覆盖分段）。
    pub fn put_blob(
        &self,
        meta: &BlobMeta,
        segment_hashes: &[[u8; 32]],
    ) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "INSERT INTO blobs(hash, size, mime, segment_hashes) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(hash) DO UPDATE SET segment_hashes = excluded.segment_hashes",
            rusqlite::params![
                &meta.hash.0[..],
                meta.size as i64,
                meta.mime,
                lanmsg_proto::to_cbor(&segment_hashes.to_vec()),
            ],
        )?;
        Ok(())
    }

    /// 查询 Blob 分段哈希，不存在返回 Ok(None)。
    pub fn blob_segments(
        &self,
        hash: &BlobHash,
    ) -> Result<Option<Vec<[u8; 32]>>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let bytes: Option<Vec<u8>> = conn
            .query_row(
                "SELECT segment_hashes FROM blobs WHERE hash = ?1",
                [&hash.0[..]],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|b| {
                lanmsg_proto::from_cbor(&b)
                    .map_err(|e| crate::store::StoreError::Codec(e.to_string()))
            })
            .transpose()
    }

    /// 写入本地 Blob 行（存在则覆盖）。
    pub fn set_local(&self, row: &BlobLocalRow) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "INSERT INTO blob_local(hash, path, kind, size, mtime) VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(hash) DO UPDATE SET path = excluded.path, kind = excluded.kind,
               size = excluded.size, mtime = excluded.mtime",
            rusqlite::params![
                &row.hash.0[..],
                row.path.to_string_lossy(),
                row.kind.as_str(),
                row.size as i64,
                row.mtime,
            ],
        )?;
        Ok(())
    }

    /// 查询本地 Blob 行。
    pub fn get_local(
        &self,
        hash: &BlobHash,
    ) -> Result<Option<BlobLocalRow>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<(String, String, i64, i64)> = conn
            .query_row(
                "SELECT path, kind, size, mtime FROM blob_local WHERE hash = ?1",
                [&hash.0[..]],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((path, kind, size, mtime)) = row else {
            return Ok(None);
        };
        Ok(Some(BlobLocalRow {
            hash: *hash,
            path: PathBuf::from(path),
            kind: BlobLocalKind::parse(&kind)?,
            size: size as u64,
            mtime,
        }))
    }

    /// 删除本地 Blob 行。
    pub fn remove_local(&self, hash: &BlobHash) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute("DELETE FROM blob_local WHERE hash = ?1", [&hash.0[..]])?;
        Ok(())
    }

    /// 记录持有者。
    pub fn add_holder(
        &self,
        hash: &BlobHash,
        device: &DeviceId,
        now: i64,
    ) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "INSERT INTO blob_holders(hash, device_id, updated_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(hash, device_id) DO UPDATE SET updated_at = excluded.updated_at",
            rusqlite::params![&hash.0[..], &device.0[..], now],
        )?;
        Ok(())
    }

    /// 移除持有者。
    pub fn remove_holder(
        &self,
        hash: &BlobHash,
        device: &DeviceId,
    ) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "DELETE FROM blob_holders WHERE hash = ?1 AND device_id = ?2",
            rusqlite::params![&hash.0[..], &device.0[..]],
        )?;
        Ok(())
    }

    /// 列出持有者。
    pub fn holders(&self, hash: &BlobHash) -> Result<Vec<DeviceId>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT device_id FROM blob_holders WHERE hash = ?1")?;
        let rows = stmt.query_map([&hash.0[..]], |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = Vec::new();
        for r in rows {
            let bytes = r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
            out.push(DeviceId(bytes.try_into().map_err(|bytes: Vec<u8>| {
                crate::store::StoreError::Codec(format!("device_id 长度非法: {}", bytes.len()))
            })?));
        }
        Ok(out)
    }

    /// 插入 preparing 行。
    pub fn insert_preparing(&self, row: &PreparingRow) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "INSERT INTO preparing(author, seq, conv_id, conv_seq, hlc_millis, hlc_counter, kind, paths, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                &row.author.0[..],
                row.seq as i64,
                &row.conv.0,
                row.conv_seq as i64,
                row.hlc.millis,
                row.hlc.counter as i64,
                row.kind.as_str(),
                lanmsg_proto::to_cbor(&row.paths.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>()),
                row.created_at,
            ],
        )?;
        Ok(())
    }

    /// 列出全部 preparing 行（created_at 升序）。
    pub fn list_preparing(&self) -> Result<Vec<PreparingRow>, crate::store::StoreError> {
        self.query_preparing("SELECT author, seq, conv_id, conv_seq, hlc_millis, hlc_counter, kind, paths, created_at FROM preparing ORDER BY created_at ASC")
    }

    /// 删除 preparing 行（后台准备失败时调用）。
    pub fn delete_preparing(
        &self,
        author: &DeviceId,
        seq: u64,
    ) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "DELETE FROM preparing WHERE author = ?1 AND seq = ?2",
            rusqlite::params![&author.0[..], seq as i64],
        )?;
        Ok(())
    }

    /// 会话内的 preparing 行。
    pub fn preparing_in_conv(
        &self,
        conv: &ConvId,
    ) -> Result<Vec<PreparingRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT author, seq, conv_id, conv_seq, hlc_millis, hlc_counter, kind, paths, created_at
             FROM preparing WHERE conv_id = ?1 ORDER BY conv_seq ASC",
        )?;
        let rows = stmt.query_map([&conv.0[..]], preparing_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?);
        }
        Ok(out)
    }

    fn query_preparing(&self, sql: &str) -> Result<Vec<PreparingRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], preparing_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?);
        }
        Ok(out)
    }

    /// 单事务完成 preparing：删 preparing 行、insert_envelope(Sending)、
    /// put_blob、set_local、add_holder(自己)。
    pub fn finish_preparing(
        &self,
        author: &DeviceId,
        seq: u64,
        env: &Envelope,
        segs: &[(BlobHash, Vec<[u8; 32]>)],
        locals: &[BlobLocalRow],
        now: i64,
    ) -> Result<(), crate::store::StoreError> {
        let mut conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM preparing WHERE author = ?1 AND seq = ?2",
            rusqlite::params![&author.0[..], seq as i64],
        )?;
        // 复用 insert_envelope 的事务内逻辑：直接在 tx 上插入核心行。
        insert_envelope_on_tx(&tx, env, MsgStateRow::Sending, now)?;
        // 信封携带的 Blob 元数据，按 hash 索引（Files/Folder 两种形态）。
        let mut metas: Vec<&BlobMeta> = Vec::new();
        match &env.body {
            lanmsg_proto::types::MsgBody::Files { items } => {
                metas.extend(items.iter());
            }
            lanmsg_proto::types::MsgBody::Folder { entries, .. } => {
                metas.extend(entries.iter().map(|e| &e.blob));
            }
            _ => {}
        }
        for (hash, seg) in segs {
            let meta = metas.iter().find(|m| m.hash == *hash).ok_or_else(|| {
                crate::store::StoreError::Codec(format!("segs 中的 hash 无对应元数据: {hash:?}"))
            })?;
            tx.execute(
                "INSERT INTO blobs(hash, size, mime, segment_hashes) VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(hash) DO UPDATE SET segment_hashes = excluded.segment_hashes",
                rusqlite::params![
                    &meta.hash.0[..],
                    meta.size as i64,
                    meta.mime,
                    lanmsg_proto::to_cbor(&seg.to_vec()),
                ],
            )?;
        }
        for local in locals {
            tx.execute(
                "INSERT INTO blob_local(hash, path, kind, size, mtime) VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(hash) DO UPDATE SET path = excluded.path, kind = excluded.kind,
                   size = excluded.size, mtime = excluded.mtime",
                rusqlite::params![
                    &local.hash.0[..],
                    local.path.to_string_lossy(),
                    local.kind.as_str(),
                    local.size as i64,
                    local.mtime,
                ],
            )?;
            tx.execute(
                "INSERT INTO blob_holders(hash, device_id, updated_at) VALUES(?1, ?2, ?3)
                 ON CONFLICT(hash, device_id) DO UPDATE SET updated_at = excluded.updated_at",
                rusqlite::params![&local.hash.0[..], &author.0[..], now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 写入或更新传输行。
    pub fn upsert_transfer(&self, row: &TransferRow) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "INSERT INTO transfers(transfer_id, hash, state, part_path, dest_path, total, segment_hashes, done_segments, error, created_at, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
             ON CONFLICT(transfer_id) DO UPDATE SET state = excluded.state,
               done_segments = excluded.done_segments, error = excluded.error, updated_at = excluded.updated_at",
            rusqlite::params![
                row.transfer_id,
                &row.hash.0[..],
                row.state.as_str(),
                row.part_path.to_string_lossy(),
                row.dest_path.to_string_lossy(),
                row.total as i64,
                lanmsg_proto::to_cbor(&row.segment_hashes.to_vec()),
                lanmsg_proto::to_cbor(&row.done_segments.iter().map(|&b| b as u8).collect::<Vec<_>>()),
                row.error,
                chrono_now(),
            ],
        )?;
        Ok(())
    }

    /// 查询传输行。
    pub fn get_transfer(&self, id: &str) -> Result<Option<TransferRow>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<TransferTuple> = conn
            .query_row(
                "SELECT hash, state, part_path, dest_path, total, segment_hashes, done_segments, error
                 FROM transfers WHERE transfer_id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )
            .optional()?;
        row.map(|t| transfer_from_tuple(id.to_string(), t))
            .transpose()
    }

    /// 活动传输（Queued/Running）。
    pub fn active_transfers(&self) -> Result<Vec<TransferRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT transfer_id, hash, state, part_path, dest_path, total, segment_hashes, done_segments, error
             FROM transfers WHERE state IN ('queued', 'running')",
        )?;
        let rows = stmt.query_map([], transfer_full_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?);
        }
        Ok(out)
    }

    /// 某 hash 的活动中传输。
    pub fn transfer_for_hash(
        &self,
        hash: &BlobHash,
    ) -> Result<Option<TransferRow>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<(String, TransferTuple)> = conn
            .query_row(
                "SELECT transfer_id, hash, state, part_path, dest_path, total, segment_hashes, done_segments, error
                 FROM transfers WHERE hash = ?1 AND state IN ('queued', 'running')",
                [&hash.0[..]],
                |r| {
                    Ok((
                        r.get(0)?,
                        (
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                            r.get(8)?,
                        ),
                    ))
                },
            )
            .optional()?;
        row.map(|(id, t)| transfer_from_tuple(id, t)).transpose()
    }

    /// 标记某段完成。
    pub fn mark_segment_done(&self, id: &str, idx: usize) -> Result<(), crate::store::StoreError> {
        let Some(Some(row)) = self.get_transfer(id)?.map(|mut r| {
            if idx < r.done_segments.len() {
                r.done_segments[idx] = true;
                Some(r)
            } else {
                None
            }
        }) else {
            return Ok(());
        };
        self.upsert_transfer(&row)?;
        Ok(())
    }

    /// 引用了某 hash 的消息 Blob 行。
    pub fn message_blobs_by_hash(
        &self,
        hash: &BlobHash,
    ) -> Result<Vec<MessageBlobRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT author, seq, idx, conv_id, meta, rel_path, hlc_millis, hlc_counter
             FROM message_blobs WHERE hash = ?1 ORDER BY hlc_millis ASC",
        )?;
        let rows = stmt.query_map([&hash.0[..]], message_blob_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?);
        }
        Ok(out)
    }

    /// 某消息引用的 Blob 行（idx 升序）。
    pub fn message_blobs_of(
        &self,
        msg: &MsgId,
    ) -> Result<Vec<MessageBlobRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT author, seq, idx, conv_id, meta, rel_path, hlc_millis, hlc_counter
             FROM message_blobs WHERE author = ?1 AND seq = ?2 ORDER BY idx ASC",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![&msg.author.0[..], msg.seq as i64],
            message_blob_from_row,
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?);
        }
        Ok(out)
    }

    /// `user` 能否看到该 Blob：自己可看任何被消息引用的；他人仅当
    /// `message_blobs` 中存在 `conv_id = dm(self_user, user)` 的引用。
    pub fn blob_visible_to(
        &self,
        hash: &BlobHash,
        user: &lanmsg_proto::ids::UserId,
        self_user: &lanmsg_proto::ids::UserId,
    ) -> Result<bool, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        if *user == *self_user {
            let n: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM message_blobs WHERE hash = ?1",
                    [&hash.0[..]],
                    |r| r.get(0),
                )
                .optional()?;
            return Ok(n.is_some());
        }
        let dm = ConvId::dm(self_user, user);
        let n: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM message_blobs WHERE hash = ?1 AND conv_id = ?2",
                rusqlite::params![&hash.0[..], &dm.0[..]],
                |r| r.get(0),
            )
            .optional()?;
        Ok(n.is_some())
    }
}

/// 查询传输行的原始元组。
type TransferTuple = (
    Vec<u8>,
    String,
    String,
    String,
    i64,
    Vec<u8>,
    Vec<u8>,
    Option<String>,
);

fn transfer_from_tuple(
    transfer_id: String,
    t: TransferTuple,
) -> Result<TransferRow, crate::store::StoreError> {
    let (hash, state, part_path, dest_path, total, segs, done, error) = t;
    let segs: Vec<Vec<u8>> = lanmsg_proto::from_cbor(&segs)
        .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
    let done_bytes: Vec<u8> = lanmsg_proto::from_cbor(&done)
        .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
    Ok(TransferRow {
        transfer_id,
        hash: BlobHash(hash.try_into().map_err(|bytes: Vec<u8>| {
            crate::store::StoreError::Codec(format!("hash 长度非法: {}", bytes.len()))
        })?),
        state: TransferState::parse(&state)?,
        part_path: PathBuf::from(part_path),
        dest_path: PathBuf::from(dest_path),
        total: total as u64,
        segment_hashes: segs
            .into_iter()
            .filter_map(|s| <[u8; 32]>::try_from(s).ok())
            .collect(),
        done_segments: done_bytes.into_iter().map(|b| b != 0).collect(),
        error,
    })
}

fn transfer_full_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TransferRow> {
    let transfer_id: String = r.get(0)?;
    let tuple: TransferTuple = (
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
    );
    transfer_from_tuple(transfer_id, tuple).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(e))
    })
}

fn preparing_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PreparingRow> {
    let paths: Vec<String> = lanmsg_proto::from_cbor(&r.get::<_, Vec<u8>>(7)?).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Blob, Box::new(e))
    })?;
    Ok(PreparingRow {
        author: DeviceId(
            r.get::<_, Vec<u8>>(0)?
                .try_into()
                .map_err(|bytes: Vec<u8>| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        format!("author 长度非法: {}", bytes.len()).into(),
                    )
                })?,
        ),
        seq: r.get::<_, i64>(1)? as u64,
        conv: ConvId(
            r.get::<_, Vec<u8>>(2)?
                .try_into()
                .map_err(|bytes: Vec<u8>| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Blob,
                        format!("conv_id 长度非法: {}", bytes.len()).into(),
                    )
                })?,
        ),
        conv_seq: r.get::<_, i64>(3)? as u64,
        hlc: Hlc {
            millis: r.get(4)?,
            counter: r.get::<_, i64>(5)? as u32,
        },
        kind: PreparingKind::parse(&r.get::<_, String>(6)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?,
        paths: paths.into_iter().map(PathBuf::from).collect(),
        created_at: r.get(8)?,
    })
}

fn message_blob_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MessageBlobRow> {
    let meta_bytes: Vec<u8> = r.get(4)?;
    let meta: BlobMeta = lanmsg_proto::from_cbor(&meta_bytes).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Blob, Box::new(e))
    })?;
    Ok(MessageBlobRow {
        msg: MsgId {
            author: DeviceId(
                r.get::<_, Vec<u8>>(0)?
                    .try_into()
                    .map_err(|bytes: Vec<u8>| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Blob,
                            format!("author 长度非法: {}", bytes.len()).into(),
                        )
                    })?,
            ),
            seq: r.get::<_, i64>(1)? as u64,
        },
        idx: r.get::<_, i64>(2)? as u32,
        conv: ConvId(
            r.get::<_, Vec<u8>>(3)?
                .try_into()
                .map_err(|bytes: Vec<u8>| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Blob,
                        format!("conv_id 长度非法: {}", bytes.len()).into(),
                    )
                })?,
        ),
        meta,
        rel_path: r.get(5)?,
        hlc: Hlc {
            millis: r.get(6)?,
            counter: r.get::<_, i64>(7)? as u32,
        },
    })
}

/// 毫秒墙钟。
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 在给定事务上插入信封核心行（供 finish_preparing 复用，推进版本向量）。
fn insert_envelope_on_tx(
    tx: &rusqlite::Transaction<'_>,
    env: &Envelope,
    state: MsgStateRow,
    received_at: i64,
) -> Result<(), crate::store::StoreError> {
    let text = match &env.body {
        lanmsg_proto::types::MsgBody::Text { text } => Some(text.clone()),
        _ => None,
    };
    tx.execute(
        "INSERT OR IGNORE INTO messages(author, seq, conv_id, conv_seq, hlc_millis, hlc_counter, envelope, state, received_at, text, author_user, hidden, deleted)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, 0)",
        rusqlite::params![
            &env.id.author.0[..],
            env.id.seq as i64,
            &env.conv.0,
            env.conv_seq as i64,
            env.hlc.millis,
            env.hlc.counter as i64,
            lanmsg_proto::to_cbor(env),
            state.as_str(),
            received_at,
            text,
            lanmsg_proto::body_is_hidden(&env.body) as i64,
        ],
    )?;
    tx.execute(
        "INSERT INTO version_vectors(conv_id, author, contiguous) VALUES(?1, ?2, ?3)
         ON CONFLICT(conv_id, author) DO UPDATE SET contiguous = MAX(contiguous, excluded.contiguous)",
        rusqlite::params![
            &env.conv.0,
            &env.id.author.0[..],
            env.conv_seq as i64
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn meta(n: u8) -> BlobMeta {
        BlobMeta {
            hash: BlobHash([n; 32]),
            size: 100,
            name: format!("文件{n}.txt"),
            mime: "text/plain".into(),
            media: None,
            thumb: None,
        }
    }

    #[test]
    fn put_and_get_blob_segments() {
        let s = store();
        s.put_blob(&meta(1), &[[1u8; 32], [2u8; 32]]).unwrap();
        assert_eq!(
            s.blob_segments(&BlobHash([1u8; 32])).unwrap(),
            Some(vec![[1u8; 32], [2u8; 32]])
        );
        assert_eq!(s.blob_segments(&BlobHash([9u8; 32])).unwrap(), None);
    }

    #[test]
    fn local_rows_roundtrip() {
        let s = store();
        let row = BlobLocalRow {
            hash: BlobHash([2u8; 32]),
            path: PathBuf::from("/tmp/a.bin"),
            kind: BlobLocalKind::Source,
            size: 5,
            mtime: 7,
        };
        s.set_local(&row).unwrap();
        assert_eq!(s.get_local(&BlobHash([2u8; 32])).unwrap(), Some(row));
        s.remove_local(&BlobHash([2u8; 32])).unwrap();
        assert_eq!(s.get_local(&BlobHash([2u8; 32])).unwrap(), None);
    }

    #[test]
    fn holders_roundtrip() {
        let s = store();
        s.add_holder(&BlobHash([3u8; 32]), &DeviceId([4u8; 32]), 1)
            .unwrap();
        s.add_holder(&BlobHash([3u8; 32]), &DeviceId([5u8; 32]), 2)
            .unwrap();
        let mut h = s.holders(&BlobHash([3u8; 32])).unwrap();
        h.sort();
        assert_eq!(h.len(), 2);
        s.remove_holder(&BlobHash([3u8; 32]), &DeviceId([4u8; 32]))
            .unwrap();
        assert_eq!(s.holders(&BlobHash([3u8; 32])).unwrap().len(), 1);
    }

    #[test]
    fn transfer_roundtrip_and_mark_segment() {
        let s = store();
        let row = TransferRow {
            transfer_id: "t1".into(),
            hash: BlobHash([6u8; 32]),
            state: TransferState::Running,
            part_path: PathBuf::from("/tmp/a.part"),
            dest_path: PathBuf::from("/tmp/a.bin"),
            total: 3,
            segment_hashes: vec![[1u8; 32], [2u8; 32], [3u8; 32]],
            done_segments: vec![false, false, false],
            error: None,
        };
        s.upsert_transfer(&row).unwrap();
        s.mark_segment_done("t1", 1).unwrap();
        let got = s.get_transfer("t1").unwrap().unwrap();
        assert_eq!(got.done_segments, vec![false, true, false]);
        assert_eq!(s.active_transfers().unwrap().len(), 1);
        assert!(s.transfer_for_hash(&BlobHash([6u8; 32])).unwrap().is_some());
    }

    #[test]
    fn finish_preparing_is_atomic() {
        use lanmsg_proto::types::{Envelope, MsgBody, Signature64};
        let s = store();
        let author = DeviceId([2u8; 32]);
        let conv = ConvId([3u8; 32]);
        let m = meta(1);
        let env = Envelope {
            conv,
            id: MsgId { author, seq: 1 },
            conv_seq: 1,
            hlc: Hlc {
                millis: 1,
                counter: 0,
            },
            body: MsgBody::Files {
                items: vec![m.clone()],
            },
            enc: None,
            sig: Signature64([0u8; 64]),
        };
        s.insert_preparing(&PreparingRow {
            author,
            seq: 1,
            conv,
            conv_seq: 1,
            hlc: env.hlc.clone(),
            kind: PreparingKind::Files,
            paths: vec![PathBuf::from("/tmp/a.bin")],
            created_at: 1,
        })
        .unwrap();
        let local = BlobLocalRow {
            hash: m.hash,
            path: PathBuf::from("/tmp/a.bin"),
            kind: BlobLocalKind::Source,
            size: 100,
            mtime: 1,
        };
        s.finish_preparing(
            &author,
            1,
            &env,
            &[(m.hash, vec![[1u8; 32]])],
            std::slice::from_ref(&local),
            2,
        )
        .unwrap();
        assert!(s.list_preparing().unwrap().is_empty());
        assert!(s.get_message(&env.id).unwrap().is_some());
        assert!(s.get_local(&m.hash).unwrap().is_some());
        assert!(s.holders(&m.hash).unwrap().contains(&author));
    }

    #[test]
    fn blob_visible_only_within_shared_conv() {
        use lanmsg_proto::ids::UserId;
        use lanmsg_proto::types::{Envelope, MsgBody, Signature64};
        let s = store();
        let self_user = UserId([1u8; 32]);
        let u = UserId([2u8; 32]);
        let v = UserId([3u8; 32]);
        // DM(self, u) 中引用 H。
        let dm = ConvId::dm(&self_user, &u);
        let m = meta(4);
        let env = Envelope {
            conv: dm,
            id: MsgId {
                author: DeviceId([5u8; 32]),
                seq: 1,
            },
            conv_seq: 1,
            hlc: Hlc {
                millis: 1,
                counter: 0,
            },
            body: MsgBody::Files { items: vec![m] },
            enc: None,
            sig: Signature64([0u8; 64]),
        };
        insert_envelope_on_tx_for_test(&s, &env);
        assert!(s
            .blob_visible_to(&BlobHash([4u8; 32]), &u, &self_user)
            .unwrap());
        assert!(!s
            .blob_visible_to(&BlobHash([4u8; 32]), &v, &self_user)
            .unwrap());
        assert!(s
            .blob_visible_to(&BlobHash([4u8; 32]), &self_user, &self_user)
            .unwrap());
    }

    /// 测试辅助：在无设备行时直接入库一条引用消息。
    fn insert_envelope_on_tx_for_test(s: &Store, env: &Envelope) {
        let conn = s.conn().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO messages(author, seq, conv_id, conv_seq, hlc_millis, hlc_counter, envelope, state, received_at, text, author_user, hidden, deleted)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 'delivered', 1, NULL, ?8, 0, 0)",
            rusqlite::params![
                &env.id.author.0[..],
                env.id.seq as i64,
                &env.conv.0,
                env.conv_seq as i64,
                env.hlc.millis,
                env.hlc.counter as i64,
                lanmsg_proto::to_cbor(env),
                &env.id.author.0[..],
            ],
        )
        .unwrap();
        for item in match &env.body {
            lanmsg_proto::types::MsgBody::Files { items } => items.clone(),
            _ => vec![],
        } {
            conn.execute(
                "INSERT OR IGNORE INTO message_blobs(author, seq, idx, conv_id, hash, name, size, mime, rel_path, hlc_millis, hlc_counter, meta)
                 VALUES(?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10)",
                rusqlite::params![
                    &env.id.author.0[..],
                    env.id.seq as i64,
                    &env.conv.0,
                    &item.hash.0[..],
                    item.name,
                    item.size as i64,
                    item.mime,
                    env.hlc.millis,
                    env.hlc.counter as i64,
                    lanmsg_proto::to_cbor(&item),
                ],
            )
            .unwrap();
        }
    }
}
