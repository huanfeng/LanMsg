// P1 核心骨架：会话存储（Task 6）。
// ConvKindRow 存为小写字符串。
// 注释中文，标识符英文。

use crate::store::Store;
use lanmsg_proto::ids::{ConvId, UserId};

/// 会话类型行，DB 中存为小写字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConvKindRow {
    SelfDevices,
    Dm,
    Guest,
    LocalSend,
    Station,
}

impl ConvKindRow {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConvKindRow::SelfDevices => "self_devices",
            ConvKindRow::Dm => "dm",
            ConvKindRow::Guest => "guest",
            ConvKindRow::LocalSend => "local_send",
            ConvKindRow::Station => "station",
        }
    }

    pub fn parse(s: &str) -> Result<Self, crate::store::StoreError> {
        match s {
            "self_devices" => Ok(ConvKindRow::SelfDevices),
            "dm" => Ok(ConvKindRow::Dm),
            "guest" => Ok(ConvKindRow::Guest),
            "local_send" => Ok(ConvKindRow::LocalSend),
            "station" => Ok(ConvKindRow::Station),
            _ => Err(crate::store::StoreError::Codec(format!(
                "unknown conversation kind: {s}"
            ))),
        }
    }
}

/// 会话表行。
#[derive(Debug, Clone, PartialEq)]
pub struct ConvRow {
    pub conv: ConvId,
    pub kind: ConvKindRow,
    pub peer_user: Option<UserId>,
    pub title: Option<String>,
    pub created_at: i64,
}

/// get_conversation 的查询行元组：(kind, peer_user, title, created_at)。
type ConversationRow = (String, Option<Vec<u8>>, Option<String>, i64);

impl Store {
    /// 插入会话，已存在不覆盖。
    pub fn ensure_conversation(&self, row: &ConvRow) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "INSERT INTO conversations(conv_id, kind, peer_user, title, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(conv_id) DO NOTHING",
            rusqlite::params![
                &row.conv.0[..],
                row.kind.as_str(),
                row.peer_user.as_ref().map(|u| &u.0[..]),
                row.title,
                row.created_at,
            ],
        )?;
        Ok(())
    }

    /// 查询会话，不存在返回 Ok(None)。
    pub fn get_conversation(
        &self,
        conv: &ConvId,
    ) -> Result<Option<ConvRow>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<ConversationRow> = conn
            .query_row(
                "SELECT kind, peer_user, title, created_at FROM conversations WHERE conv_id = ?1",
                [&conv.0[..]],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, peer_user, title, created_at)) = row else {
            return Ok(None);
        };
        let kind = ConvKindRow::parse(&kind)?;
        let peer_user = match peer_user {
            Some(bytes) => Some(UserId(bytes.try_into().map_err(|bytes: Vec<u8>| {
                crate::store::StoreError::Codec(format!(
                    "peer_user invalid length: {}",
                    bytes.len()
                ))
            })?)),
            None => None,
        };
        Ok(Some(ConvRow {
            conv: *conv,
            kind,
            peer_user,
            title,
            created_at,
        }))
    }

    /// 列出全部 `gate = 'open'` 的会话（Pending/Rejected 不进列表）。
    pub fn list_conversations(&self) -> Result<Vec<ConvRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT conv_id, kind, peer_user, title, created_at FROM conversations WHERE gate = 'open'",
        )?;
        let rows = stmt.query_map([], |row| {
            let conv_bytes: Vec<u8> = row.get(0)?;
            let conv = ConvId(conv_bytes.try_into().map_err(|bytes: Vec<u8>| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Blob,
                    format!("conv_id invalid length: {}", bytes.len()).into(),
                )
            })?);
            let kind: String = row.get(1)?;
            let kind = ConvKindRow::parse(&kind).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let peer_user: Option<Vec<u8>> = row.get(2)?;
            let peer_user = match peer_user {
                Some(bytes) => Some(UserId(bytes.try_into().map_err(|bytes: Vec<u8>| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Blob,
                        format!("peer_user invalid length: {}", bytes.len()).into(),
                    )
                })?)),
                None => None,
            };
            let title: Option<String> = row.get(3)?;
            let created_at: i64 = row.get(4)?;
            Ok(ConvRow {
                conv,
                kind,
                peer_user,
                title,
                created_at,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?);
        }
        Ok(out)
    }

    /// 设置会话准入门状态与失效时间。
    pub fn set_gate(
        &self,
        conv: &ConvId,
        gate: Gate,
        gate_until: Option<i64>,
    ) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "UPDATE conversations SET gate = ?1, gate_until = ?2 WHERE conv_id = ?3",
            rusqlite::params![gate.as_str(), gate_until, &conv.0[..]],
        )?;
        Ok(())
    }

    /// 查询准入门状态，会话不存在返回默认 Open。
    pub fn get_gate(&self, conv: &ConvId) -> Result<(Gate, Option<i64>), crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<(String, Option<i64>)> = conn
            .query_row(
                "SELECT gate, gate_until FROM conversations WHERE conv_id = ?1",
                [&conv.0[..]],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((gate, until)) => Ok((Gate::parse(&gate)?, until)),
            None => Ok((Gate::Open, None)),
        }
    }

    /// 更新会话已读水位：仅当 HLC 严格更新才写，返回是否变化。
    pub fn set_last_read(
        &self,
        conv: &ConvId,
        hlc: lanmsg_proto::types::Hlc,
    ) -> Result<bool, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let n = conn.execute(
            "UPDATE conversations SET last_read_millis = ?1, last_read_counter = ?2
             WHERE conv_id = ?3
               AND (last_read_millis < ?1 OR (last_read_millis = ?1 AND last_read_counter < ?2))",
            rusqlite::params![hlc.millis, hlc.counter as i64, &conv.0[..]],
        )?;
        Ok(n > 0)
    }

    /// 读取会话已读水位；会话不存在返回零值。
    pub fn last_read(
        &self,
        conv: &ConvId,
    ) -> Result<lanmsg_proto::types::Hlc, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT last_read_millis, last_read_counter FROM conversations WHERE conv_id = ?1",
                [&conv.0[..]],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (millis, counter) = row.unwrap_or((0, 0));
        Ok(lanmsg_proto::types::Hlc {
            millis,
            counter: counter as u32,
        })
    }
}

