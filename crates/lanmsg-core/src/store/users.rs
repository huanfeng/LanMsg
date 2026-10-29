// P1 核心骨架：用户与设备存储（Task 6）。
// 注释中文，标识符英文。

use lanmsg_proto::ids::{DeviceId, UserId};
use lanmsg_proto::types::{DeviceCert, DeviceType, PathClass};

use crate::store::Store;

/// 用户表行。
#[derive(Debug, Clone, PartialEq)]
pub struct UserRow {
    pub user: UserId,
    pub display_name: String,
    pub is_self: bool,
    pub updated_at: i64,
}

/// 设备表行。
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRow {
    pub device: DeviceId,
    pub user: UserId,
    pub cert: DeviceCert,
    pub name: String,
    pub device_type: DeviceType,
    pub last_addr: Option<String>,
    pub last_path: Option<PathClass>,
    pub last_seen: Option<i64>,
    pub revoked: bool,
}

impl Store {
    /// 插入或更新用户（按 user_id）。
    pub fn upsert_user(&self, row: &UserRow) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "INSERT INTO users(user_id, display_name, is_self, updated_at)
             VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(user_id) DO UPDATE SET
               display_name = excluded.display_name,
               is_self = excluded.is_self,
               updated_at = excluded.updated_at",
            rusqlite::params![
                row.user.to_hex(),
                row.display_name,
                row.is_self as i64,
                row.updated_at,
            ],
        )?;
        Ok(())
    }

    /// 查询用户，不存在返回 Ok(None)。
    pub fn get_user(&self, user: &UserId) -> Result<Option<UserRow>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<(String, i64, i64)> = conn
            .query_row(
                "SELECT display_name, is_self, updated_at FROM users WHERE user_id = ?1",
                [user.to_hex()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((display_name, is_self, updated_at)) = row else {
            return Ok(None);
        };
        Ok(Some(UserRow {
            user: *user,
            display_name,
            is_self: is_self != 0,
            updated_at,
        }))
    }

    /// 插入或更新设备（按 device_id）。
    pub fn upsert_device(&self, row: &DeviceRow) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let cert_bytes = lanmsg_proto::to_cbor(&row.cert);
        let last_path = row.last_path.as_ref().map(|p| format!("{p:?}"));
        conn.execute(
            "INSERT INTO devices(device_id, user_id, cert, name, device_type, last_addr, last_path, last_seen, revoked)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(device_id) DO UPDATE SET
               user_id = excluded.user_id,
               cert = excluded.cert,
               name = excluded.name,
               device_type = excluded.device_type,
               last_addr = excluded.last_addr,
               last_path = excluded.last_path,
               last_seen = excluded.last_seen,
               revoked = excluded.revoked",
            rusqlite::params![
                row.device.to_hex(),
                row.user.to_hex(),
                cert_bytes,
                row.name,
                format!("{:?}", row.device_type),
                row.last_addr,
                last_path,
                row.last_seen,
                row.revoked as i64,
            ],
        )?;
        Ok(())
    }

    /// 查询设备，不存在返回 Ok(None)。
    pub fn get_device(
        &self,
        device: &DeviceId,
    ) -> Result<Option<DeviceRow>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<(
            String,
            Vec<u8>,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            i64,
        )> = conn
            .query_row(
                "SELECT user_id, cert, name, device_type, last_addr, last_path, last_seen, revoked
                 FROM devices WHERE device_id = ?1",
                [device.to_hex()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            user_hex,
            cert_bytes,
            name,
            device_type,
            last_addr,
            last_path,
            last_seen,
            revoked,
        )) = row
        else {
            return Ok(None);
        };
        let user = UserId::from_hex(&user_hex)
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let cert: DeviceCert = lanmsg_proto::from_cbor(&cert_bytes)
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let device_type = match device_type.as_str() {
            "Desktop" => DeviceType::Desktop,
            "Laptop" => DeviceType::Laptop,
            "Phone" => DeviceType::Phone,
            "Tablet" => DeviceType::Tablet,
            "Headless" => DeviceType::Headless,
            "Web" => DeviceType::Web,
            _ => {
                return Err(crate::store::StoreError::Codec(format!(
                    "未知设备类型: {device_type}"
                )))
            }
        };
        let last_path = match last_path.as_deref() {
            None => None,
            Some("Lan") => Some(PathClass::Lan),
            Some("Vpn") => Some(PathClass::Vpn),
            Some("Internet") => Some(PathClass::Internet),
            Some("Relay") => Some(PathClass::Relay),
            Some(other) => {
                return Err(crate::store::StoreError::Codec(format!(
                    "未知路径类型: {other}"
                )))
            }
        };
        Ok(Some(DeviceRow {
            device: *device,
            user,
            cert,
            name,
            device_type,
            last_addr,
            last_path,
            last_seen,
            revoked: revoked != 0,
        }))
    }

    /// 列出指定用户的全部设备。
    pub fn devices_of_user(
        &self,
        user: &UserId,
    ) -> Result<Vec<DeviceRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT device_id, cert, name, device_type, last_addr, last_path, last_seen, revoked
             FROM devices WHERE user_id = ?1",
        )?;
        let rows = stmt.query_map([user.to_hex()], |row| {
            let device_hex: String = row.get(0)?;
            let device = DeviceId::from_hex(&device_hex).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let cert_bytes: Vec<u8> = row.get(1)?;
            let cert: DeviceCert = lanmsg_proto::from_cbor(&cert_bytes).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Blob,
                    Box::new(e),
                )
            })?;
            let name: String = row.get(2)?;
            let device_type: String = row.get(3)?;
            let device_type = match device_type.as_str() {
                "Desktop" => DeviceType::Desktop,
                "Laptop" => DeviceType::Laptop,
                "Phone" => DeviceType::Phone,
                "Tablet" => DeviceType::Tablet,
                "Headless" => DeviceType::Headless,
                "Web" => DeviceType::Web,
                _ => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "未知设备类型",
                        )),
                    ))
                }
            };
            let last_addr: Option<String> = row.get(4)?;
            let last_path: Option<String> = row.get(5)?;
            let last_path = match last_path.as_deref() {
                None => None,
                Some("Lan") => Some(PathClass::Lan),
                Some("Vpn") => Some(PathClass::Vpn),
                Some("Internet") => Some(PathClass::Internet),
                Some("Relay") => Some(PathClass::Relay),
                Some(_other) => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "未知路径类型",
                        )),
                    ))
                }
            };
            let last_seen: Option<i64> = row.get(6)?;
            let revoked: i64 = row.get(7)?;
            Ok(DeviceRow {
                device,
                user: *user,
                cert,
                name,
                device_type,
                last_addr,
                last_path,
                last_seen,
                revoked: revoked != 0,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(crate::store::StoreError::Sqlite)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn device_id() -> DeviceId {
        DeviceId([42u8; 32])
    }

    fn user_id() -> UserId {
        UserId([7u8; 32])
    }

    fn cert() -> DeviceCert {
        DeviceCert {
            device: device_id(),
            user: user_id(),
            device_name: "台式机".into(),
            device_type: DeviceType::Desktop,
            created_at: 1_700_000_000_000,
            key_epoch: 0,
            sig: lanmsg_proto::types::Signature64([0u8; 64]),
        }
    }

    #[test]
    fn upsert_and_get_user() {
        let s = store();
        let row = UserRow {
            user: user_id(),
            display_name: "alice".into(),
            is_self: true,
            updated_at: 1,
        };
        s.upsert_user(&row).unwrap();
        let got = s.get_user(&user_id()).unwrap().unwrap();
        assert_eq!(got.display_name, "alice");
        assert!(got.is_self);
        assert_eq!(got.updated_at, 1);
        assert_eq!(s.get_user(&UserId([0u8; 32])).unwrap(), None);
    }

    #[test]
    fn upsert_and_get_device() {
        let s = store();
        let row = DeviceRow {
            device: device_id(),
            user: user_id(),
            cert: cert(),
            name: "dev1".into(),
            device_type: DeviceType::Desktop,
            last_addr: Some("127.0.0.1".into()),
            last_path: Some(PathClass::Lan),
            last_seen: Some(2),
            revoked: false,
        };
        s.upsert_device(&row).unwrap();
        let got = s.get_device(&device_id()).unwrap().unwrap();
        assert_eq!(got.name, "dev1");
        assert_eq!(got.last_path, Some(PathClass::Lan));
        assert!(!got.revoked);
        assert_eq!(s.get_device(&DeviceId([0u8; 32])).unwrap(), None);
    }

    #[test]
    fn devices_of_user() {
        let s = store();
        let row = DeviceRow {
            device: device_id(),
            user: user_id(),
            cert: cert(),
            name: "dev1".into(),
            device_type: DeviceType::Phone,
            last_addr: None,
            last_path: None,
            last_seen: None,
            revoked: false,
        };
        s.upsert_device(&row).unwrap();
        let devs = s.devices_of_user(&user_id()).unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].device, device_id());
    }
}
