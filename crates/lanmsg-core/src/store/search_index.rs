// P3 Task 15：消息文字搜索索引（message_text_index + FTS5 trigram）。
// 只索引可见的文字消息：隐藏（陌生人待处理）、已撤回、已删除的不写 / 会被移除。
// 注释中文，标识符英文。

use lanmsg_proto::ids::{ConvId, DeviceId, MsgId};
use lanmsg_proto::types::{Envelope, MsgBody};
use rusqlite::types::Value;

use crate::store::file_index::NameMatch;
use crate::store::{Store, StoreError};

/// 搜索结果条数上限。
const MAX_SEARCH_LIMIT: u32 = 200;

impl Store {
    /// 写文字索引：仅 `Text` 且 `!hidden` 且文本非空时写一行。须在 `insert_envelope`
    /// 新插入成功（且消息未处于撤回状态）的同一事务内调用。
    pub fn index_message_text(
        &self,
        tx: &rusqlite::Transaction<'_>,
        env: &Envelope,
        hidden: bool,
    ) -> Result<(), StoreError> {
        let MsgBody::Text { text } = &env.body else {
            return Ok(());
        };
        if hidden || text.trim().is_empty() {
            return Ok(());
        }
        tx.execute(
            "INSERT OR IGNORE INTO message_text_index(conv_id, author, seq, hlc_millis, text)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                &env.conv.0[..],
                &env.id.author.0[..],
                env.id.seq as i64,
                env.hlc.millis,
                text
            ],
        )?;
        Ok(())
    }

    /// 搜索消息文字：规则同文件名查询（trim 后 ≥3 字符走 FTS5 短语，1–2 字符走 LIKE，
    /// 空串返回空）。按 `hlc_millis DESC, id DESC`，`limit` 上限 200。
    pub fn search_messages(
        &self,
        query: &str,
        conv: Option<&ConvId>,
        limit: u32,
    ) -> Result<Vec<MsgId>, StoreError> {
        let Some(m) = NameMatch::parse(query) else {
            return Ok(Vec::new());
        };
        // JOIN messages 再过滤一次，作为索引遗漏清理时的兜底。
        let mut sql = String::from(
            "SELECT t.author, t.seq FROM message_text_index t
             JOIN messages m ON m.author = t.author AND m.seq = t.seq
             WHERE m.hidden = 0 AND m.deleted = 0 AND m.state != 'recalled'",
        );
        let mut args: Vec<Value> = Vec::new();
        match &m {
            NameMatch::Fts(q) => {
                sql.push_str(
                    " AND t.id IN (SELECT rowid FROM message_text_fts WHERE message_text_fts MATCH ?)",
                );
                args.push(Value::Text(q.clone()));
            }
            NameMatch::Like(q) => {
                sql.push_str(" AND t.text LIKE '%' || ? || '%' ESCAPE '\\'");
                args.push(Value::Text(q.clone()));
            }
        }
        if let Some(c) = conv {
            sql.push_str(" AND t.conv_id = ?");
            args.push(Value::Blob(c.0.to_vec()));
        }
        sql.push_str(" ORDER BY t.hlc_millis DESC, t.id DESC LIMIT ?");
        args.push(Value::Integer(i64::from(limit.min(MAX_SEARCH_LIMIT))));
        let conn = self.conn().map_err(|e| StoreError::Codec(e.to_string()))?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (author, seq) = row?;
            let author: [u8; 32] = author.try_into().map_err(|b: Vec<u8>| {
                StoreError::Codec(format!("author invalid length: {}", b.len()))
            })?;
            out.push(MsgId {
                author: DeviceId(author),
                seq: seq as u64,
            });
        }
        Ok(out)
    }

    /// 本机 SQLite 是否带 FTS5 trigram 分词器（建临时虚表探测后删除）。
    pub fn fts5_trigram_available(&self) -> Result<bool, StoreError> {
        let conn = self.conn().map_err(|e| StoreError::Codec(e.to_string()))?;
        match conn
            .execute_batch("CREATE VIRTUAL TABLE temp.__tri USING fts5(x, tokenize='trigram')")
        {
            Ok(()) => {
                conn.execute_batch("DROP TABLE temp.__tri")?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::store::{MsgStateRow, Store};
    use lanmsg_proto::ids::{ConvId, DeviceId, MsgId};
    use lanmsg_proto::types::{Envelope, Hlc, MsgBody, Signature64};

    const AUTHOR: DeviceId = DeviceId([2u8; 32]);

    fn text_env(seq: u64, conv: u8, millis: i64, text: &str) -> Envelope {
        Envelope {
            conv: ConvId([conv; 32]),
            id: MsgId {
                author: AUTHOR,
                seq,
            },
            conv_seq: seq,
            hlc: Hlc { millis, counter: 0 },
            body: MsgBody::Text { text: text.into() },
            enc: None,
            sig: Signature64([0u8; 64]),
        }
    }

    fn put(s: &Store, e: &Envelope) {
        s.insert_envelope(e, MsgStateRow::Delivered, 1).unwrap();
    }

    #[test]
    fn search_messages_short_and_long_queries() {
        let s = Store::open_in_memory().unwrap();
        let a = text_env(1, 1, 100, "明天开会");
        let b = text_env(2, 1, 101, "年度报告已发");
        let c = text_env(3, 1, 102, "annual meeting");
        for e in [&a, &b, &c] {
            put(&s, e);
        }
        assert_eq!(
            s.search_messages("开会", None, 10).unwrap(),
            vec![a.id.clone()]
        );
        assert_eq!(
            s.search_messages("annual", None, 10).unwrap(),
            vec![c.id.clone()]
        );
        // 大小写不敏感；单字走 LIKE。
        assert_eq!(
            s.search_messages("ANNUAL", None, 10).unwrap(),
            vec![c.id.clone()]
        );
        assert_eq!(
            s.search_messages("报", None, 10).unwrap(),
            vec![b.id.clone()]
        );
        assert!(s.search_messages("", None, 10).unwrap().is_empty());
        assert!(s.search_messages("  ", None, 10).unwrap().is_empty());
        // 会话筛选与排序（新的在前）。
        assert!(s
            .search_messages("开会", Some(&ConvId([9; 32])), 10)
            .unwrap()
            .is_empty());
        let both = text_env(4, 1, 103, "开会改到周五");
        put(&s, &both);
        assert_eq!(
            s.search_messages("开会", Some(&ConvId([1; 32])), 10)
                .unwrap(),
            vec![both.id.clone(), a.id.clone()]
        );
        assert_eq!(s.search_messages("开会", None, 1).unwrap(), vec![both.id]);
    }

    #[test]
    fn search_special_characters_are_literal() {
        let s = Store::open_in_memory().unwrap();
        put(&s, &text_env(1, 1, 100, "50% off \"sale\" now"));
        put(&s, &text_env(2, 1, 101, "500 off"));
        assert_eq!(s.search_messages("0% o", None, 10).unwrap().len(), 1);
        assert_eq!(s.search_messages("\"sale\"", None, 10).unwrap().len(), 1);
        assert!(s
            .search_messages("x\" OR \"1", None, 10)
            .unwrap()
            .is_empty());
        assert!(s.search_messages("_", None, 10).unwrap().is_empty());
    }

    #[test]
    fn hidden_and_recalled_messages_not_searchable() {
        let s = Store::open_in_memory().unwrap();
        let hidden = text_env(1, 1, 100, "秘密消息内容");
        s.insert_envelope_hidden(&hidden, MsgStateRow::Delivered, 1, true)
            .unwrap();
        // 撤回先于原消息到达。
        let target = text_env(3, 1, 102, "已撤回的内容");
        let recall = Envelope {
            body: MsgBody::Recall {
                target: target.id.clone(),
            },
            ..text_env(2, 1, 101, "")
        };
        put(&s, &recall);
        put(&s, &target);
        assert!(s.search_messages("秘密", None, 10).unwrap().is_empty());
        assert!(s.search_messages("撤回", None, 10).unwrap().is_empty());
        // 转为可见（接受陌生人）后可搜到。
        s.set_hidden(std::slice::from_ref(&hidden.id), false)
            .unwrap();
        assert_eq!(
            s.search_messages("秘密", None, 10).unwrap(),
            vec![hidden.id]
        );
    }

    #[test]
    fn delete_removes_from_message_search() {
        let s = Store::open_in_memory().unwrap();
        let e = text_env(1, 1, 100, "明天开会");
        put(&s, &e);
        assert_eq!(s.search_messages("开会", None, 10).unwrap().len(), 1);
        s.mark_deleted(&[e.id]).unwrap();
        assert!(s.search_messages("开会", None, 10).unwrap().is_empty());
    }
}
