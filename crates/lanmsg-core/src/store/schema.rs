// P1 核心骨架：数据库 schema 迁移（Task 6）。
// MIGRATIONS[i] 把版本从 i 升到 i+1，事务内执行。
// 后续阶段只能在数组末尾追加，不得修改已有元素。
// 注释中文，标识符英文。

/// 所有 schema 迁移脚本。第 i 个元素把数据库从版本 i 升到 i+1。
pub const MIGRATIONS: &[&str] = &[
    // 迁移 1：初始 schema。
    "
CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE users(user_id BLOB PRIMARY KEY, display_name TEXT NOT NULL,
  is_self INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL);
CREATE TABLE devices(device_id BLOB PRIMARY KEY, user_id BLOB NOT NULL, cert BLOB NOT NULL,
  name TEXT NOT NULL, device_type TEXT NOT NULL, last_addr TEXT, last_path TEXT,
  last_seen INTEGER, revoked INTEGER NOT NULL DEFAULT 0);
CREATE INDEX devices_by_user ON devices(user_id);
CREATE TABLE conversations(conv_id BLOB PRIMARY KEY, kind TEXT NOT NULL, peer_user BLOB,
  title TEXT, created_at INTEGER NOT NULL);
CREATE TABLE messages(author BLOB NOT NULL, seq INTEGER NOT NULL, conv_id BLOB NOT NULL,
  conv_seq INTEGER NOT NULL, hlc_millis INTEGER NOT NULL, hlc_counter INTEGER NOT NULL,
  envelope BLOB NOT NULL, state TEXT NOT NULL, received_at INTEGER NOT NULL, text TEXT,
  PRIMARY KEY(author, seq), UNIQUE(conv_id, author, conv_seq));
CREATE INDEX messages_order ON messages(conv_id, hlc_millis, hlc_counter, author, seq);
CREATE TABLE version_vectors(conv_id BLOB NOT NULL, author BLOB NOT NULL,
  contiguous INTEGER NOT NULL, PRIMARY KEY(conv_id, author));
",
    // 迁移 2（P2）：消息归属与隐藏、会话准入、联系人/请求/吊销、撤回、
    // preparing、blob 与传输表。
    "
ALTER TABLE messages ADD COLUMN author_user BLOB;
UPDATE messages SET author_user = (SELECT user_id FROM devices WHERE devices.device_id = messages.author);
ALTER TABLE messages ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
CREATE INDEX messages_by_user ON messages(conv_id, author_user);
ALTER TABLE conversations ADD COLUMN gate TEXT NOT NULL DEFAULT 'open';
ALTER TABLE conversations ADD COLUMN gate_until INTEGER;
ALTER TABLE conversations ADD COLUMN last_read_millis INTEGER NOT NULL DEFAULT 0;
ALTER TABLE conversations ADD COLUMN last_read_counter INTEGER NOT NULL DEFAULT 0;
CREATE TABLE contacts(user_id BLOB PRIMARY KEY, state TEXT NOT NULL,
  auto_accept INTEGER NOT NULL DEFAULT 1, ask_above_bytes INTEGER,
  updated_millis INTEGER NOT NULL, updated_counter INTEGER NOT NULL);
CREATE TABLE requests(request_id TEXT PRIMARY KEY, kind TEXT NOT NULL, from_user BLOB NOT NULL,
  conv_id BLOB, preview TEXT NOT NULL, created_at INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'open');
CREATE INDEX requests_open ON requests(status, from_user, kind);
CREATE TABLE revocations(device_id BLOB PRIMARY KEY, user_id BLOB NOT NULL, record BLOB NOT NULL);
CREATE TABLE recalls(target_author BLOB NOT NULL, target_seq INTEGER NOT NULL,
  recall_author BLOB NOT NULL, recall_seq INTEGER NOT NULL, PRIMARY KEY(target_author, target_seq));
CREATE TABLE preparing(author BLOB NOT NULL, seq INTEGER NOT NULL, conv_id BLOB NOT NULL,
  conv_seq INTEGER NOT NULL, hlc_millis INTEGER NOT NULL, hlc_counter INTEGER NOT NULL,
  kind TEXT NOT NULL, paths BLOB NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY(author, seq));
CREATE TABLE blobs(hash BLOB PRIMARY KEY, size INTEGER NOT NULL, mime TEXT NOT NULL,
  segment_hashes BLOB NOT NULL);
CREATE TABLE blob_local(hash BLOB PRIMARY KEY, path TEXT NOT NULL, kind TEXT NOT NULL,
  size INTEGER NOT NULL, mtime INTEGER NOT NULL);
CREATE TABLE blob_holders(hash BLOB NOT NULL, device_id BLOB NOT NULL, updated_at INTEGER NOT NULL,
  PRIMARY KEY(hash, device_id));
CREATE TABLE message_blobs(author BLOB NOT NULL, seq INTEGER NOT NULL, idx INTEGER NOT NULL,
  conv_id BLOB NOT NULL, hash BLOB NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL,
  mime TEXT NOT NULL, rel_path TEXT, hlc_millis INTEGER NOT NULL, hlc_counter INTEGER NOT NULL,
  meta BLOB NOT NULL, PRIMARY KEY(author, seq, idx));
CREATE INDEX message_blobs_hash ON message_blobs(hash);
CREATE INDEX message_blobs_conv ON message_blobs(conv_id, hlc_millis);
CREATE TABLE transfers(transfer_id TEXT PRIMARY KEY, hash BLOB NOT NULL, state TEXT NOT NULL,
  part_path TEXT NOT NULL, dest_path TEXT NOT NULL, total INTEGER NOT NULL,
  segment_hashes BLOB NOT NULL, done_segments BLOB NOT NULL, error TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX transfers_hash ON transfers(hash, state);
",
    // 迁移 3（P3）：文件视图索引与消息文字搜索索引（FTS5 trigram，外部内容表）。
    // 索引由 insert_envelope 同事务维护，旧库由 backfill_search_index 一次性回填。
    "
CREATE TABLE file_index(id INTEGER PRIMARY KEY, conv_id BLOB NOT NULL, author BLOB NOT NULL,
  seq INTEGER NOT NULL, item_idx INTEGER NOT NULL, hash BLOB NOT NULL, name TEXT NOT NULL,
  mime TEXT NOT NULL, size INTEGER NOT NULL, kind TEXT NOT NULL, hlc_millis INTEGER NOT NULL,
  UNIQUE(author, seq, item_idx));
CREATE INDEX file_index_order ON file_index(hlc_millis DESC);
CREATE INDEX file_index_conv ON file_index(conv_id, hlc_millis DESC);
CREATE VIRTUAL TABLE file_index_fts USING fts5(name, content='file_index', content_rowid='id', tokenize='trigram');
CREATE TRIGGER file_index_ai AFTER INSERT ON file_index BEGIN
  INSERT INTO file_index_fts(rowid, name) VALUES (new.id, new.name); END;
CREATE TRIGGER file_index_ad AFTER DELETE ON file_index BEGIN
  INSERT INTO file_index_fts(file_index_fts, rowid, name) VALUES('delete', old.id, old.name); END;
CREATE TABLE message_text_index(id INTEGER PRIMARY KEY, conv_id BLOB NOT NULL, author BLOB NOT NULL,
  seq INTEGER NOT NULL, hlc_millis INTEGER NOT NULL, text TEXT NOT NULL, UNIQUE(author, seq));
CREATE INDEX message_text_index_conv ON message_text_index(conv_id, hlc_millis DESC);
CREATE VIRTUAL TABLE message_text_fts USING fts5(text, content='message_text_index', content_rowid='id', tokenize='trigram');
CREATE TRIGGER message_text_ai AFTER INSERT ON message_text_index BEGIN
  INSERT INTO message_text_fts(rowid, text) VALUES (new.id, new.text); END;
CREATE TRIGGER message_text_ad AFTER DELETE ON message_text_index BEGIN
  INSERT INTO message_text_fts(message_text_fts, rowid, text) VALUES('delete', old.id, old.text); END;
",
    // 迁移 4（P5）：地址簿、手动设备、中转站、LocalSend 对端、Web 访客。
    "
CREATE TABLE known_addresses(device_id BLOB NOT NULL, addr TEXT NOT NULL, path TEXT NOT NULL,
  last_ok INTEGER NOT NULL, source TEXT NOT NULL, PRIMARY KEY(device_id, addr));
CREATE TABLE manual_peers(addr TEXT PRIMARY KEY, added_at INTEGER NOT NULL, device_id BLOB);
CREATE TABLE stations(station_id BLOB PRIMARY KEY, name TEXT NOT NULL, host_device BLOB NOT NULL,
  owner_user BLOB NOT NULL, acl BLOB NOT NULL, created_at INTEGER NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0, synced_until INTEGER NOT NULL DEFAULT 0);
CREATE TABLE station_items(item_id BLOB PRIMARY KEY, station_id BLOB NOT NULL, added_by BLOB NOT NULL,
  added_at INTEGER NOT NULL, expires_at INTEGER, content BLOB NOT NULL, pinned INTEGER NOT NULL DEFAULT 0,
  removed INTEGER NOT NULL DEFAULT 0, available INTEGER NOT NULL DEFAULT 1, updated_at INTEGER NOT NULL,
  store_mode TEXT, local_path TEXT, local_size INTEGER, local_mtime INTEGER);
CREATE INDEX station_items_by_station ON station_items(station_id, updated_at);
CREATE INDEX station_items_by_expiry ON station_items(expires_at);
CREATE TABLE station_blobs(item_id BLOB NOT NULL, hash BLOB NOT NULL, name TEXT NOT NULL,
  size INTEGER NOT NULL, mime TEXT NOT NULL, rel_path TEXT NOT NULL DEFAULT '', meta BLOB NOT NULL,
  PRIMARY KEY(item_id, rel_path, hash));
CREATE INDEX station_blobs_by_hash ON station_blobs(hash);
CREATE TABLE localsend_peers(fingerprint TEXT PRIMARY KEY, pseudo_device BLOB NOT NULL, alias TEXT NOT NULL,
  device_model TEXT, device_type TEXT, last_addr TEXT, protocol TEXT NOT NULL, port INTEGER NOT NULL,
  last_seen INTEGER NOT NULL, trusted INTEGER NOT NULL DEFAULT 0, blocked INTEGER NOT NULL DEFAULT 0);
CREATE INDEX localsend_peers_by_device ON localsend_peers(pseudo_device);
CREATE TABLE web_guests(conv_id BLOB PRIMARY KEY, pseudo_device BLOB NOT NULL, user_agent TEXT NOT NULL,
  ip TEXT NOT NULL, started_at INTEGER NOT NULL, ended_at INTEGER);
CREATE INDEX web_guests_by_device ON web_guests(pseudo_device);
",
];

#[cfg(test)]
mod tests {
    use super::MIGRATIONS;
    use crate::store::Store;
    use rusqlite::Connection;

    const P5_TABLES: [&str; 7] = [
        "known_addresses",
        "manual_peers",
        "stations",
        "station_items",
        "station_blobs",
        "localsend_peers",
        "web_guests",
    ];
    const P5_INDEXES: [&str; 5] = [
        "station_items_by_station",
        "station_items_by_expiry",
        "station_blobs_by_hash",
        "localsend_peers_by_device",
        "web_guests_by_device",
    ];

    fn check_p5(conn: &Connection) {
        for table in P5_TABLES {
            let n: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0, "{table}");
        }
        for index in P5_INDEXES {
            let n: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    [index],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "{index}");
        }
    }

    #[test]
    fn p5_migration_creates_tables() {
        let s = Store::open_in_memory().unwrap();
        check_p5(&s.conn().unwrap());
    }

    #[test]
    fn p5_migration_upgrades_from_v3() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lanmsg.db");
        {
            let conn = Connection::open(&path).unwrap();
            for (i, sql) in MIGRATIONS[..3].iter().enumerate() {
                conn.execute_batch(sql).unwrap();
                conn.pragma_update(None, "user_version", i as u32 + 1)
                    .unwrap();
            }
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as u32);
        check_p5(&s.conn().unwrap());
    }

    #[test]
    fn station_blobs_allows_same_hash_under_different_rel_paths() {
        let s = Store::open_in_memory().unwrap();
        let conn = s.conn().unwrap();
        let sql = "INSERT INTO station_blobs(item_id, hash, name, size, mime, rel_path, meta)
                   VALUES(x'01', x'02', 'a.txt', 1, 'text/plain', ?1, x'00')";
        conn.execute(sql, ["d/a.txt"]).unwrap();
        conn.execute(sql, ["e/a.txt"]).unwrap();
        // 相同 (item, rel_path, hash) 仍然冲突。
        assert!(conn.execute(sql, ["e/a.txt"]).is_err());
    }
}
