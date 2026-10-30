// P1 核心骨架：消息存储（Task 6）。
// MsgStateRow 存为小写字符串；envelope 存完整 CBOR；text 冗余存放 MsgBody::Text。
// 注释中文，标识符英文。

use lanmsg_proto::ids::{ConvId, DeviceId, MsgId, UserId};
use lanmsg_proto::types::{Envelope, MsgBody, VersionVector};

use crate::store::Store;

/// 查询消息行的原始元组（字段顺序与 SELECT 列一致）。
type MessageTuple = (
    Vec<u8>,
    i64,
    Vec<u8>,
    i64,
    i64,
    i64,
    Vec<u8>,
    String,
    i64,
    Option<String>,
);

/// 查作者所属用户（devices 表），查不到返回 None。
fn author_user_bytes(
    tx: &rusqlite::Transaction<'_>,
    author: &[u8],
) -> Result<Option<Vec<u8>>, rusqlite::Error> {
    use rusqlite::OptionalExtension;
    Ok(tx
        .query_row(
            "SELECT user_id FROM devices WHERE device_id = ?1",
            [author],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()?
        .flatten())
}

/// 把 Blob 元数据列表写入 message_blobs（idx 从 0）。
fn write_message_blobs<'a>(
    tx: &rusqlite::Transaction<'_>,
    env: &Envelope,
    items: impl Iterator<Item = (&'a lanmsg_proto::types::BlobMeta, Option<&'a str>)>,
) -> Result<(), rusqlite::Error> {
    for (idx, (meta, rel_path)) in items.enumerate() {
        tx.execute(
            "INSERT OR IGNORE INTO message_blobs(author, seq, idx, conv_id, hash, name, size, mime, rel_path, hlc_millis, hlc_counter, meta)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                &env.id.author.0[..],
                env.id.seq as i64,
                idx as i64,
                &env.conv.0,
                &meta.hash.0[..],
                meta.name,
                meta.size as i64,
                meta.mime,
                rel_path,
                env.hlc.millis,
                env.hlc.counter as i64,
                lanmsg_proto::to_cbor(meta),
            ],
        )?;
    }
    Ok(())
}

/// 消息状态行，DB 中存为小写字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MsgStateRow {
    Sending,
    Sent,
    Delivered,
    Read,
    Failed,
    Recalled,
}

impl MsgStateRow {
    pub fn as_str(&self) -> &'static str {
        match self {
            MsgStateRow::Sending => "sending",
            MsgStateRow::Sent => "sent",
            MsgStateRow::Delivered => "delivered",
            MsgStateRow::Read => "read",
            MsgStateRow::Failed => "failed",
            MsgStateRow::Recalled => "recalled",
        }
    }

    pub fn parse(s: &str) -> Result<Self, crate::store::StoreError> {
        match s {
            "sending" => Ok(MsgStateRow::Sending),
            "sent" => Ok(MsgStateRow::Sent),
            "delivered" => Ok(MsgStateRow::Delivered),
            "read" => Ok(MsgStateRow::Read),
            "failed" => Ok(MsgStateRow::Failed),
            "recalled" => Ok(MsgStateRow::Recalled),
            _ => Err(crate::store::StoreError::Codec(format!(
                "未知消息状态: {s}"
            ))),
        }
    }
}

/// 存储的消息。
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub env: Envelope,
    pub state: MsgStateRow,
    pub received_at: i64,
}

