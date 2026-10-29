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

    pub fn from_str(s: &str) -> Result<Self, crate::store::StoreError> {
        match s {
            "self_devices" => Ok(ConvKindRow::SelfDevices),
            "dm" => Ok(ConvKindRow::Dm),
            "guest" => Ok(ConvKindRow::Guest),
            "local_send" => Ok(ConvKindRow::LocalSend),
            "station" => Ok(ConvKindRow::Station),
            _ => Err(crate::store::StoreError::Codec(format!(
                "未知会话类型: {s}"
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
                row.conv.to_hex(),
                row.kind.as_str(),
                row.peer_user.as_ref().map(|u| u.to_hex()),
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
        let row: Option<(String, Option<String>, Option<String>, i64)> = conn
            .query_row(
                "SELECT kind, peer_user, title, created_at FROM conversations WHERE conv_id = ?1",
                [conv.to_hex()],
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
        let kind = ConvKindRow::from_str(&kind)?;
        let peer_user = peer_user
            .map(|s| {
                UserId::from_hex(&s).map_err(|e| crate::store::StoreError::Codec(e.to_string()))
            })
            .transpose()?;
        Ok(Some(ConvRow {
            conv: *conv,
            kind,
            peer_user,
            title,
            created_at,
        }))
    }

    /// 列出全部会话。
    pub fn list_conversations(&self) -> Result<Vec<ConvRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt =
            conn.prepare("SELECT conv_id, kind, peer_user, title, created_at FROM conversations")?;
        let rows = stmt.query_map([], |row| {
            let conv_hex: String = row.get(0)?;
            let conv = ConvId::from_hex(&conv_hex).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let kind: String = row.get(1)?;
            let kind = ConvKindRow::from_str(&kind).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let peer_user: Option<String> = row.get(2)?;
            let peer_user = peer_user
                .map(|s| {
                    UserId::from_hex(&s).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })
                })
                .transpose()?;
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
}
