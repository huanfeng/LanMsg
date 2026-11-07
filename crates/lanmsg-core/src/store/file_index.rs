// P3 Task 15：文件视图索引（file_index 表 + FTS5 trigram）。
// 文件页的类型 / 会话 / 来源设备 / 名字筛选都走这里；索引在 insert_envelope 的同一
// 事务内维护，旧库由 backfill_search_index 一次性回填。
// 注释中文，标识符英文。

use lanmsg_api::types::FileKindFilter;
use lanmsg_proto::ids::{BlobHash, ConvId, DeviceId, MsgId};
use lanmsg_proto::types::{BlobMeta, Envelope, MsgBody};
use rusqlite::types::Value;

use crate::store::{Store, StoreError};

/// 单次查询的行数硬上限（含"同毫秒补齐"的额外行），防止超大文件夹拖垮界面。
const MAX_TIE_ROWS: u32 = 5000;

/// 按 MIME 与扩展名把文件归类（存储为小写字符串，见 `kind_str`）。
pub fn file_kind(mime: &str, name: &str) -> FileKindFilter {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if mime.starts_with("image/") {
        FileKindFilter::Image
    } else if mime.starts_with("video/") {
        FileKindFilter::Video
    } else if matches!(
        ext.as_str(),
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst"
    ) || matches!(
        mime,
        "application/zip"
            | "application/x-7z-compressed"
            | "application/x-rar-compressed"
            | "application/gzip"
    ) {
        FileKindFilter::Archive
    } else if matches!(
        ext.as_str(),
        "pdf"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "odt"
            | "ods"
            | "odp"
            | "rtf"
            | "txt"
            | "md"
            | "csv"
            | "epub"
    ) {
        FileKindFilter::Document
    } else {
        FileKindFilter::Other
    }
}

/// 筛选类型的存储字符串；`All` 无对应存储值（表示不筛选）。
pub fn kind_str(kind: &FileKindFilter) -> Option<&'static str> {
    match kind {
        FileKindFilter::All => None,
        FileKindFilter::Image => Some("image"),
        FileKindFilter::Video => Some("video"),
        FileKindFilter::Document => Some("document"),
        FileKindFilter::Archive => Some("archive"),
        FileKindFilter::Other => Some("other"),
    }
}

/// 文件查询条件。
#[derive(Debug, Clone, Default)]
pub struct FileQuery {
    /// `image|video|archive|document|other`；None 不筛选。
    pub kind: Option<String>,
    pub conv: Option<ConvId>,
    /// 发出该文件消息的设备。
    pub author: Option<DeviceId>,
    /// 名字关键字（已含首尾空白时会被 trim）。
    pub name: Option<String>,
    /// 只返回 hlc_millis 严格小于该值的行（分页游标）。
    pub before_millis: Option<i64>,
    pub limit: u32,
}

/// 文件查询结果行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub conv: ConvId,
    pub msg: MsgId,
    pub item_idx: u32,
    pub hash: BlobHash,
    pub hlc_millis: i64,
}

/// 名字匹配条件：≥3 字符走 FTS5 trigram 短语，1–2 字符走 LIKE。
pub(crate) enum NameMatch {
    Fts(String),
    Like(String),
}

impl NameMatch {
    /// trim 后为空返回 None（调用方据此返回空结果或不加条件）。
    pub(crate) fn parse(q: &str) -> Option<NameMatch> {
        let q = q.trim();
        let n = q.chars().count();
        if n == 0 {
            None
        } else if n >= 3 {
            // 双引号短语，内部双引号双写转义；其余字符在引号内均为字面量。
            Some(NameMatch::Fts(format!("\"{}\"", q.replace('"', "\"\""))))
        } else {
            Some(NameMatch::Like(
                q.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_"),
            ))
        }
    }
}

fn arr32(v: Vec<u8>, what: &str) -> Result<[u8; 32], StoreError> {
    v.try_into()
        .map_err(|b: Vec<u8>| StoreError::Codec(format!("{what} invalid length: {}", b.len())))
}

/// `rel_path` 的文件名部分（兼容 `/` 与 `\`）。
fn file_name_of(rel_path: &str) -> &str {
    rel_path.rsplit(['/', '\\']).next().unwrap_or(rel_path)
}