impl Store {
    /// 返回 (seq, conv_seq)：作者全局最大 seq+1、会话内最大 conv_seq+1（无记录为 1）。
    /// 同时考虑 preparing 表中占位的 seq/conv_seq。
    pub fn next_seqs(
        &self,
        author: &DeviceId,
        conv: &ConvId,
    ) -> Result<(u64, u64), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let seq: Option<i64> = conn.query_row(
            "SELECT MAX(m) FROM (
               SELECT MAX(seq) AS m FROM messages WHERE author = ?1
               UNION ALL SELECT MAX(seq) AS m FROM preparing WHERE author = ?1
             )",
            rusqlite::params![&author.0[..]],
            |r| r.get::<_, Option<i64>>(0),
        )?;
        let conv_seq: Option<i64> = conn.query_row(
            "SELECT MAX(m) FROM (
               SELECT MAX(conv_seq) AS m FROM messages WHERE conv_id = ?1 AND author = ?2
               UNION ALL SELECT MAX(conv_seq) AS m FROM preparing WHERE conv_id = ?1 AND author = ?2
             )",
            rusqlite::params![&conv.0[..], &author.0[..]],
            |r| r.get::<_, Option<i64>>(0),
        )?;
        Ok((
            (seq.unwrap_or(0) + 1) as u64,
            (conv_seq.unwrap_or(0) + 1) as u64,
        ))
    }

    /// INSERT OR IGNORE 插入信封，返回是否新插入；新插入时在同一事务内推进
    /// version_vectors，并处理 author_user、hidden、message_blobs、recalls 关联。
    pub fn insert_envelope(
        &self,
        env: &Envelope,
        state: MsgStateRow,
        received_at: i64,
    ) -> Result<bool, crate::store::StoreError> {
        self.insert_envelope_inner(env, state, received_at, None)
    }

    /// 同 `insert_envelope`，但强制 hidden 值（陌生人待处理消息用 `hidden = true`）。
    pub fn insert_envelope_hidden(
        &self,
        env: &Envelope,
        state: MsgStateRow,
        received_at: i64,
        hidden: bool,
    ) -> Result<bool, crate::store::StoreError> {
        self.insert_envelope_inner(env, state, received_at, Some(hidden))
    }

    fn insert_envelope_inner(
        &self,
        env: &Envelope,
        state: MsgStateRow,
        received_at: i64,
        hidden_override: Option<bool>,
    ) -> Result<bool, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let mut conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let tx = conn.transaction()?;

        let author_bytes: &[u8] = &env.id.author.0;
        let conv_bytes: &[u8] = &env.conv.0;
        let contiguous: i64 = tx.query_row(
            "SELECT COALESCE(MAX(contiguous), 0) FROM version_vectors WHERE conv_id = ?1 AND author = ?2",
            rusqlite::params![conv_bytes, author_bytes],
            |r| r.get(0),
        ).optional()?.unwrap_or(0);

        let text = match &env.body {
            MsgBody::Text { text } => Some(text.clone()),
            _ => None,
        };
        let envelope_bytes = lanmsg_proto::to_cbor(env);
        let hidden = hidden_override.unwrap_or_else(|| lanmsg_proto::body_is_hidden(&env.body));
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO messages(author, seq, conv_id, conv_seq, hlc_millis, hlc_counter, envelope, state, received_at, text, author_user, hidden, deleted)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0)",
            rusqlite::params![
                author_bytes,
                env.id.seq as i64,
                conv_bytes,
                env.conv_seq as i64,
                env.hlc.millis,
                env.hlc.counter as i64,
                envelope_bytes,
                state.as_str(),
                received_at,
                text,
                author_user_bytes(&tx, author_bytes)?.as_deref(),
                hidden as i64,
            ],
        )? > 0;

        if inserted {
            // Files/Folder：写 message_blobs。
            match &env.body {
                MsgBody::Files { items } => {
                    write_message_blobs(&tx, env, items.iter().map(|b| (b, None)))?;
                }
                MsgBody::Folder { name: _, entries } => {
                    write_message_blobs(
                        &tx,
                        env,
                        entries.iter().map(|e| (&e.blob, Some(e.rel_path.as_str()))),
                    )?;
                }
                _ => {}
            }
            // Recall：写 recalls 并召回已存在的目标。
            if let MsgBody::Recall { target } = &env.body {
                tx.execute(
                    "INSERT OR IGNORE INTO recalls(target_author, target_seq, recall_author, recall_seq)
                     VALUES(?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        &target.author.0[..],
                        target.seq as i64,
                        author_bytes,
                        env.id.seq as i64,
                    ],
                )?;
                tx.execute(
                    "UPDATE messages SET state = 'recalled'
                     WHERE author = ?1 AND seq = ?2 AND state != 'recalled'",
                    rusqlite::params![&target.author.0[..], target.seq as i64],
                )?;
            }
            // 目标此前被召回：新插入的消息直接置 recalled。
            let recalled: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM recalls WHERE target_author = ?1 AND target_seq = ?2",
                    rusqlite::params![author_bytes, env.id.seq as i64],
                    |r| r.get(0),
                )
                .optional()?;
            if recalled.is_some() {
                tx.execute(
                    "UPDATE messages SET state = 'recalled' WHERE author = ?1 AND seq = ?2",
                    rusqlite::params![author_bytes, env.id.seq as i64],
                )?;
            }
        }

        if inserted && (env.conv_seq as i64) == contiguous + 1 {
            let mut cont = env.conv_seq;
            loop {
                let exists: Option<i64> = tx.query_row(
                    "SELECT 1 FROM messages WHERE conv_id = ?1 AND author = ?2 AND conv_seq = ?3",
                    rusqlite::params![conv_bytes, author_bytes, (cont + 1) as i64],
                    |r| r.get(0),
                ).optional()?;
                match exists {
                    Some(_) => cont += 1,
                    None => break,
                }
            }
            tx.execute(
                "INSERT INTO version_vectors(conv_id, author, contiguous) VALUES(?1, ?2, ?3)
                 ON CONFLICT(conv_id, author) DO UPDATE SET contiguous = excluded.contiguous",
                rusqlite::params![conv_bytes, author_bytes, cont as i64],
            )?;
        }

        tx.commit()?;
        Ok(inserted)
    }

    /// 批量设置隐藏标记。
    pub fn set_hidden(&self, ids: &[MsgId], hidden: bool) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        for id in ids {
            conn.execute(
                "UPDATE messages SET hidden = ?1 WHERE author = ?2 AND seq = ?3",
                rusqlite::params![hidden as i64, &id.author.0[..], id.seq as i64],
            )?;
        }
        Ok(())
    }

    /// 批量标记本地删除（同步保留，UI 不再显示）。
    pub fn mark_deleted(&self, ids: &[MsgId]) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        for id in ids {
            conn.execute(
                "UPDATE messages SET deleted = 1 WHERE author = ?1 AND seq = ?2",
                rusqlite::params![&id.author.0[..], id.seq as i64],
            )?;
        }
        Ok(())
    }

    /// 把该会话中 `author_user` 发出的、`hlc <= upto`、状态为 sent/delivered 的消息
    /// 置 read，返回被改的 id。
    pub fn mark_read_upto(
        &self,
        conv: &ConvId,
        author_user: &UserId,
        upto: lanmsg_proto::types::Hlc,
    ) -> Result<Vec<MsgId>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT author, seq FROM messages
             WHERE conv_id = ?1 AND author_user = ?2 AND state IN ('sent', 'delivered')
               AND (hlc_millis < ?3 OR (hlc_millis = ?3 AND hlc_counter <= ?4))",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![
                &conv.0[..],
                &author_user.0[..],
                upto.millis,
                upto.counter as i64
            ],
            |row| {
                let author: Vec<u8> = row.get(0)?;
                let seq: i64 = row.get(1)?;
                Ok((author, seq))
            },
        )?;
        let mut ids = Vec::new();
        for r in rows {
            let (author, seq) = r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
            ids.push(MsgId {
                author: DeviceId(author.try_into().map_err(|bytes: Vec<u8>| {
                    crate::store::StoreError::Codec(format!("author 长度非法: {}", bytes.len()))
                })?),
                seq: seq as u64,
            });
        }
        drop(stmt);
        self.set_message_state_multi(&ids, MsgStateRow::Read)?;
        Ok(ids)
    }

    /// 未读数：会话内他人发出、hlc 晚于 after、未隐藏未删除未召回的消息条数。
    pub fn unread_count(
        &self,
        conv: &ConvId,
        self_user: &UserId,
        after: lanmsg_proto::types::Hlc,
    ) -> Result<u32, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE conv_id = ?1 AND author_user IS NOT NULL AND author_user != ?2
               AND hidden = 0 AND deleted = 0 AND state != 'recalled'
               AND (hlc_millis > ?3 OR (hlc_millis = ?3 AND hlc_counter > ?4))",
            rusqlite::params![
                &conv.0[..],
                &self_user.0[..],
                after.millis,
                after.counter as i64
            ],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    /// 会话内某用户发出的全部消息 ID（撤回传播用）。
    pub fn messages_of_user_in_conv(
        &self,
        conv: &ConvId,
        user: &UserId,
    ) -> Result<Vec<MsgId>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT author, seq FROM messages WHERE conv_id = ?1 AND author_user = ?2")?;
        let rows = stmt.query_map(rusqlite::params![&conv.0[..], &user.0[..]], |row| {
            let author: Vec<u8> = row.get(0)?;
            let seq: i64 = row.get(1)?;
            Ok((author, seq))
        })?;
        let mut ids = Vec::new();
        for r in rows {
            let (author, seq) = r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
            ids.push(MsgId {
                author: DeviceId(author.try_into().map_err(|bytes: Vec<u8>| {
                    crate::store::StoreError::Codec(format!("author 长度非法: {}", bytes.len()))
                })?),
                seq: seq as u64,
            });
        }
        Ok(ids)
    }

    fn set_message_state_multi(
        &self,
        ids: &[MsgId],
        state: MsgStateRow,
    ) -> Result<(), crate::store::StoreError> {
        for id in ids {
            self.set_message_state(id, state)?;
        }
        Ok(())
    }

    /// 设置消息状态。
    pub fn set_message_state(
        &self,
        id: &MsgId,
        state: MsgStateRow,
    ) -> Result<(), crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        conn.execute(
            "UPDATE messages SET state = ?1 WHERE author = ?2 AND seq = ?3",
            rusqlite::params![state.as_str(), &id.author.0[..], id.seq as i64],
        )?;
        Ok(())
    }

    /// 查询单条消息。
    pub fn get_message(
        &self,
        id: &MsgId,
    ) -> Result<Option<StoredMessage>, crate::store::StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let row: Option<MessageTuple> = conn
            .query_row(
                "SELECT author, seq, conv_id, conv_seq, hlc_millis, hlc_counter, envelope, state, received_at, text
                 FROM messages WHERE author = ?1 AND seq = ?2",
                rusqlite::params![&id.author.0[..], id.seq as i64],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            _author,
            _seq,
            _conv_id,
            _conv_seq,
            _hlc_millis,
            _hlc_counter,
            envelope,
            state,
            received_at,
            _text,
        )) = row
        else {
            return Ok(None);
        };
        let env: Envelope = lanmsg_proto::from_cbor(&envelope)
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let state = MsgStateRow::parse(&state)?;
        Ok(Some(StoredMessage {
            env,
            state,
            received_at,
        }))
    }

    /// 列出会话消息：按 (hlc_millis, hlc_counter, author, seq) 降序取 limit 条，返回升序。
    pub fn list_messages(
        &self,
        conv: &ConvId,
        before: Option<&MsgId>,
        limit: u32,
    ) -> Result<Vec<StoredMessage>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let conv_key: &[u8] = &conv.0;
        let mut out: Vec<StoredMessage> = Vec::new();

        match before {
            Some(before_id) => {
                use rusqlite::OptionalExtension;
                let brow: Option<(i64, i64, Vec<u8>, i64)> = conn
                    .query_row(
                        "SELECT hlc_millis, hlc_counter, author, seq FROM messages WHERE author = ?1 AND seq = ?2",
                        rusqlite::params![&before_id.author.0[..], before_id.seq as i64],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .optional()?;
                let Some((bmillis, bcounter, bauthor, bseq)) = brow else {
                    return Ok(out);
                };
                let mut stmt = conn.prepare(
                    "SELECT author, seq, conv_id, conv_seq, hlc_millis, hlc_counter, envelope, state, received_at, text
                     FROM messages WHERE conv_id = ?1 AND hidden = 0 AND deleted = 0
                       AND (hlc_millis < ?2 OR (hlc_millis = ?2 AND hlc_counter < ?3) OR (hlc_millis = ?2 AND hlc_counter = ?3 AND author < ?4) OR (hlc_millis = ?2 AND hlc_counter = ?3 AND author = ?4 AND seq < ?5))
                     ORDER BY hlc_millis DESC, hlc_counter DESC, author DESC, seq DESC
                     LIMIT ?6",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![conv_key, bmillis, bcounter, bauthor, bseq, limit as i64],
                    stored_message_from_row,
                )?;
                for r in rows {
                    out.push(r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT author, seq, conv_id, conv_seq, hlc_millis, hlc_counter, envelope, state, received_at, text
                     FROM messages WHERE conv_id = ?1 AND hidden = 0 AND deleted = 0
                     ORDER BY hlc_millis DESC, hlc_counter DESC, author DESC, seq DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![conv_key, limit as i64],
                    stored_message_from_row,
                )?;
                for r in rows {
                    out.push(r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?);
                }
            }
        }

        out.reverse();
        Ok(out)
    }

    /// 返回会话中该作者 conv_seq > after_conv_seq 的信封（升序）。
    pub fn messages_after(
        &self,
        conv: &ConvId,
        author: &DeviceId,
        after_conv_seq: u64,
        limit: u32,
    ) -> Result<Vec<Envelope>, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let conv_key: &[u8] = &conv.0;
        let author_key: &[u8] = &author.0;
        let mut stmt = conn.prepare(
            "SELECT envelope FROM messages WHERE conv_id = ?1 AND author = ?2 AND conv_seq > ?3
             ORDER BY conv_seq ASC LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![conv_key, author_key, after_conv_seq as i64, limit as i64],
            |row| {
                let bytes: Vec<u8> = row.get(0)?;
                lanmsg_proto::from_cbor::<Envelope>(&bytes).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(e),
                    )
                })
            },
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?);
        }
        Ok(out)
    }

    /// 返回会话的版本向量。
    pub fn version_vector(&self, conv: &ConvId) -> Result<VersionVector, crate::store::StoreError> {
        let conn = self
            .conn()
            .map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
        let conv_key: &[u8] = &conv.0;
        let mut stmt =
            conn.prepare("SELECT author, contiguous FROM version_vectors WHERE conv_id = ?1")?;
        let rows = stmt.query_map(rusqlite::params![conv_key], |row| {
            let author_bytes: Vec<u8> = row.get(0)?;
            let author = DeviceId(author_bytes.try_into().map_err(|bytes: Vec<u8>| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Blob,
                    format!("author 长度非法: {}", bytes.len()).into(),
                )
            })?);
            let contiguous: i64 = row.get(1)?;
            Ok((author, contiguous as u64))
        })?;
        let mut map = std::collections::BTreeMap::new();
        for r in rows {
            let (k, v) = r.map_err(|e| crate::store::StoreError::Codec(e.to_string()))?;
            map.insert(k, v);
        }
        Ok(VersionVector(map))
    }
}

