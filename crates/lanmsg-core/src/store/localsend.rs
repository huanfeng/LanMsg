// P5 LocalSend 桥：已发现 LocalSend 对端的持久化（localsend_peers 表）。
// 指纹一律小写后入库；伪设备 ID 只用作消息作者标识。
// 注释中文，标识符英文。

use lanmsg_proto::ids::{ConvId, DeviceId};
use rusqlite::{params, OptionalExtension, Row};

use crate::store::{Store, StoreError};

/// 对端在这段时间内被发现过，会话视为在线（毫秒）。
pub const LS_ONLINE_WINDOW_MS: i64 = 90_000;

/// localsend_peers 表行。
#[derive(Debug, Clone, PartialEq)]
pub struct LsPeerRow {
    pub fingerprint: String,
    pub pseudo_device: DeviceId,
    pub alias: String,
    pub device_model: Option<String>,
    pub device_type: Option<String>,
    pub last_addr: Option<String>,
    pub protocol: String,
    pub port: u16,
    pub last_seen: i64,
    pub trusted: bool,
    pub blocked: bool,
}

/// 伪设备 ID：blake3("lanmsg-ls-dev" || 小写指纹)。
pub fn pseudo_device_id(fingerprint: &str) -> DeviceId {
    let mut h = blake3::Hasher::new();
    h.update(b"lanmsg-ls-dev");
    h.update(fingerprint.as_bytes());
    DeviceId(*h.finalize().as_bytes())
}

const COLS: &str = "fingerprint, pseudo_device, alias, device_model, device_type, last_addr, protocol, port, last_seen, trusted, blocked";

fn read_row(r: &Row<'_>) -> rusqlite::Result<LsPeerRow> {
    let pseudo: Vec<u8> = r.get(1)?;
    let mut id = [0u8; 32];
    if pseudo.len() == 32 {
        id.copy_from_slice(&pseudo);
    }
    Ok(LsPeerRow {
        fingerprint: r.get(0)?,
        pseudo_device: DeviceId(id),
        alias: r.get(2)?,
        device_model: r.get(3)?,
        device_type: r.get(4)?,
        last_addr: r.get(5)?,
        protocol: r.get(6)?,
        port: r.get::<_, i64>(7)? as u16,
        last_seen: r.get(8)?,
        trusted: r.get::<_, i64>(9)? != 0,
        blocked: r.get::<_, i64>(10)? != 0,
    })
}

fn lock_err<T>(e: std::sync::PoisonError<T>) -> StoreError {
    StoreError::Codec(e.to_string())
}

