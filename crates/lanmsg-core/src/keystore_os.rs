// P3 Task 7：平台密钥库。
// 桌面端优先把密钥放进系统密钥库（Windows 凭据管理器 / macOS 钥匙串 / Linux Secret Service），
// 系统密钥库不可用（无头 Linux、无会话 D-Bus 等）时回退到文件密钥库并记 warn。
// 注释中文，标识符英文。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::KeyStoreError;
use crate::keystore::{FileKeyStore, KeyStore};

/// 系统密钥库后端抽象（便于测试注入）。
pub trait SecretBackend: Send + Sync {
    /// 读取名为 `name` 的密钥，不存在返回 Ok(None)。
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError>;
    /// 写入密钥。
    fn set(&self, name: &str, bytes: &[u8]) -> Result<(), KeyStoreError>;
    /// 删除密钥，不存在视为成功。
    fn remove(&self, name: &str) -> Result<(), KeyStoreError>;
}

/// 基于 `keyring` crate 的后端；`service` 为密钥库中的服务名。
pub struct KeyringBackend {
    service: String,
}

impl KeyringBackend {
    pub fn new(service: String) -> Self {
        Self { service }
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry, KeyStoreError> {
        keyring::Entry::new(&self.service, name).map_err(to_store_err)
    }
}

/// keyring 错误统一映射为 IO 错误（后端不可用不是数据损坏）。
fn to_store_err(e: keyring::Error) -> KeyStoreError {
    KeyStoreError::Io(std::io::Error::other(e.to_string()))
}

impl SecretBackend for KeyringBackend {
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        match self.entry(name)?.get_secret() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(to_store_err(e)),
        }
    }

    fn set(&self, name: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.entry(name)?.set_secret(bytes).map_err(to_store_err)
    }

    fn remove(&self, name: &str) -> Result<(), KeyStoreError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(to_store_err(e)),
        }
    }
}

/// 平台密钥库：primary（系统密钥库）优先，出错回退 fallback（文件密钥库）。
pub struct OsKeyStore {
    primary: Option<Box<dyn SecretBackend>>,
    fallback: FileKeyStore,
    /// primary 出错的 warn 只记一次，避免刷屏。
    warned: AtomicBool,
}

impl OsKeyStore {
    pub fn new(primary: Option<Box<dyn SecretBackend>>, fallback: FileKeyStore) -> Self {
        Self {
            primary,
            fallback,
            warned: AtomicBool::new(false),
        }
    }

    fn warn_primary_failed(&self, err: &KeyStoreError) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!("系统密钥库不可用，回退到文件密钥库: {err}");
        }
    }
}

impl KeyStore for OsKeyStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        let Some(primary) = &self.primary else {
            return self.fallback.load(name);
        };
        match primary.get(name) {
            Ok(Some(v)) => Ok(Some(v)),
            Ok(None) => {
                // 迁移：primary 没有而文件库有 → 写入 primary，成功后删除文件。
                let Some(v) = self.fallback.load(name)? else {
                    return Ok(None);
                };
                match primary.set(name, &v) {
                    Ok(()) => {
                        if let Err(e) = self.fallback.delete(name) {
                            tracing::warn!("密钥已迁入系统密钥库，但删除旧文件失败: {e}");
                        }
                    }
                    Err(e) => self.warn_primary_failed(&e),
                }
                Ok(Some(v))
            }
            Err(e) => {
                self.warn_primary_failed(&e);
                self.fallback.load(name)
            }
        }
    }

    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        if let Some(primary) = &self.primary {
            match primary.set(name, bytes) {
                Ok(()) => return Ok(()),
                Err(e) => self.warn_primary_failed(&e),
            }
        }
        self.fallback.save(name, bytes)
    }

    fn delete(&self, name: &str) -> Result<(), KeyStoreError> {
        if let Some(primary) = &self.primary {
            if let Err(e) = primary.remove(name) {
                self.warn_primary_failed(&e);
            }
        }
        // 两处都删，避免残留的文件副本在 primary 恢复后被迁回。
        self.fallback.delete(name)
    }
}

