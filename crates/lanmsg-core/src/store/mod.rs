// P1 核心骨架：存储层入口（Task 6）。
// Store 基于 rusqlite，所有方法 &self、同步执行，不跨 .await 持锁。
// 注释中文，标识符英文。

pub mod blobs;
pub mod contacts;
pub mod convs;
pub mod messages;
pub mod schema;
pub mod users;

pub use contacts::{ContactRow, ContactState, RequestKindRow, RequestRow};
pub use convs::{ConvKindRow, ConvRow, Gate};
pub use messages::{MsgStateRow, StoredMessage};
pub use users::{DeviceRow, UserRow};

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::store::schema::MIGRATIONS;

/// 存储层错误。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite 错误。
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// 编解码错误。
    #[error("编解码错误: {0}")]
    Codec(String),
}

/// SQLite 存储，所有方法 `&self`、同步执行，不跨 `.await` 持锁。
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// 打开（或创建）`path` 的文件数据库，开启 WAL 与外键约束，执行迁移。
    pub fn open(path: &Path) -> Result<Store, StoreError> {
        let mut conn = Connection::open(path)?;
        Self::init_conn(&mut conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    /// 打开内存数据库，开启 WAL 与外键约束，执行迁移。
    pub fn open_in_memory() -> Result<Store, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        Self::init_conn(&mut conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    fn init_conn(conn: &mut Connection) -> Result<(), StoreError> {
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::run_migrations(conn)?;
        Ok(())
    }

    /// 执行所有尚未应用的迁移。按顺序执行 MIGRATIONS[user_version..]，每个在事务内；
    /// 失败回滚并返回 StoreError。
    fn run_migrations(conn: &mut Connection) -> Result<(), StoreError> {
        let user_version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let migrations = MIGRATIONS;
        if (user_version as usize) >= migrations.len() {
            return Ok(());
        }
        for (i, sql) in migrations[user_version as usize..].iter().enumerate() {
            let version = user_version + i as u32;
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute_batch(&format!("PRAGMA user_version = {}", version + 1))?;
            tx.commit()?;
        }
        Ok(())
    }

    /// 返回当前 schema 版本（`PRAGMA user_version`）。
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        let conn = self.conn().map_err(|e| StoreError::Codec(e.to_string()))?;
        let v: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        Ok(v)
    }

    /// 读取 settings 中的键值，不存在返回 Ok(None)。
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self.conn().map_err(|e| StoreError::Codec(e.to_string()))?;
        let row: Option<String> = conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(row)
    }

    /// 写入 settings 中的键值（存在则覆盖）。
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let conn = self.conn().map_err(|e| StoreError::Codec(e.to_string()))?;
        conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    pub(crate) fn conn(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn migrations_set_user_version() {
        assert_eq!(store().schema_version().unwrap(), MIGRATIONS.len() as u32);
    }

    #[test]
    fn schema_at_least_v2_with_new_tables() {
        let s = store();
        assert!(s.schema_version().unwrap() >= 2);
        let conn = s.conn().unwrap();
        for table in [
            "contacts",
            "requests",
            "revocations",
            "recalls",
            "preparing",
            "blobs",
            "blob_local",
            "blob_holders",
            "message_blobs",
            "transfers",
        ] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap_or_else(|e| panic!("表 {table} 查询失败: {e}"));
            assert_eq!(n, 0, "表 {table} 应为空");
        }
    }
}
