// P5 跨网段连接：已知地址（known_addresses）与手动添加的对端（manual_peers）。
// 注释中文，标识符英文。

use std::net::SocketAddr;

use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::types::PathClass;

use crate::store::{Store, StoreError};

/// 每台设备最多保留的地址条数。
pub const MAX_ADDRS_PER_DEVICE: usize = 8;

/// 每台设备最多保留的地址簿交换来源（Exchanged）地址条数。
pub const MAX_EXCHANGED_PER_DEVICE: usize = 4;

/// 手动对端条目上限。
pub const MAX_MANUAL_PEERS: usize = 32;

/// 地址来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrSource {
    /// 本机直连成功过。
    Direct,
    /// 经地址簿从其他设备得到。
    Exchanged,
    /// 用户手动添加。
    Manual,
}

impl AddrSource {
    fn as_str(&self) -> &'static str {
        match self {
            AddrSource::Direct => "direct",
            AddrSource::Exchanged => "exchanged",
            AddrSource::Manual => "manual",
        }
    }

    fn parse(s: &str) -> AddrSource {
        match s {
            "exchanged" => AddrSource::Exchanged,
            "manual" => AddrSource::Manual,
            _ => AddrSource::Direct,
        }
    }
}

/// known_addresses 表行。
#[derive(Debug, Clone, PartialEq)]
pub struct KnownAddrRow {
    pub device: DeviceId,
    pub addr: SocketAddr,
    pub path: PathClass,
    pub last_ok: i64,
    pub source: AddrSource,
}

/// 路径优先级：数字越小越优先。
fn path_rank(p: &PathClass) -> u8 {
    match p {
        PathClass::Lan => 0,
        PathClass::Vpn => 1,
        PathClass::Internet => 2,
        PathClass::Relay => 3,
    }
}

fn parse_path(s: &str) -> PathClass {
    match s {
        "Lan" => PathClass::Lan,
        "Vpn" => PathClass::Vpn,
        "Relay" => PathClass::Relay,
        _ => PathClass::Internet,
    }
}

fn lock_err<T>(e: std::sync::PoisonError<T>) -> StoreError {
    StoreError::Codec(e.to_string())
}

impl Store {
    /// 写入（upsert）一条已知地址，规则见 [`Store::record_addresses`]。
    pub fn record_address(&self, row: &KnownAddrRow) -> Result<(), StoreError> {
        self.record_addresses(std::slice::from_ref(row))
    }

