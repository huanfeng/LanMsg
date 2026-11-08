// P5 文件中转站：站点、条目与条目引用的 Blob（station_blobs）。
// content/acl/meta 存 CBOR。已删除条目不保留 station_blobs 行，
// 因此其 Blob 不再对外可见。
// 注释中文，标识符英文。

use std::path::PathBuf;

use lanmsg_proto::ids::{BlobHash, ConvId, DeviceId, UserId};
use lanmsg_proto::station::{ItemId, StationContent, StationInfo, StationItem};
use lanmsg_proto::types::BlobMeta;
use rusqlite::OptionalExtension;

use crate::store::{Store, StoreError};

/// 条目的存储模式（存为 reference / copy）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreModeRow {
    Reference,
    Copy,
}

impl StoreModeRow {
    fn as_str(&self) -> &'static str {
        match self {
            StoreModeRow::Reference => "reference",
            StoreModeRow::Copy => "copy",
        }
    }

    fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "reference" => Ok(StoreModeRow::Reference),
            "copy" => Ok(StoreModeRow::Copy),
            _ => Err(StoreError::Codec(format!("unknown store mode: {s}"))),
        }
    }
}

/// 条目行：协议条目加主机本地的存储信息。
#[derive(Debug, Clone, PartialEq)]
pub struct StationItemRow {
    pub item: StationItem,
    pub station: ConvId,
    pub store_mode: Option<StoreModeRow>,
    pub local_path: Option<PathBuf>,
    pub local_size: Option<u64>,
    pub local_mtime: Option<i64>,
}

fn codec<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Codec(e.to_string())
}

fn arr<const N: usize>(v: Vec<u8>, what: &str) -> Result<[u8; N], StoreError> {
    v.try_into()
        .map_err(|b: Vec<u8>| StoreError::Codec(format!("{what} invalid length: {}", b.len())))
}

type StationTuple = (Vec<u8>, String, Vec<u8>, Vec<u8>, Vec<u8>, i64, bool);

fn station_from_tuple(t: StationTuple) -> Result<StationInfo, StoreError> {
    let (id, name, host, owner, acl, created_at, deleted) = t;
    Ok(StationInfo {
        station: ConvId(arr(id, "station_id")?),
        name,
        host: DeviceId(arr(host, "host_device")?),
        owner: UserId(arr(owner, "owner_user")?),
        acl: lanmsg_proto::from_cbor(&acl).map_err(codec)?,
        created_at,
        deleted,
    })
}

const STATION_COLS: &str = "station_id, name, host_device, owner_user, acl, created_at, deleted";

fn station_tuple(r: &rusqlite::Row<'_>) -> rusqlite::Result<StationTuple> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
    ))
}

const ITEM_COLS: &str = "item_id, station_id, added_by, added_at, expires_at, content, pinned,
    removed, available, updated_at, store_mode, local_path, local_size, local_mtime";

fn item_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ItemTuple> {
    Ok(ItemTuple {
        item_id: r.get(0)?,
        station_id: r.get(1)?,
        added_by: r.get(2)?,
        added_at: r.get(3)?,
        expires_at: r.get(4)?,
        content: r.get(5)?,
        pinned: r.get(6)?,
        removed: r.get(7)?,
        available: r.get(8)?,
        updated_at: r.get(9)?,
        store_mode: r.get(10)?,
        local_path: r.get(11)?,
        local_size: r.get(12)?,
        local_mtime: r.get(13)?,
    })
}

struct ItemTuple {
    item_id: Vec<u8>,
    station_id: Vec<u8>,
    added_by: Vec<u8>,
    added_at: i64,
    expires_at: Option<i64>,
    content: Vec<u8>,
    pinned: bool,
    removed: bool,
    available: bool,
    updated_at: i64,
    store_mode: Option<String>,
    local_path: Option<String>,
    local_size: Option<i64>,
    local_mtime: Option<i64>,
}