fn stored_message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    let env_bytes: Vec<u8> = row.get(6)?;
    let env: Envelope = lanmsg_proto::from_cbor(&env_bytes).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Blob, Box::new(e))
    })?;
    let state: String = row.get(7)?;
    let state = MsgStateRow::parse(&state)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(StoredMessage {
        env,
        state,
        received_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    const CONV: ConvId = ConvId([1u8; 32]);
    const AUTHOR: DeviceId = DeviceId([2u8; 32]);

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn env(seq: u64, conv_seq: u64) -> Envelope {
        Envelope {
            conv: CONV,
            id: MsgId {
                author: AUTHOR,
                seq,
            },
            conv_seq,
            hlc: lanmsg_proto::types::Hlc {
                millis: seq as i64,
                counter: 0,
            },
            body: MsgBody::Text { text: "x".into() },
            enc: None,
            sig: lanmsg_proto::types::Signature64([0u8; 64]),
        }
    }

    #[test]
    fn migrations_set_user_version() {
        let s = store();
        assert!(s.schema_version().unwrap() >= 2);
    }

    #[test]
    fn insert_envelope_is_idempotent() {
        let s = store();
        let e = env(1, 1);
        assert!(s.insert_envelope(&e, MsgStateRow::Sent, 0).unwrap());
        assert!(!s.insert_envelope(&e, MsgStateRow::Sent, 0).unwrap());
    }

    #[test]
    fn version_vector_tracks_contiguous_prefix_only() {
        let s = store();
        s.insert_envelope(&env(1, 1), MsgStateRow::Sent, 0).unwrap();
        s.insert_envelope(&env(3, 3), MsgStateRow::Sent, 0).unwrap();
        assert_eq!(s.version_vector(&CONV).unwrap().0[&AUTHOR], 1);
        s.insert_envelope(&env(2, 2), MsgStateRow::Sent, 0).unwrap();
        assert_eq!(s.version_vector(&CONV).unwrap().0[&AUTHOR], 3);
    }

    #[test]
    fn list_messages_pages_backwards_in_ascending_order() {
        let s = store();
        for i in 1..=5 {
            let e = env(i, i);
            s.insert_envelope(&e, MsgStateRow::Sent, i.try_into().unwrap())
                .unwrap();
        }
        let first = s.list_messages(&CONV, None, 2).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].env.conv_seq, 4);
        assert_eq!(first[1].env.conv_seq, 5);
        // 游标取页面最旧一条（升序首元素），下一页取严格更早且不重叠。
        let before = first[0].env.id.clone();
        let second = s.list_messages(&CONV, Some(&before), 2).unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].env.conv_seq, 2);
        assert_eq!(second[1].env.conv_seq, 3);
    }

    #[test]
    fn next_seqs_starts_at_one_and_increments() {
        let s = store();
        assert_eq!(s.next_seqs(&AUTHOR, &CONV).unwrap(), (1, 1));
        let e = env(1, 1);
        s.insert_envelope(&e, MsgStateRow::Sent, 1).unwrap();
        assert_eq!(s.next_seqs(&AUTHOR, &CONV).unwrap(), (2, 2));
        let other = ConvId([3u8; 32]);
        assert_eq!(s.next_seqs(&AUTHOR, &other).unwrap(), (2, 1));
    }

    #[test]
    fn reopen_file_db_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        {
            let s = Store::open(&path).unwrap();
            let e = env(1, 1);
            s.insert_envelope(&e, MsgStateRow::Sent, 1).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert!(s.get_message(&env(1, 1).id).unwrap().is_some());
    }

    #[test]
    fn messages_after_returns_ascending() {
        let s = store();
        for i in 1..=3 {
            let e = env(i, i);
            s.insert_envelope(&e, MsgStateRow::Sent, i.try_into().unwrap())
                .unwrap();
        }
        let after = s.messages_after(&CONV, &AUTHOR, 1, 10).unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].conv_seq, 2);
        assert_eq!(after[1].conv_seq, 3);
    }

    #[test]
    fn hidden_messages_excluded_from_list_but_included_in_sync() {
        let s = store();
        let mut text = env(1, 1);
        text.body = MsgBody::Text {
            text: "可见".into(),
        };
        let mut read = env(2, 2);
        read.body = MsgBody::Read {
            upto: lanmsg_proto::types::Hlc {
                millis: 1,
                counter: 0,
            },
        };
        s.insert_envelope(&text, MsgStateRow::Delivered, 1).unwrap();
        s.insert_envelope(&read, MsgStateRow::Delivered, 2).unwrap();
        assert_eq!(s.list_messages(&CONV, None, 10).unwrap().len(), 1);
        assert_eq!(s.messages_after(&CONV, &AUTHOR, 0, 10).unwrap().len(), 2);
    }

    #[test]
    fn recall_before_target_marks_target_recalled_on_arrival() {
        let s = store();
        let target = env(2, 2);
        let mut recall = env(1, 1);
        recall.body = MsgBody::Recall {
            target: target.id.clone(),
        };
        // 先撤回后到目标。
        s.insert_envelope(&recall, MsgStateRow::Delivered, 1)
            .unwrap();
        s.insert_envelope(&target, MsgStateRow::Delivered, 2)
            .unwrap();
        assert_eq!(
            s.get_message(&target.id).unwrap().unwrap().state,
            MsgStateRow::Recalled
        );
    }

    #[test]
    fn next_seqs_accounts_for_preparing() {
        let s = store();
        s.insert_preparing(&crate::store::blobs::PreparingRow {
            author: AUTHOR,
            seq: 1,
            conv: CONV,
            conv_seq: 1,
            hlc: lanmsg_proto::types::Hlc {
                millis: 1,
                counter: 0,
            },
            kind: crate::store::blobs::PreparingKind::Files,
            paths: vec![std::path::PathBuf::from("/tmp/a")],
            created_at: 1,
        })
        .unwrap();
        assert_eq!(s.next_seqs(&AUTHOR, &CONV).unwrap(), (2, 2));
    }

    #[test]
    fn unread_count_counts_only_others_after_last_read() {
        let s = store();
        let other = DeviceId([7u8; 32]);
        // 先插设备行，使后续消息的 author_user 关联生效。
        s.upsert_device(&crate::store::DeviceRow {
            device: other,
            user: UserId([8u8; 32]),
            cert: lanmsg_proto::types::DeviceCert::issue(
                &lanmsg_proto::sign::UserKey::generate(),
                other,
                "对方设备".into(),
                lanmsg_proto::types::DeviceType::Desktop,
                1,
            ),
            name: "对方设备".into(),
            device_type: lanmsg_proto::types::DeviceType::Desktop,
            last_addr: None,
            last_path: None,
            last_seen: None,
            revoked: false,
        })
        .unwrap();
        let mut mine = env(1, 1);
        mine.body = MsgBody::Text {
            text: "我的".into(),
        };
        s.insert_envelope(&mine, MsgStateRow::Delivered, 1).unwrap();
        let mut theirs = env(2, 1);
        theirs.id.author = other;
        theirs.body = MsgBody::Text {
            text: "对方的".into(),
        };
        s.insert_envelope(&theirs, MsgStateRow::Delivered, 2)
            .unwrap();
        let self_user = UserId([9u8; 32]);
        let after = lanmsg_proto::types::Hlc {
            millis: 0,
            counter: 0,
        };
        let n = s.unread_count(&CONV, &self_user, after).unwrap();
        assert!(n >= 1, "他人消息应计数");
    }
}