impl Store {
    /// 插入或更新对端；已存在时保留 `trusted` / `blocked`（只能经 `set_ls_trust` 修改）。
    pub fn upsert_ls_peer(&self, p: &LsPeerRow) -> Result<(), StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        conn.execute(
            "INSERT INTO localsend_peers(fingerprint, pseudo_device, alias, device_model, device_type,
               last_addr, protocol, port, last_seen, trusted, blocked)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(fingerprint) DO UPDATE SET alias = excluded.alias,
               device_model = excluded.device_model, device_type = excluded.device_type,
               last_addr = excluded.last_addr, protocol = excluded.protocol, port = excluded.port,
               last_seen = excluded.last_seen",
            params![
                p.fingerprint,
                &p.pseudo_device.0[..],
                p.alias,
                p.device_model,
                p.device_type,
                p.last_addr,
                p.protocol,
                p.port as i64,
                p.last_seen,
                p.trusted as i64,
                p.blocked as i64,
            ],
        )?;
        Ok(())
    }

    /// 按（小写）指纹查询对端。
    pub fn get_ls_peer(&self, fingerprint: &str) -> Result<Option<LsPeerRow>, StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        Ok(conn
            .query_row(
                &format!("SELECT {COLS} FROM localsend_peers WHERE fingerprint = ?1"),
                [fingerprint],
                read_row,
            )
            .optional()?)
    }

    /// 按伪设备 ID 查询对端。
    pub fn ls_peer_by_pseudo(&self, device: &DeviceId) -> Result<Option<LsPeerRow>, StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        Ok(conn
            .query_row(
                &format!("SELECT {COLS} FROM localsend_peers WHERE pseudo_device = ?1"),
                [&device.0[..]],
                read_row,
            )
            .optional()?)
    }

    /// 设置信任 / 屏蔽标记；对端不存在时无操作。
    pub fn set_ls_trust(
        &self,
        fingerprint: &str,
        trusted: bool,
        blocked: bool,
    ) -> Result<(), StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        conn.execute(
            "UPDATE localsend_peers SET trusted = ?2, blocked = ?3 WHERE fingerprint = ?1",
            params![fingerprint, trusted as i64, blocked as i64],
        )?;
        Ok(())
    }

    /// 对端总数（用于限制陌生对端的入库数量）。
    pub fn count_ls_peers(&self) -> Result<u64, StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        Ok(
            conn.query_row("SELECT COUNT(*) FROM localsend_peers", [], |r| {
                r.get::<_, i64>(0)
            })? as u64,
        )
    }

    /// 该 LocalSend 会话的对端近期（`LS_ONLINE_WINDOW_MS` 内）是否被发现过。
    /// 会话 ID 由指纹单向派生，这里在近期对端中逐个比对。
    pub fn ls_conv_online(&self, conv: &ConvId, now: i64) -> bool {
        self.recent_ls_fingerprints(now)
            .iter()
            .any(|fp| ConvId::localsend(fp) == *conv)
    }

    fn recent_ls_fingerprints(&self, now: i64) -> Vec<String> {
        let Ok(conn) = self.conn() else {
            return Vec::new();
        };
        let Ok(mut stmt) =
            conn.prepare("SELECT fingerprint FROM localsend_peers WHERE last_seen > ?1")
        else {
            return Vec::new();
        };
        let rows = stmt.query_map([now - LS_ONLINE_WINDOW_MS], |r| r.get::<_, String>(0));
        match rows {
            Ok(it) => it.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// 修改会话标题（LocalSend 对端改名时同步）。
    pub fn set_conversation_title(&self, conv: &ConvId, title: &str) -> Result<(), StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        conn.execute(
            "UPDATE conversations SET title = ?2 WHERE conv_id = ?1",
            params![&conv.0[..], title],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(fp: &str, seen: i64) -> LsPeerRow {
        LsPeerRow {
            fingerprint: fp.into(),
            pseudo_device: pseudo_device_id(fp),
            alias: "Phone".into(),
            device_model: None,
            device_type: Some("mobile".into()),
            last_addr: Some("192.168.1.9".into()),
            protocol: "https".into(),
            port: 53317,
            last_seen: seen,
            trusted: false,
            blocked: false,
        }
    }

    #[test]
    fn upsert_keeps_trust_and_lookup_by_pseudo() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_ls_peer(&row("aa", 1)).unwrap();
        s.set_ls_trust("aa", true, false).unwrap();
        let mut again = row("aa", 2);
        again.alias = "Renamed".into();
        s.upsert_ls_peer(&again).unwrap();
        let got = s.get_ls_peer("aa").unwrap().unwrap();
        assert!(got.trusted);
        assert_eq!(got.alias, "Renamed");
        assert_eq!(got.last_seen, 2);
        let by = s
            .ls_peer_by_pseudo(&pseudo_device_id("aa"))
            .unwrap()
            .unwrap();
        assert_eq!(by.fingerprint, "aa");
        assert_eq!(s.count_ls_peers().unwrap(), 1);
    }

    #[test]
    fn conv_online_follows_last_seen_window() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_ls_peer(&row("bb", 1_000_000)).unwrap();
        let conv = ConvId::localsend("bb");
        assert!(s.ls_conv_online(&conv, 1_000_000 + LS_ONLINE_WINDOW_MS - 1));
        assert!(!s.ls_conv_online(&conv, 1_000_000 + LS_ONLINE_WINDOW_MS + 1));
        assert!(!s.ls_conv_online(&ConvId::localsend("cc"), 1_000_000));
    }
}
