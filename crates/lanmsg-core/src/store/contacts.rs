// P2 存储扩展：联系人、请求、吊销（Task 2）。
// ContactState/RequestKindRow 存为小写字符串；Revocation 整条记录存 CBOR。
// 注释中文，标识符英文。

use lanmsg_proto::ids::{DeviceId, UserId};
use lanmsg_proto::types::{Hlc, Revocation};

use crate::store::Store;
use lanmsg_api::types::RequestPreviewView;

/// 联系人状态，DB 中存为小写字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactState {
    Outgoing,
    Incoming,
    Accepted,
    Rejected,
    Blocked,
}

impl ContactState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContactState::Outgoing => "outgoing",
            ContactState::Incoming => "incoming",
            ContactState::Accepted => "accepted",
            ContactState::Rejected => "rejected",
            ContactState::Blocked => "blocked",
        }
    }

    pub fn parse(s: &str) -> Result<Self, crate::store::StoreError> {
        match s {
            "outgoing" => Ok(ContactState::Outgoing),
            "incoming" => Ok(ContactState::Incoming),
            "accepted" => Ok(ContactState::Accepted),
            "rejected" => Ok(ContactState::Rejected),
            "blocked" => Ok(ContactState::Blocked),
            _ => Err(crate::store::StoreError::Codec(format!(
                "unknown contact state: {s}"
            ))),
        }
    }
}

/// 联系人表行。
#[derive(Debug, Clone, PartialEq)]
pub struct ContactRow {
    pub user: UserId,
    pub state: ContactState,
    pub auto_accept: bool,
    pub ask_above_bytes: Option<u64>,
    pub updated: Hlc,
}

/// 请求类型，DB 中存为小写字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKindRow {
    Contact,
    Message,
    File,
}

impl RequestKindRow {
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestKindRow::Contact => "contact",
            RequestKindRow::Message => "message",
            RequestKindRow::File => "file",
        }
    }

    pub fn parse(s: &str) -> Result<Self, crate::store::StoreError> {
        match s {
            "contact" => Ok(RequestKindRow::Contact),
            "message" => Ok(RequestKindRow::Message),
            "file" => Ok(RequestKindRow::File),
            _ => Err(crate::store::StoreError::Codec(format!(
                "unknown request kind: {s}"
            ))),
        }
    }
}

/// 待处理请求表行。
#[derive(Debug, Clone, PartialEq)]
pub struct RequestRow {
    pub request_id: String,
    pub kind: RequestKindRow,
    pub from_user: UserId,
    pub conv: Option<lanmsg_proto::ids::ConvId>,
    pub preview: RequestPreviewView,
    pub created_at: i64,
}

/// 预览写库：CBOR 的十六进制文本（列类型保持 TEXT）。
fn preview_to_db(p: &RequestPreviewView) -> String {
    crate::identity::hex_encode(&lanmsg_proto::sign::to_cbor(p))
}

/// 预览读库。旧版本存的是纯文本：空串视为无预览，其余当作文字预览。
fn preview_from_db(s: &str) -> RequestPreviewView {
    crate::identity::hex_decode(s)
        .ok()
        .and_then(|b| lanmsg_proto::sign::from_cbor::<RequestPreviewView>(&b).ok())
        .unwrap_or_else(|| {
            if s.is_empty() {
                RequestPreviewView::None
            } else {
                RequestPreviewView::Text {
                    text: s.to_string(),
                }
            }
        })
}

/// 查询请求行的原始元组。
type RequestTuple = (String, Vec<u8>, Option<Vec<u8>>, String, i64);

impl Store {
    /// 查询联系人，不存在返回 Ok(None)。
    pub fn get_contact(
        &self,
        user: &UserId,
    ) -> Result<Option<ContactRow>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<(String, i64, Option<i64>, i64, i64)> = conn
            .query_row(
                "SELECT state, auto_accept, ask_above_bytes, updated_millis, updated_counter
                 FROM contacts WHERE user_id = ?1",
                [&user.0[..]],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((state, auto_accept, ask_above_bytes, millis, counter)) = row else {
            return Ok(None);
        };
        Ok(Some(ContactRow {
            user: *user,
            state: ContactState::parse(&state)?,
            auto_accept: auto_accept != 0,
            ask_above_bytes: ask_above_bytes.map(|n| n as u64),
            updated: Hlc {
                millis,
                counter: counter as u32,
            },
        }))
    }