impl ItemTuple {
    fn into_row(self) -> Result<StationItemRow, StoreError> {
        Ok(StationItemRow {
            item: StationItem {
                item: ItemId(arr(self.item_id, "item_id")?),
                added_by: DeviceId(arr(self.added_by, "added_by")?),
                added_at: self.added_at,
                expires_at: self.expires_at,
                content: lanmsg_proto::from_cbor(&self.content).map_err(codec)?,
                pinned: self.pinned,
                removed: self.removed,
                available: self.available,
                updated_at: self.updated_at,
            },
            station: ConvId(arr(self.station_id, "station_id")?),
            store_mode: self
                .store_mode
                .as_deref()
                .map(StoreModeRow::parse)
                .transpose()?,
            local_path: self.local_path.map(PathBuf::from),
            local_size: self.local_size.map(|v| v as u64),
            local_mtime: self.local_mtime,
        })
    }
}

/// 条目内容引用的 Blob：(rel_path, meta)，Text 为空。
fn content_blobs(content: &StationContent) -> Vec<(String, &BlobMeta)> {
    match content {
        StationContent::Text { .. } => Vec::new(),
        StationContent::File { blob } => vec![(String::new(), blob)],
        StationContent::Folder { entries, .. } => entries
            .iter()
            .map(|e| (e.rel_path.clone(), &e.blob))
            .collect(),
    }
}

impl Store {
    /// 写入或更新站点（按 station_id 覆盖；synced_until 保持不变）。
    pub fn upsert_station(&self, info: &StationInfo) -> Result<(), StoreError> {
        let conn = self.conn().map_err(codec)?;
        conn.execute(
            "INSERT INTO stations(station_id, name, host_device, owner_user, acl, created_at, deleted)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(station_id) DO UPDATE SET name = excluded.name,
               host_device = excluded.host_device, owner_user = excluded.owner_user,
               acl = excluded.acl, deleted = excluded.deleted",
            rusqlite::params![
                &info.station.0[..],
                info.name,
                &info.host.0[..],
                &info.owner.0[..],
                lanmsg_proto::to_cbor(&info.acl),
                info.created_at,
                info.deleted,
            ],
        )?;
        Ok(())
    }

    /// 查询站点。
    pub fn get_station(&self, station: &ConvId) -> Result<Option<StationInfo>, StoreError> {
        let conn = self.conn().map_err(codec)?;
        let t = conn
            .query_row(
                &format!("SELECT {STATION_COLS} FROM stations WHERE station_id = ?1"),
                [&station.0[..]],
                station_tuple,
            )
            .optional()?;
        t.map(station_from_tuple).transpose()
    }