fn insert_row(
    tx: &rusqlite::Transaction<'_>,
    env: &Envelope,
    idx: usize,
    meta: &BlobMeta,
    name: &str,
) -> Result<(), rusqlite::Error> {
    // INSERT OR IGNORE：UNIQUE(author, seq, item_idx) 保证重复入库不产生重复行。
    tx.execute(
        "INSERT OR IGNORE INTO file_index(conv_id, author, seq, item_idx, hash, name, mime, size, kind, hlc_millis)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            &env.conv.0[..],
            &env.id.author.0[..],
            env.id.seq as i64,
            idx as i64,
            &meta.hash.0[..],
            name,
            meta.mime,
            meta.size as i64,
            kind_str(&file_kind(&meta.mime, name)).unwrap_or("other"),
            env.hlc.millis,
        ],
    )?;
    Ok(())
}

/// 删除某消息在两个索引中的行（事务内版本）。
pub(crate) fn unindex_tx(
    tx: &rusqlite::Transaction<'_>,
    id: &MsgId,
) -> Result<(), rusqlite::Error> {
    let p = rusqlite::params![&id.author.0[..], id.seq as i64];
    tx.execute("DELETE FROM file_index WHERE author = ?1 AND seq = ?2", p)?;
    tx.execute(
        "DELETE FROM message_text_index WHERE author = ?1 AND seq = ?2",
        p,
    )?;
    Ok(())
}

