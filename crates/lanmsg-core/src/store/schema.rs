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
];
