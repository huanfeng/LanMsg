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
];

#[cfg(test)]
mod tests {}