/// 构造平台密钥库：服务名 `LanMsg-<blake3(规范化 data_dir) 前 8 位 hex>`，
/// 文件回退目录为 `<data_dir>/keys`。
pub fn os_keystore(data_dir: &Path) -> Arc<dyn KeyStore> {
    let canonical = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let digest = blake3::hash(canonical.to_string_lossy().as_bytes());
    let service = format!("LanMsg-{}", &digest.to_hex()[..8]);
    Arc::new(OsKeyStore::new(
        Some(Box::new(KeyringBackend::new(service))),
        FileKeyStore::new(data_dir.join("keys")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// 内存后端：模拟可用的系统密钥库。
    #[derive(Clone, Default)]
    struct MemBackend(Arc<Mutex<HashMap<String, Vec<u8>>>>);

    impl SecretBackend for MemBackend {
        fn get(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
            Ok(self.0.lock().unwrap().get(name).cloned())
        }
        fn set(&self, name: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
            self.0.lock().unwrap().insert(name.into(), bytes.to_vec());
            Ok(())
        }
        fn remove(&self, name: &str) -> Result<(), KeyStoreError> {
            self.0.lock().unwrap().remove(name);
            Ok(())
        }
    }

    /// 始终失败的后端：模拟无 Secret Service 的环境。
    struct FailingBackend;

    fn unavailable() -> KeyStoreError {
        KeyStoreError::Io(std::io::Error::other("模拟：系统密钥库不可用"))
    }

    impl SecretBackend for FailingBackend {
        fn get(&self, _: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
            Err(unavailable())
        }
        fn set(&self, _: &str, _: &[u8]) -> Result<(), KeyStoreError> {
            Err(unavailable())
        }
        fn remove(&self, _: &str) -> Result<(), KeyStoreError> {
            Err(unavailable())
        }
    }

    #[test]
    fn uses_primary_when_available() {
        let dir = tempfile::tempdir().unwrap();
        let mem = MemBackend::default();
        let ks = OsKeyStore::new(
            Some(Box::new(mem.clone())),
            FileKeyStore::new(dir.path().join("keys")),
        );
        ks.save("device.key", b"secret").unwrap();
        assert_eq!(
            mem.get("device.key").unwrap().as_deref(),
            Some(&b"secret"[..])
        );
        assert!(!dir.path().join("keys").join("device.key").exists());
        assert_eq!(
            ks.load("device.key").unwrap().as_deref(),
            Some(&b"secret"[..])
        );
        ks.delete("device.key").unwrap();
        assert_eq!(ks.load("device.key").unwrap(), None);
    }

    #[test]
    fn falls_back_to_file_when_primary_fails() {
        let dir = tempfile::tempdir().unwrap();
        let ks = OsKeyStore::new(
            Some(Box::new(FailingBackend)),
            FileKeyStore::new(dir.path().join("keys")),
        );
        ks.save("device.key", b"secret").unwrap();
        assert!(dir.path().join("keys").join("device.key").exists());
        assert_eq!(
            ks.load("device.key").unwrap().as_deref(),
            Some(&b"secret"[..])
        );
        ks.delete("device.key").unwrap();
        assert_eq!(ks.load("device.key").unwrap(), None);
    }

    #[test]
    fn no_primary_uses_file_only() {
        let dir = tempfile::tempdir().unwrap();
        let ks = OsKeyStore::new(None, FileKeyStore::new(dir.path().join("keys")));
        ks.save("a", b"1").unwrap();
        assert_eq!(ks.load("a").unwrap().as_deref(), Some(&b"1"[..]));
    }

    #[test]
    fn migrates_file_key_into_primary_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let fallback = FileKeyStore::new(dir.path().join("keys"));
        fallback.save("device.key", b"old").unwrap();
        let mem = MemBackend::default();
        let ks = OsKeyStore::new(
            Some(Box::new(mem.clone())),
            FileKeyStore::new(dir.path().join("keys")),
        );
        assert_eq!(ks.load("device.key").unwrap().as_deref(), Some(&b"old"[..]));
        assert_eq!(mem.get("device.key").unwrap().as_deref(), Some(&b"old"[..]));
        assert!(!dir.path().join("keys").join("device.key").exists());
    }

    #[test]
    fn migration_failure_keeps_file_and_returns_value() {
        // primary 读返回 None 但写失败：值照常返回，文件不删。
        struct ReadOkWriteFail;
        impl SecretBackend for ReadOkWriteFail {
            fn get(&self, _: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
                Ok(None)
            }
            fn set(&self, _: &str, _: &[u8]) -> Result<(), KeyStoreError> {
                Err(unavailable())
            }
            fn remove(&self, _: &str) -> Result<(), KeyStoreError> {
                Ok(())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let fallback = FileKeyStore::new(dir.path().join("keys"));
        fallback.save("k", b"v").unwrap();
        let ks = OsKeyStore::new(Some(Box::new(ReadOkWriteFail)), fallback);
        assert_eq!(ks.load("k").unwrap().as_deref(), Some(&b"v"[..]));
        assert!(dir.path().join("keys").join("k").exists());
    }

    /// 真实系统密钥库读写；检测到不可用（CI 无 Secret Service 等）时跳过。
    #[test]
    fn keyring_backend_roundtrip_if_available() {
        let backend = KeyringBackend::new(format!("LanMsg-test-{}", std::process::id()));
        if let Err(e) = backend.set("probe", b"hello") {
            println!("跳过：本机系统密钥库不可用（{e}）");
            return;
        }
        assert_eq!(
            backend.get("probe").unwrap().as_deref(),
            Some(&b"hello"[..])
        );
        backend.remove("probe").unwrap();
        assert_eq!(backend.get("probe").unwrap(), None);
        // 删除不存在的条目视为成功。
        backend.remove("probe").unwrap();
    }
}