    /// 在一个事务内批量写入已知地址。每台设备：Exchanged 最多 4 条（保留最近的），
    /// 总数最多 8 条，淘汰时优先丢 Exchanged，其次丢最旧的 Direct/Manual。
    pub fn record_addresses(&self, rows: &[KnownAddrRow]) -> Result<(), StoreError> {
        let mut conn = self.conn().map_err(lock_err)?;
        let tx = conn.transaction()?;
        let mut devices: Vec<DeviceId> = Vec::new();
        for row in rows {
            tx.execute(
                "INSERT INTO known_addresses(device_id, addr, path, last_ok, source)
                 VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(device_id, addr) DO UPDATE SET
                   path = CASE WHEN known_addresses.source = 'direct' AND excluded.source != 'direct'
                               THEN known_addresses.path ELSE excluded.path END,
                   last_ok = CASE WHEN known_addresses.source = 'direct' AND excluded.source != 'direct'
                                  THEN known_addresses.last_ok
                                  ELSE MAX(known_addresses.last_ok, excluded.last_ok) END,
                   source = CASE WHEN known_addresses.source = 'direct' AND excluded.source != 'direct'
                                 THEN known_addresses.source ELSE excluded.source END",
                rusqlite::params![
                    &row.device.0[..],
                    row.addr.to_string(),
                    format!("{:?}", row.path),
                    row.last_ok,
                    row.source.as_str(),
                ],
            )?;
            if !devices.contains(&row.device) {
                devices.push(row.device);
            }
        }
        for device in devices {
            // 先限制 Exchanged 条数，再限制总数（受保护的 Direct/Manual 排在 Exchanged 之前）。
            tx.execute(
                "DELETE FROM known_addresses WHERE device_id = ?1 AND source = 'exchanged'
                   AND addr NOT IN (
                     SELECT addr FROM known_addresses WHERE device_id = ?1 AND source = 'exchanged'
                     ORDER BY last_ok DESC, addr LIMIT ?2)",
                rusqlite::params![&device.0[..], MAX_EXCHANGED_PER_DEVICE as i64],
            )?;
            tx.execute(
                "DELETE FROM known_addresses WHERE device_id = ?1 AND addr NOT IN (
                   SELECT addr FROM known_addresses WHERE device_id = ?1
                   ORDER BY (source = 'exchanged') ASC, last_ok DESC, addr LIMIT ?2)",
                rusqlite::params![&device.0[..], MAX_ADDRS_PER_DEVICE as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 某设备的已知地址：先按路径优先级（Lan < Vpn < Internet < Relay），同路径下
    /// Direct/Manual 优先于 Exchanged，最后按 `last_ok` 降序。
    pub fn addresses_of(&self, device: &DeviceId) -> Result<Vec<KnownAddrRow>, StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        let mut stmt = conn.prepare(
            "SELECT addr, path, last_ok, source FROM known_addresses WHERE device_id = ?1",
        )?;
        let rows = stmt.query_map([&device.0[..]], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (addr, path, last_ok, source) = r?;
            // 无法解析的地址行直接跳过。
            let Ok(addr) = addr.parse::<SocketAddr>() else {
                continue;
            };
            out.push(KnownAddrRow {
                device: *device,
                addr,
                path: parse_path(&path),
                last_ok,
                source: AddrSource::parse(&source),
            });
        }
        out.sort_by(|a, b| {
            path_rank(&a.path)
                .cmp(&path_rank(&b.path))
                .then((a.source == AddrSource::Exchanged).cmp(&(b.source == AddrSource::Exchanged)))
                .then(b.last_ok.cmp(&a.last_ok))
        });
        Ok(out)
    }

    /// 登记手动添加的对端地址（已存在则保持原记录）。
    pub fn add_manual_peer(&self, addr: &str, now: i64) -> Result<(), StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        conn.execute(
            "INSERT OR IGNORE INTO manual_peers(addr, added_at, device_id) VALUES(?1, ?2, NULL)",
            rusqlite::params![addr, now],
        )?;
        Ok(())
    }

    /// 全部手动对端：（地址, 已绑定的设备）。
    pub fn list_manual_peers(&self) -> Result<Vec<(String, Option<DeviceId>)>, StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        let mut stmt =
            conn.prepare("SELECT addr, device_id FROM manual_peers ORDER BY added_at, addr")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<Vec<u8>>>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (addr, dev) = r?;
            let dev = dev.and_then(|b| <[u8; 32]>::try_from(b).ok()).map(DeviceId);
            out.push((addr, dev));
        }
        Ok(out)
    }

    /// 把手动对端地址绑定到连上的设备；地址不在 manual_peers 中则什么都不做。
    /// 首次绑定与改绑都会记一条日志。
    pub fn bind_manual_peer(&self, addr: &str, device: &DeviceId) -> Result<(), StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self.conn().map_err(lock_err)?;
        let current: Option<Option<Vec<u8>>> = conn
            .query_row(
                "SELECT device_id FROM manual_peers WHERE addr = ?1",
                [addr],
                |r| r.get(0),
            )
            .optional()?;
        let Some(current) = current else {
            return Ok(());
        };
        if current.as_deref() == Some(&device.0[..]) {
            return Ok(());
        }
        conn.execute(
            "UPDATE manual_peers SET device_id = ?2 WHERE addr = ?1",
            rusqlite::params![addr, &device.0[..]],
        )?;
        if current.is_some() {
            tracing::info!(%addr, %device, "manual peer rebound to a different device");
        } else {
            tracing::info!(%addr, %device, "manual peer bound to device");
        }
        Ok(())
    }

    /// 删除手动对端，返回是否存在。
    pub fn remove_manual_peer(&self, addr: &str) -> Result<bool, StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        let n = conn.execute("DELETE FROM manual_peers WHERE addr = ?1", [addr])?;
        Ok(n > 0)
    }

    /// 该设备是否由用户手动添加过。
    pub fn is_manual_device(&self, device: &DeviceId) -> Result<bool, StoreError> {
        let conn = self.conn().map_err(lock_err)?;
        let n: i64 = conn.query_row(
            "SELECT count(*) FROM manual_peers WHERE device_id = ?1",
            [&device.0[..]],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(dev: u8, addr: &str, path: PathClass, last_ok: i64) -> KnownAddrRow {
        KnownAddrRow {
            device: DeviceId([dev; 32]),
            addr: addr.parse().unwrap(),
            path,
            last_ok,
            source: AddrSource::Direct,
        }
    }

    #[test]
    fn addresses_ordered_by_path_then_recency() {
        let s = Store::open_in_memory().unwrap();
        s.record_address(&row(1, "10.0.0.2:1", PathClass::Vpn, 50))
            .unwrap();
        s.record_address(&row(1, "192.168.1.2:1", PathClass::Lan, 10))
            .unwrap();
        s.record_address(&row(1, "192.168.1.3:1", PathClass::Lan, 20))
            .unwrap();
        let got: Vec<String> = s
            .addresses_of(&DeviceId([1; 32]))
            .unwrap()
            .iter()
            .map(|r| r.addr.to_string())
            .collect();
        assert_eq!(got, ["192.168.1.3:1", "192.168.1.2:1", "10.0.0.2:1"]);
    }

    #[test]
    fn exchanged_does_not_overwrite_direct_record() {
        let s = Store::open_in_memory().unwrap();
        let d = DeviceId([1; 32]);
        s.record_address(&row(1, "10.0.0.2:1", PathClass::Vpn, 50))
            .unwrap();
        let mut ex = row(1, "10.0.0.2:1", PathClass::Lan, 99);
        ex.source = AddrSource::Exchanged;
        s.record_address(&ex).unwrap();
        let got = s.addresses_of(&d).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, PathClass::Vpn);
        assert_eq!(got[0].last_ok, 50);
        assert_eq!(got[0].source, AddrSource::Direct);
    }

    #[test]
    fn keeps_only_eight_newest_addresses() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..12u8 {
            s.record_address(&row(
                1,
                &format!("10.0.0.{}:1", i + 1),
                PathClass::Vpn,
                i as i64,
            ))
            .unwrap();
        }
        let rows = s.addresses_of(&DeviceId([1; 32])).unwrap();
        assert_eq!(rows.len(), MAX_ADDRS_PER_DEVICE);
        assert!(rows.iter().all(|r| r.last_ok >= 4));
    }

    #[test]
    fn manual_peer_bind_and_lookup() {
        let s = Store::open_in_memory().unwrap();
        s.add_manual_peer("10.0.0.5:53320", 1).unwrap();
        s.add_manual_peer("10.0.0.5:53320", 2).unwrap();
        assert_eq!(
            s.list_manual_peers().unwrap(),
            vec![("10.0.0.5:53320".to_string(), None)]
        );
        let d = DeviceId([7; 32]);
        assert!(!s.is_manual_device(&d).unwrap());
        s.bind_manual_peer("10.0.0.5:53320", &d).unwrap();
        assert!(s.is_manual_device(&d).unwrap());
        // 绑定不存在的地址不会凭空创建记录。
        s.bind_manual_peer("9.9.9.9:1", &DeviceId([8; 32])).unwrap();
        assert!(!s.is_manual_device(&DeviceId([8; 32])).unwrap());
    }

    fn row_src(dev: u8, addr: &str, last_ok: i64, source: AddrSource) -> KnownAddrRow {
        let mut r = row(dev, addr, PathClass::Vpn, last_ok);
        r.source = source;
        r
    }

    #[test]
    fn exchanged_capped_at_four_and_direct_protected() {
        let s = Store::open_in_memory().unwrap();
        let d = DeviceId([1; 32]);
        for i in 0..6u8 {
            s.record_address(&row_src(
                1,
                &format!("10.0.0.{}:1", i + 1),
                100 + i as i64,
                AddrSource::Exchanged,
            ))
            .unwrap();
        }
        let rows = s.addresses_of(&d).unwrap();
        assert_eq!(rows.len(), MAX_EXCHANGED_PER_DEVICE);
        // 8 条 Direct/Manual 之后，Exchanged 先被淘汰。
        let mut batch = Vec::new();
        for i in 0..8u8 {
            batch.push(row_src(
                1,
                &format!("172.16.0.{}:1", i + 1),
                i as i64,
                AddrSource::Direct,
            ));
        }
        s.record_addresses(&batch).unwrap();
        let rows = s.addresses_of(&d).unwrap();
        assert_eq!(rows.len(), MAX_ADDRS_PER_DEVICE);
        assert!(rows.iter().all(|r| r.source == AddrSource::Direct));
    }

    #[test]
    fn direct_sorts_before_exchanged_on_same_path() {
        let s = Store::open_in_memory().unwrap();
        s.record_address(&row_src(1, "10.0.0.1:1", 500, AddrSource::Exchanged))
            .unwrap();
        s.record_address(&row_src(1, "10.0.0.2:1", 1, AddrSource::Direct))
            .unwrap();
        let rows = s.addresses_of(&DeviceId([1; 32])).unwrap();
        assert_eq!(rows[0].addr.to_string(), "10.0.0.2:1");
    }

    #[test]
    fn remove_manual_peer_deletes_binding() {
        let s = Store::open_in_memory().unwrap();
        s.add_manual_peer("host.example:53320", 1).unwrap();
        let d = DeviceId([5; 32]);
        s.bind_manual_peer("host.example:53320", &d).unwrap();
        assert!(s.is_manual_device(&d).unwrap());
        assert!(s.remove_manual_peer("host.example:53320").unwrap());
        assert!(!s.is_manual_device(&d).unwrap());
        assert!(!s.remove_manual_peer("host.example:53320").unwrap());
    }
}