    /// `updated` 严格更新才写入，返回是否写入。
    pub fn upsert_contact_if_newer(
        &self,
        row: &ContactRow,
    ) -> Result<bool, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let n = conn.execute(
            "INSERT INTO contacts(user_id, state, auto_accept, ask_above_bytes, updated_millis, updated_counter)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(user_id) DO UPDATE SET
               state = excluded.state, auto_accept = excluded.auto_accept,
               ask_above_bytes = excluded.ask_above_bytes,
               updated_millis = excluded.updated_millis, updated_counter = excluded.updated_counter
             WHERE excluded.updated_millis > contacts.updated_millis
                OR (excluded.updated_millis = contacts.updated_millis
                    AND excluded.updated_counter > contacts.updated_counter)",
            rusqlite::params![
                &row.user.0[..],
                row.state.as_str(),
                row.auto_accept as i64,
                row.ask_above_bytes.map(|n| n as i64),
                row.updated.millis,
                row.updated.counter as i64,
            ],
        )?;
        Ok(n > 0)
    }

    /// 列出联系人；`state` 为 None 时返回全部。
    pub fn list_contacts(
        &self,
        state: Option<ContactState>,
    ) -> Result<Vec<ContactRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let sql =
            "SELECT user_id, state, auto_accept, ask_above_bytes, updated_millis, updated_counter
                   FROM contacts WHERE (?1 IS NULL OR state = ?1)";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![state.map(|s| s.as_str())], |r| {
            let user: Vec<u8> = r.get(0)?;
            let state: String = r.get(1)?;
            let auto_accept: i64 = r.get(2)?;
            let ask: Option<i64> = r.get(3)?;
            let millis: i64 = r.get(4)?;
            let counter: i64 = r.get(5)?;
            Ok((user, state, auto_accept, ask, millis, counter))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (user, state, auto_accept, ask, millis, counter) =
                r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
            out.push(ContactRow {
                user: UserId(user.try_into().map_err(|bytes: Vec<u8>| {
                    crate::store::StoreError::Codec(format!(
                        "user_id invalid length: {}",
                        bytes.len()
                    ))
                })?),
                state: ContactState::parse(&state)?,
                auto_accept: auto_accept != 0,
                ask_above_bytes: ask.map(|n| n as u64),
                updated: Hlc {
                    millis,
                    counter: counter as u32,
                },
            });
        }
        Ok(out)
    }

    /// 写入待处理请求：同一 `from_user`+`kind` 已有 open 请求则更新其 preview
    /// 并返回已有 id。
    pub fn upsert_open_request(
        &self,
        row: &RequestRow,
    ) -> Result<String, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "INSERT INTO requests(request_id, kind, from_user, conv_id, preview, created_at, status)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, 'open')
             ON CONFLICT(request_id) DO UPDATE SET preview = excluded.preview",
            rusqlite::params![
                row.request_id,
                row.kind.as_str(),
                &row.from_user.0[..],
                row.conv.as_ref().map(|c| &c.0[..]),
                preview_to_db(&row.preview),
                row.created_at,
            ],
        )?;
        // 同一 from_user+kind 折叠：若有其他 open 请求，取最早的并归并到本条。
        conn.execute(
            "UPDATE requests SET status = 'closed'
             WHERE status = 'open' AND kind = ?1 AND from_user = ?2 AND request_id != ?3",
            rusqlite::params![row.kind.as_str(), &row.from_user.0[..], row.request_id],
        )?;
        Ok(row.request_id.clone())
    }

    /// 查询请求，不存在返回 Ok(None)。
    pub fn get_request(&self, id: &str) -> Result<Option<RequestRow>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<RequestTuple> = conn
            .query_row(
                "SELECT kind, from_user, conv_id, preview, created_at
                 FROM requests WHERE request_id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, Option<Vec<u8>>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, from_user, conv, preview, created_at)) = row else {
            return Ok(None);
        };
        Ok(Some(RequestRow {
            request_id: id.to_string(),
            kind: RequestKindRow::parse(&kind)?,
            from_user: UserId(from_user.try_into().map_err(|bytes: Vec<u8>| {
                crate::store::StoreError::Codec(format!(
                    "from_user invalid length: {}",
                    bytes.len()
                ))
            })?),
            conv: conv
                .map(|c| {
                    c.try_into()
                        .map(lanmsg_proto::ids::ConvId)
                        .map_err(|bytes: Vec<u8>| {
                            crate::store::StoreError::Codec(format!(
                                "conv_id invalid length: {}",
                                bytes.len()
                            ))
                        })
                })
                .transpose()?,
            preview: preview_from_db(&preview),
            created_at,
        }))
    }

    /// 关闭请求。
    pub fn close_request(&self, id: &str) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "UPDATE requests SET status = 'closed' WHERE request_id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// 列出全部 open 请求（created_at 升序）。
    pub fn list_open_requests(&self) -> Result<Vec<RequestRow>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT request_id, kind, from_user, conv_id, preview, created_at
             FROM requests WHERE status = 'open' ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, Option<Vec<u8>>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (request_id, kind, from_user, conv, preview, created_at) =
                r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
            out.push(RequestRow {
                request_id,
                kind: RequestKindRow::parse(&kind)?,
                from_user: UserId(from_user.try_into().map_err(|bytes: Vec<u8>| {
                    crate::store::StoreError::Codec(format!(
                        "from_user invalid length: {}",
                        bytes.len()
                    ))
                })?),
                conv: conv
                    .map(|c| {
                        c.try_into()
                            .map(lanmsg_proto::ids::ConvId)
                            .map_err(|bytes: Vec<u8>| {
                                crate::store::StoreError::Codec(format!(
                                    "conv_id invalid length: {}",
                                    bytes.len()
                                ))
                            })
                    })
                    .transpose()?,
                preview: preview_from_db(&preview),
                created_at,
            });
        }
        Ok(out)
    }

    /// 插入吊销记录（CBOR 整条存 record），重复返回 false。
    pub fn insert_revocation(&self, r: &Revocation) -> Result<bool, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let n = conn.execute(
            "INSERT OR IGNORE INTO revocations(device_id, user_id, record) VALUES(?1, ?2, ?3)",
            rusqlite::params![&r.device.0[..], &r.user.0[..], lanmsg_proto::to_cbor(r)],
        )?;
        Ok(n > 0)
    }

    /// 列出全部吊销记录。
    pub fn list_revocations(&self) -> Result<Vec<Revocation>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT record FROM revocations")?;
        let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = Vec::new();
        for r in rows {
            let bytes = r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
            let rec: Revocation = lanmsg_proto::from_cbor(&bytes)
                .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
            out.push(rec);
        }
        Ok(out)
    }

    /// 设备是否已被吊销。
    pub fn is_revoked(&self, d: &DeviceId) -> Result<bool, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let n: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM revocations WHERE device_id = ?1",
                [&d.0[..]],
                |r| r.get(0),
            )
            .optional()?;
        Ok(n.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn user(n: u8) -> UserId {
        UserId([n; 32])
    }

    #[test]
    fn contact_upsert_only_if_newer() {
        let s = store();
        let mut row = ContactRow {
            user: user(1),
            state: ContactState::Outgoing,
            auto_accept: true,
            ask_above_bytes: None,
            updated: Hlc {
                millis: 5,
                counter: 0,
            },
        };
        assert!(s.upsert_contact_if_newer(&row).unwrap());
        // 更旧：拒绝。
        row.updated = Hlc {
            millis: 4,
            counter: 0,
        };
        row.state = ContactState::Rejected;
        assert!(!s.upsert_contact_if_newer(&row).unwrap());
        assert_eq!(
            s.get_contact(&user(1)).unwrap().unwrap().state,
            ContactState::Outgoing
        );
        // 同毫秒更高 counter：接受。
        row.updated = Hlc {
            millis: 5,
            counter: 1,
        };
        assert!(s.upsert_contact_if_newer(&row).unwrap());
        assert_eq!(
            s.get_contact(&user(1)).unwrap().unwrap().state,
            ContactState::Rejected
        );
    }

    #[test]
    fn open_request_is_folded_per_user_and_kind() {
        let s = store();
        let r1 = RequestRow {
            request_id: "r1".into(),
            kind: RequestKindRow::Message,
            from_user: user(2),
            conv: None,
            preview: RequestPreviewView::Text {
                text: "第一条".into(),
            },
            created_at: 1,
        };
        let id1 = s.upsert_open_request(&r1).unwrap();
        let r2 = RequestRow {
            request_id: "r2".into(),
            kind: RequestKindRow::Message,
            from_user: user(2),
            conv: None,
            preview: RequestPreviewView::Text {
                text: "第二条".into(),
            },
            created_at: 2,
        };
        let id2 = s.upsert_open_request(&r2).unwrap();
        assert_ne!(id1, id2);
        // 折叠后只剩最后一条 open，preview 为第二条。
        let open = s.list_open_requests().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].request_id, "r2");
        assert_eq!(
            open[0].preview,
            RequestPreviewView::Text {
                text: "第二条".into()
            }
        );
    }
}

#[cfg(test)]
mod preview_codec_tests {
    use super::*;

    #[test]
    fn preview_round_trips_through_db_text() {
        for p in [
            RequestPreviewView::None,
            RequestPreviewView::Text {
                text: "你好".into(),
            },
            RequestPreviewView::Files {
                first_name: "a.bin".into(),
                count: 3,
                total_size: 99,
            },
            RequestPreviewView::Folder {
                name: "docs".into(),
                count: 2,
            },
        ] {
            assert_eq!(preview_from_db(&preview_to_db(&p)), p);
        }
    }

    #[test]
    fn legacy_plain_text_rows_are_tolerated() {
        assert_eq!(preview_from_db(""), RequestPreviewView::None);
        assert_eq!(
            preview_from_db("第一条"),
            RequestPreviewView::Text {
                text: "第一条".into()
            }
        );
    }
}