    /// 全部站点（按创建时间升序）。
    pub fn list_stations(&self) -> Result<Vec<StationInfo>, StoreError> {
        let conn = self.conn().map_err(codec)?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {STATION_COLS} FROM stations ORDER BY created_at ASC"
        ))?;
        let rows = stmt.query_map([], station_tuple)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(station_from_tuple(r?)?);
        }
        Ok(out)
    }

    /// 记录已同步到的 updated_at 水位。
    pub fn set_synced_until(&self, station: &ConvId, until: i64) -> Result<(), StoreError> {
        let conn = self.conn().map_err(codec)?;
        conn.execute(
            "UPDATE stations SET synced_until = ?2 WHERE station_id = ?1",
            rusqlite::params![&station.0[..], until],
        )?;
        Ok(())
    }

    /// 已同步水位，站点不存在为 0。
    pub fn synced_until(&self, station: &ConvId) -> Result<i64, StoreError> {
        let conn = self.conn().map_err(codec)?;
        let v: Option<i64> = conn
            .query_row(
                "SELECT synced_until FROM stations WHERE station_id = ?1",
                [&station.0[..]],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.unwrap_or(0))
    }

    /// 写入或更新条目；同一事务内按内容重写 station_blobs
    ///（已删除条目不保留 Blob 行）。
    pub fn upsert_station_item(&self, row: &StationItemRow) -> Result<(), StoreError> {
        let mut conn = self.conn().map_err(codec)?;
        let tx = conn.transaction()?;
        let it = &row.item;
        tx.execute(
            "INSERT INTO station_items(item_id, station_id, added_by, added_at, expires_at, content,
               pinned, removed, available, updated_at, store_mode, local_path, local_size, local_mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(item_id) DO UPDATE SET station_id = excluded.station_id,
               added_by = excluded.added_by, added_at = excluded.added_at,
               expires_at = excluded.expires_at, content = excluded.content,
               pinned = excluded.pinned, removed = excluded.removed,
               available = excluded.available, updated_at = excluded.updated_at,
               store_mode = excluded.store_mode, local_path = excluded.local_path,
               local_size = excluded.local_size, local_mtime = excluded.local_mtime",
            rusqlite::params![
                &it.item.0[..],
                &row.station.0[..],
                &it.added_by.0[..],
                it.added_at,
                it.expires_at,
                lanmsg_proto::to_cbor(&it.content),
                it.pinned,
                it.removed,
                it.available,
                it.updated_at,
                row.store_mode.map(|m| m.as_str()),
                row.local_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                row.local_size.map(|v| v as i64),
                row.local_mtime,
            ],
        )?;
        tx.execute(
            "DELETE FROM station_blobs WHERE item_id = ?1",
            [&it.item.0[..]],
        )?;
        if !it.removed {
            for (rel_path, meta) in content_blobs(&it.content) {
                tx.execute(
                    "INSERT OR REPLACE INTO station_blobs(item_id, hash, name, size, mime, rel_path, meta)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        &it.item.0[..],
                        &meta.hash.0[..],
                        meta.name,
                        meta.size as i64,
                        meta.mime,
                        rel_path,
                        lanmsg_proto::to_cbor(meta),
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 站点内 updated_at > since 的条目（升序）；`include_removed` 为 false 时跳过已删除。
    pub fn station_items(
        &self,
        station: &ConvId,
        since: i64,
        include_removed: bool,
    ) -> Result<Vec<StationItemRow>, StoreError> {
        let conn = self.conn().map_err(codec)?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ITEM_COLS} FROM station_items
             WHERE station_id = ?1 AND updated_at > ?2 AND (?3 OR removed = 0)
             ORDER BY updated_at ASC, added_at ASC"
        ))?;
        let rows = stmt.query_map(
            rusqlite::params![&station.0[..], since, include_removed],
            item_from_row,
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?.into_row()?);
        }
        Ok(out)
    }

    /// 查询单个条目。
    pub fn get_station_item(&self, item: &ItemId) -> Result<Option<StationItemRow>, StoreError> {
        let conn = self.conn().map_err(codec)?;
        let t = conn
            .query_row(
                &format!("SELECT {ITEM_COLS} FROM station_items WHERE item_id = ?1"),
                [&item.0[..]],
                item_from_row,
            )
            .optional()?;
        t.map(ItemTuple::into_row).transpose()
    }

    /// 全部未删除且带有过期时间的条目（过期清理用）。
    pub fn station_items_expiring(&self, now: i64) -> Result<Vec<StationItemRow>, StoreError> {
        let conn = self.conn().map_err(codec)?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ITEM_COLS} FROM station_items
             WHERE removed = 0 AND expires_at IS NOT NULL AND expires_at <= ?1"
        ))?;
        let rows = stmt.query_map([now], item_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?.into_row()?);
        }
        Ok(out)
    }

    /// 全部未删除的引用模式条目（可用性巡检用）。
    pub fn station_reference_items(&self) -> Result<Vec<StationItemRow>, StoreError> {
        let conn = self.conn().map_err(codec)?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ITEM_COLS} FROM station_items
             WHERE removed = 0 AND store_mode = 'reference'"
        ))?;
        let rows = stmt.query_map([], item_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?.into_row()?);
        }
        Ok(out)
    }

    /// 按 Blob 哈希找所属站点：(站点, 元数据, rel_path)，取第一条；
    /// File 条目的 rel_path 为 None。
    pub fn station_blob(
        &self,
        hash: &BlobHash,
    ) -> Result<Option<(ConvId, BlobMeta, Option<String>)>, StoreError> {
        let conn = self.conn().map_err(codec)?;
        let t: Option<(Vec<u8>, Vec<u8>, String)> = conn
            .query_row(
                "SELECT i.station_id, b.meta, b.rel_path FROM station_blobs b
                 JOIN station_items i ON i.item_id = b.item_id
                 WHERE b.hash = ?1 ORDER BY i.added_at ASC LIMIT 1",
                [&hash.0[..]],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((sid, meta, rel)) = t else {
            return Ok(None);
        };
        Ok(Some((
            ConvId(arr(sid, "station_id")?),
            lanmsg_proto::from_cbor(&meta).map_err(codec)?,
            if rel.is_empty() { None } else { Some(rel) },
        )))
    }
}