impl Store {
    /// 写文件索引：`Files` 每项一行，`Folder` 每个条目一行（名字取 rel_path 的文件名），
    /// 其它消息类型不写。须在 `insert_envelope` 新插入成功的同一事务内调用。
    pub fn index_files(
        &self,
        tx: &rusqlite::Transaction<'_>,
        env: &Envelope,
    ) -> Result<(), StoreError> {
        match &env.body {
            MsgBody::Files { items } => {
                for (i, meta) in items.iter().enumerate() {
                    insert_row(tx, env, i, meta, &meta.name)?;
                }
            }
            MsgBody::Folder { entries, .. } => {
                for (i, e) in entries.iter().enumerate() {
                    insert_row(tx, env, i, &e.blob, file_name_of(&e.rel_path))?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 删除该消息在文件索引与文字索引中的行（本地删除 / 撤回时调用）。
    pub fn unindex_message(&self, id: &MsgId) -> Result<(), StoreError> {
        let mut conn = self.conn().map_err(|e| StoreError::Codec(e.to_string()))?;
        let tx = conn.transaction()?;
        unindex_tx(&tx, id)?;
        tx.commit()?;
        Ok(())
    }

    /// 旧库回填：`settings["search_index_v"] != "1"` 时按 1000 条一批遍历 messages
    /// （跳过已删除与已撤回），重建两个索引；完成后写标记，返回处理条数。
    pub fn backfill_search_index(&self) -> Result<u64, StoreError> {
        if self.get_setting("search_index_v")?.as_deref() == Some("1") {
            return Ok(0);
        }
        let mut done = 0u64;
        let mut skipped = 0u64;
        // 游标 (author, seq)：主键顺序，批间不持有连接锁。
        let mut cursor: (Vec<u8>, i64) = (Vec::new(), -1);
        loop {
            let mut conn = self.conn().map_err(|e| StoreError::Codec(e.to_string()))?;
            let tx = conn.transaction()?;
            let batch: Vec<(Vec<u8>, i64, Vec<u8>, bool)> = {
                let mut stmt = tx.prepare(
                    "SELECT author, seq, envelope, hidden FROM messages
                     WHERE deleted = 0 AND state != 'recalled' AND (author > ?1 OR (author = ?1 AND seq > ?2))
                     ORDER BY author, seq LIMIT 1000",
                )?;
                let rows = stmt.query_map(rusqlite::params![cursor.0, cursor.1], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0))
                })?;
                rows.collect::<Result<_, _>>()?
            };
            if batch.is_empty() {
                tx.commit()?;
                break;
            }
            for (author, seq, envelope, hidden) in &batch {
                cursor = (author.clone(), *seq);
                // 单条信封损坏只跳过并记录，不中止整个回填。
                let env: Envelope = match lanmsg_proto::from_cbor(envelope) {
                    Ok(env) => env,
                    Err(e) => {
                        skipped += 1;
                        tracing::warn!("回填索引：跳过无法解码的信封 (seq={seq}): {e}");
                        continue;
                    }
                };
                self.index_files(&tx, &env)?;
                self.index_message_text(&tx, &env, *hidden)?;
                done += 1;
            }
            tx.commit()?;
            // 释放连接锁并让出片刻，避免回填长时间饿死其他读写。
            drop(conn);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if skipped > 0 {
            tracing::warn!("回填索引：共跳过 {skipped} 条无法解码的消息");
        }
        self.set_setting("search_index_v", "1")?;
        Ok(done)
    }

    /// 文件查询：排除隐藏 / 已删除 / 已撤回的消息，按 `hlc_millis DESC, id DESC`。
    /// 分页游标只有毫秒：为避免同一毫秒的行（如一个大文件夹）被游标跳过，
    /// 末行同毫秒的剩余行一并返回（页可能略大于 `limit`，上限 `MAX_TIE_ROWS`）。
    pub fn query_file_rows(&self, f: &FileQuery) -> Result<Vec<FileRow>, StoreError> {
        let name = match f.name.as_deref().map(NameMatch::parse) {
            Some(None) => return Ok(Vec::new()),
            Some(Some(m)) => Some(m),
            None => None,
        };
        let mut sql = String::from(
            "SELECT f.id, f.conv_id, f.author, f.seq, f.item_idx, f.hash, f.hlc_millis
             FROM file_index f JOIN messages m ON m.author = f.author AND m.seq = f.seq
             WHERE m.hidden = 0 AND m.deleted = 0 AND m.state != 'recalled'",
        );
        let mut args: Vec<Value> = Vec::new();
        if let Some(k) = &f.kind {
            sql.push_str(" AND f.kind = ?");
            args.push(Value::Text(k.clone()));
        }
        if let Some(c) = &f.conv {
            sql.push_str(" AND f.conv_id = ?");
            args.push(Value::Blob(c.0.to_vec()));
        }
        if let Some(a) = &f.author {
            sql.push_str(" AND f.author = ?");
            args.push(Value::Blob(a.0.to_vec()));
        }
        if let Some(b) = f.before_millis {
            sql.push_str(" AND f.hlc_millis < ?");
            args.push(Value::Integer(b));
        }
        match &name {
            Some(NameMatch::Fts(q)) => {
                sql.push_str(
                    " AND f.id IN (SELECT rowid FROM file_index_fts WHERE file_index_fts MATCH ?)",
                );
                args.push(Value::Text(q.clone()));
            }
            Some(NameMatch::Like(q)) => {
                sql.push_str(" AND f.name LIKE '%' || ? || '%' ESCAPE '\\'");
                args.push(Value::Text(q.clone()));
            }
            None => {}
        }
        let conn = self.conn().map_err(|e| StoreError::Codec(e.to_string()))?;
        let run = |extra: &str,
                   extra_args: Vec<Value>,
                   limit: u32|
         -> Result<Vec<(i64, FileRow)>, StoreError> {
            let q = format!("{sql}{extra} ORDER BY f.hlc_millis DESC, f.id DESC LIMIT {limit}");
            let mut all = args.clone();
            all.extend(extra_args);
            let mut stmt = conn.prepare(&q)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(all), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Vec<u8>>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, conv, author, seq, idx, hash, millis) = row?;
                out.push((
                    id,
                    FileRow {
                        conv: ConvId(arr32(conv, "conv_id")?),
                        msg: MsgId {
                            author: DeviceId(arr32(author, "author")?),
                            seq: seq as u64,
                        },
                        item_idx: idx as u32,
                        hash: BlobHash(arr32(hash, "hash")?),
                        hlc_millis: millis,
                    },
                ));
            }
            Ok(out)
        };
        let limit = f.limit.max(1);
        let mut rows = run("", Vec::new(), limit)?;
        if rows.len() as u32 == limit {
            if let Some((last_id, last)) = rows.last().cloned() {
                let rest = run(
                    " AND f.hlc_millis = ? AND f.id < ?",
                    vec![Value::Integer(last.hlc_millis), Value::Integer(last_id)],
                    MAX_TIE_ROWS,
                )?;
                rows.extend(rest);
            }
        }
        Ok(rows.into_iter().map(|(_, r)| r).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MsgStateRow;
    use lanmsg_proto::types::{FolderEntry, Hlc, Signature64};

    const CONV: ConvId = ConvId([1u8; 32]);
    const AUTHOR: DeviceId = DeviceId([2u8; 32]);

    fn meta(name: &str, mime: &str, tag: u8) -> BlobMeta {
        BlobMeta {
            hash: BlobHash([tag; 32]),
            size: 10,
            name: name.into(),
            mime: mime.into(),
            media: None,
            thumb: None,
        }
    }

    fn env(seq: u64, millis: i64, body: MsgBody) -> Envelope {
        Envelope {
            conv: CONV,
            id: MsgId {
                author: AUTHOR,
                seq,
            },
            conv_seq: seq,
            hlc: Hlc { millis, counter: 0 },
            body,
            enc: None,
            sig: Signature64([0u8; 64]),
        }
    }

    fn files_env(seq: u64, millis: i64, names: &[(&str, &str)]) -> Envelope {
        env(
            seq,
            millis,
            MsgBody::Files {
                items: names
                    .iter()
                    .enumerate()
                    .map(|(i, (n, m))| meta(n, m, seq as u8 * 10 + i as u8))
                    .collect(),
            },
        )
    }

    fn put(s: &Store, e: &Envelope) {
        s.insert_envelope(e, MsgStateRow::Delivered, 1).unwrap();
    }

    fn names(s: &Store, q: &str) -> Vec<BlobHash> {
        s.query_file_rows(&FileQuery {
            name: Some(q.into()),
            limit: 50,
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .map(|r| r.hash)
        .collect()
    }

    fn index_count(s: &Store) -> i64 {
        s.conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM file_index", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn file_kind_table() {
        assert_eq!(file_kind("image/png", "a.png"), FileKindFilter::Image);
        assert_eq!(file_kind("video/mp4", "a.mp4"), FileKindFilter::Video);
        assert_eq!(
            file_kind("application/octet-stream", "a.7z"),
            FileKindFilter::Archive
        );
        assert_eq!(
            file_kind("application/zip", "noext"),
            FileKindFilter::Archive
        );
        assert_eq!(file_kind("text/plain", "a.MD"), FileKindFilter::Document);
        assert_eq!(
            file_kind("application/x-foo", "a.bin"),
            FileKindFilter::Other
        );
    }

    #[test]
    fn insert_files_message_indexes_each_item() {
        let s = Store::open_in_memory().unwrap();
        let e = files_env(
            1,
            100,
            &[
                ("a.png", "image/png"),
                ("b.pdf", "application/pdf"),
                ("c", "x/y"),
            ],
        );
        put(&s, &e);
        assert_eq!(index_count(&s), 3);
        // 重复入库同一 envelope 仍 3 行。
        put(&s, &e);
        assert_eq!(index_count(&s), 3);
    }

    #[test]
    fn folder_entries_use_file_name_part() {
        let s = Store::open_in_memory().unwrap();
        let e = env(
            1,
            100,
            MsgBody::Folder {
                name: "dir".into(),
                entries: vec![FolderEntry {
                    rel_path: "dir/sub/notes.txt".into(),
                    blob: meta("ignored", "text/plain", 7),
                }],
            },
        );
        put(&s, &e);
        assert_eq!(names(&s, "notes").len(), 1);
        assert!(names(&s, "ignored").is_empty());
    }

    #[test]
    fn query_by_short_cjk_name_uses_like() {
        let s = Store::open_in_memory().unwrap();
        put(
            &s,
            &files_env(
                1,
                100,
                &[
                    ("报告.pdf", "application/pdf"),
                    ("周报.docx", "application/msword"),
                    ("图.png", "image/png"),
                ],
            ),
        );
        assert_eq!(names(&s, "报").len(), 2);
    }

    #[test]
    fn query_by_long_name_uses_fts() {
        let s = Store::open_in_memory().unwrap();
        put(
            &s,
            &files_env(
                1,
                100,
                &[
                    ("2024-annual-report.pdf", "application/pdf"),
                    ("anal.txt", "text/plain"),
                ],
            ),
        );
        let hits = names(&s, "annual");
        assert_eq!(hits, vec![BlobHash([10; 32])]);
    }

    #[test]
    fn query_escapes_like_wildcards() {
        let s = Store::open_in_memory().unwrap();
        put(
            &s,
            &files_env(
                1,
                100,
                &[("100%.txt", "text/plain"), ("1000.txt", "text/plain")],
            ),
        );
        assert_eq!(names(&s, "0%"), vec![BlobHash([10; 32])]);
    }

    #[test]
    fn query_fts_special_characters_are_literal() {
        let s = Store::open_in_memory().unwrap();
        put(
            &s,
            &files_env(
                1,
                100,
                &[("a\"b OR c*.txt", "text/plain"), ("abc.txt", "text/plain")],
            ),
        );
        // 引号、OR、* 都按字面量匹配，不报语法错误，也不当作运算符。
        assert_eq!(names(&s, "a\"b OR c*"), vec![BlobHash([10; 32])]);
        assert!(names(&s, "\" OR 1=1 --").is_empty());
        assert!(names(&s, "   ").is_empty());
    }

    #[test]
    fn query_filters_by_kind_conv_author_and_paginates() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..5u64 {
            put(
                &s,
                &files_env(i + 1, 100 + i as i64, &[("p.png", "image/png")]),
            );
        }
        put(&s, &files_env(9, 50, &[("d.pdf", "application/pdf")]));
        let q = |kind: Option<&str>, before: Option<i64>, limit: u32| {
            s.query_file_rows(&FileQuery {
                kind: kind.map(String::from),
                before_millis: before,
                limit,
                ..Default::default()
            })
            .unwrap()
        };
        assert_eq!(q(Some("image"), None, 100).len(), 5);
        assert_eq!(q(Some("document"), None, 100).len(), 1);
        let page = q(Some("image"), None, 2);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].hlc_millis, 104);
        let next = q(Some("image"), Some(page[1].hlc_millis), 10);
        assert_eq!(next.len(), 3);
        let other_conv = s
            .query_file_rows(&FileQuery {
                conv: Some(ConvId([9u8; 32])),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(other_conv.is_empty());
        let by_author = s
            .query_file_rows(&FileQuery {
                author: Some(AUTHOR),
                limit: 100,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_author.len(), 6);
    }

    #[test]
    fn same_millisecond_rows_are_not_skipped_by_cursor() {
        let s = Store::open_in_memory().unwrap();
        // 一个消息里 7 个文件，hlc 相同：limit=3 的页必须带出同毫秒的剩余行。
        let items: Vec<(String, &str)> =
            (0..7).map(|i| (format!("f{i}.png"), "image/png")).collect();
        let refs: Vec<(&str, &str)> = items.iter().map(|(n, m)| (n.as_str(), *m)).collect();
        put(&s, &files_env(1, 500, &refs));
        let page = s
            .query_file_rows(&FileQuery {
                limit: 3,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.len(), 7);
        let next = s
            .query_file_rows(&FileQuery {
                before_millis: Some(500),
                limit: 3,
                ..Default::default()
            })
            .unwrap();
        assert!(next.is_empty());
    }

    #[test]
    fn hidden_deleted_and_recalled_files_not_returned() {
        let s = Store::open_in_memory().unwrap();
        let visible = files_env(1, 100, &[("v.png", "image/png")]);
        let hidden = files_env(2, 101, &[("h.png", "image/png")]);
        let deleted = files_env(3, 102, &[("d.png", "image/png")]);
        put(&s, &visible);
        s.insert_envelope_hidden(&hidden, MsgStateRow::Delivered, 1, true)
            .unwrap();
        put(&s, &deleted);
        s.mark_deleted(std::slice::from_ref(&deleted.id)).unwrap();
        let rows = s
            .query_file_rows(&FileQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].msg, visible.id);
    }

    #[test]
    fn backfill_indexes_existing_messages_once() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..3u64 {
            put(
                &s,
                &files_env(
                    i + 1,
                    100 + i as i64,
                    &[("a.png", "image/png"), ("b.txt", "text/plain")],
                ),
            );
        }
        put(
            &s,
            &env(
                10,
                200,
                MsgBody::Text {
                    text: "你好世界".into(),
                },
            ),
        );
        assert_eq!(index_count(&s), 6);
        // 模拟旧库：清空索引并去掉版本标记。
        s.conn()
            .unwrap()
            .execute_batch("DELETE FROM file_index; DELETE FROM message_text_index; DELETE FROM settings WHERE key = 'search_index_v';")
            .unwrap();
        assert_eq!(index_count(&s), 0);
        assert_eq!(s.backfill_search_index().unwrap(), 4);
        assert_eq!(index_count(&s), 6);
        assert_eq!(s.search_messages("世界", None, 10).unwrap().len(), 1);
        assert_eq!(s.backfill_search_index().unwrap(), 0);
        assert_eq!(index_count(&s), 6);
    }

    #[test]
    fn backfill_skips_undecodable_envelope() {
        let s = Store::open_in_memory().unwrap();
        let bad = files_env(1, 100, &[("bad.png", "image/png")]);
        let good = files_env(2, 101, &[("good.png", "image/png")]);
        put(&s, &bad);
        put(&s, &good);
        // 破坏第一条的信封，并清空索引模拟旧库。
        s.conn()
            .unwrap()
            .execute(
                "UPDATE messages SET envelope = x'ff' WHERE author = ?1 AND seq = ?2",
                rusqlite::params![&bad.id.author.0[..], bad.id.seq as i64],
            )
            .unwrap();
        s.conn()
            .unwrap()
            .execute_batch("DELETE FROM file_index; DELETE FROM message_text_index; DELETE FROM settings WHERE key = 'search_index_v';")
            .unwrap();
        assert_eq!(s.backfill_search_index().unwrap(), 1);
        let rows = s
            .query_file_rows(&FileQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].msg, good.id);
    }

    #[test]
    fn unindex_on_delete() {
        let s = Store::open_in_memory().unwrap();
        let e = files_env(1, 100, &[("annual.pdf", "application/pdf")]);
        put(&s, &e);
        assert_eq!(names(&s, "annual").len(), 1);
        s.unindex_message(&e.id).unwrap();
        assert_eq!(index_count(&s), 0);
        let fts: i64 = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM file_index_fts WHERE file_index_fts MATCH '\"annual\"'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fts, 0);
    }

    #[test]
    fn unindex_on_recall() {
        let s = Store::open_in_memory().unwrap();
        let f = files_env(1, 100, &[("annual.pdf", "application/pdf")]);
        let t = env(
            2,
            101,
            MsgBody::Text {
                text: "明天开会吧".into(),
            },
        );
        put(&s, &f);
        put(&s, &t);
        assert_eq!(s.search_messages("开会", None, 10).unwrap().len(), 1);
        for (seq, target) in [(3u64, &f), (4u64, &t)] {
            let r = env(
                seq,
                110,
                MsgBody::Recall {
                    target: target.id.clone(),
                },
            );
            put(&s, &r);
        }
        assert_eq!(index_count(&s), 0);
        assert!(names(&s, "annual").is_empty());
        assert!(s.search_messages("开会", None, 10).unwrap().is_empty());
    }

    #[test]
    fn undo_recall_restores_index() {
        let s = Store::open_in_memory().unwrap();
        let f = files_env(1, 100, &[("annual.pdf", "application/pdf")]);
        put(&s, &f);
        put(
            &s,
            &env(
                3,
                110,
                MsgBody::Recall {
                    target: f.id.clone(),
                },
            ),
        );
        assert!(names(&s, "annual").is_empty());
        s.undo_recall(&f.id).unwrap();
        assert_eq!(names(&s, "annual").len(), 1);
    }

    #[test]
    fn fts5_trigram_is_available() {
        assert!(Store::open_in_memory()
            .unwrap()
            .fts5_trigram_available()
            .unwrap());
    }
}