/// 会话准入门，DB 中存为小写字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Open,
    Pending,
    Rejected,
}

impl Gate {
    pub fn as_str(&self) -> &'static str {
        match self {
            Gate::Open => "open",
            Gate::Pending => "pending",
            Gate::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Result<Self, crate::store::StoreError> {
        match s {
            "open" => Ok(Gate::Open),
            "pending" => Ok(Gate::Pending),
            "rejected" => Ok(Gate::Rejected),
            _ => Err(crate::store::StoreError::Codec(format!(
                "unknown admission state: {s}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn conv() -> ConvId {
        ConvId([9u8; 32])
    }

    fn user() -> UserId {
        UserId([5u8; 32])
    }

    #[test]
    fn ensure_and_get_conversation() {
        let s = store();
        let row = ConvRow {
            conv: conv(),
            kind: ConvKindRow::Dm,
            peer_user: Some(user()),
            title: Some("dm".into()),
            created_at: 1,
        };
        s.ensure_conversation(&row).unwrap();
        let got = s.get_conversation(&conv()).unwrap().unwrap();
        assert_eq!(got.kind, ConvKindRow::Dm);
        assert_eq!(got.title, Some("dm".into()));
        assert_eq!(s.get_conversation(&ConvId([0u8; 32])).unwrap(), None);
    }

    #[test]
    fn ensure_conversation_no_overwrite() {
        let s = store();
        let row = ConvRow {
            conv: conv(),
            kind: ConvKindRow::Dm,
            peer_user: Some(user()),
            title: Some("old".into()),
            created_at: 1,
        };
        s.ensure_conversation(&row).unwrap();
        let row2 = ConvRow {
            conv: conv(),
            kind: ConvKindRow::Guest,
            peer_user: None,
            title: Some("new".into()),
            created_at: 2,
        };
        s.ensure_conversation(&row2).unwrap();
        let got = s.get_conversation(&conv()).unwrap().unwrap();
        assert_eq!(got.title, Some("old".into()));
    }

    #[test]
    fn list_conversations() {
        let s = store();
        let row = ConvRow {
            conv: conv(),
            kind: ConvKindRow::SelfDevices,
            peer_user: None,
            title: None,
            created_at: 1,
        };
        s.ensure_conversation(&row).unwrap();
        let list = s.list_conversations().unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn list_conversations_hides_pending_gate() {
        let s = store();
        let row = ConvRow {
            conv: conv(),
            kind: ConvKindRow::Dm,
            peer_user: Some(user()),
            title: None,
            created_at: 1,
        };
        s.ensure_conversation(&row).unwrap();
        assert_eq!(s.list_conversations().unwrap().len(), 1);
        s.set_gate(&conv(), Gate::Pending, Some(1000)).unwrap();
        assert_eq!(s.list_conversations().unwrap().len(), 0);
        assert_eq!(s.get_gate(&conv()).unwrap(), (Gate::Pending, Some(1000)));
        s.set_gate(&conv(), Gate::Open, None).unwrap();
        assert_eq!(s.list_conversations().unwrap().len(), 1);
        // last_read 仅严格更新。
        let h1 = lanmsg_proto::types::Hlc {
            millis: 5,
            counter: 0,
        };
        assert!(s.set_last_read(&conv(), h1.clone()).unwrap());
        assert!(!s.set_last_read(&conv(), h1).unwrap());
        assert_eq!(
            s.last_read(&conv()).unwrap(),
            lanmsg_proto::types::Hlc {
                millis: 5,
                counter: 0
            }
        );
    }
}
