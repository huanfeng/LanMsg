// P3 Task 7：平台密钥库。
// 桌面端优先把密钥放进系统密钥库（Windows 凭据管理器 / macOS 钥匙串 / Linux Secret Service），
// 后端（系统库/文件库）首次使用时选定一次并持久化，之后出错不再回退（见 OsKeyStore）。
// 注释中文，标识符英文。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use lanmsg_proto::consts::{
    KEYSTORE_BACKEND_FILE, KEYSTORE_BACKEND_MARKER, KEYSTORE_BACKEND_OS, KEYSTORE_PROBE_NAME,
};

use crate::error::KeyStoreError;
use crate::identity::{KEY_DEVICE, KEY_USER};
use crate::keystore::{atomic_write, FileKeyStore, KeyStore};

/// 首次选择用的跨进程锁文件名（位于数据目录）。
const KEYSTORE_LOCK_NAME: &str = "keystore.lock";

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

/// 已选定的后端。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Os,
    File,
}

/// 平台密钥库：后端在首次使用时选定一次并持久化到数据目录的标记文件。
///
/// - 标记 `file`：只用文件库，不再尝试系统库。
/// - 标记 `os`：只用系统库；系统库出错一律返回 Err，绝不回退文件、绝不把错误当“不存在”
///   （否则登录时 Secret Service 尚未解锁会导致身份被重建）。
/// - 标记不存在：持 `keystore.lock` 文件锁重读标记后再选择；文件库已有密钥（老用户）则尝试迁移（写入后读回比对一致才删文件），
///   文件库为空则探测系统库；成功选 `os`，否则选 `file`。
pub struct OsKeyStore {
    primary: Option<Box<dyn SecretBackend>>,
    fallback: FileKeyStore,
    marker: PathBuf,
    mode: Mutex<Option<Mode>>,
    /// 迁移后删旧文件失败的 warn 只记一次，避免刷屏。
    warned: AtomicBool,
}

impl OsKeyStore {
    /// `marker` 为后端选择标记文件路径；`primary` 为 None 时恒用文件库（不写标记）。
    pub fn new(
        primary: Option<Box<dyn SecretBackend>>,
        fallback: FileKeyStore,
        marker: PathBuf,
    ) -> Self {
        Self {
            primary,
            fallback,
            marker,
            mode: Mutex::new(None),
            warned: AtomicBool::new(false),
        }
    }

    fn warn_once(&self, msg: &str, err: &KeyStoreError) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!("{msg}: {err}");
        }
    }

    /// 读标记：不存在 Ok(None)；空/非法内容明确报错（不得当作无标记，否则可能误重建身份）。
    fn read_marker(&self) -> Result<Option<Mode>, KeyStoreError> {
        match std::fs::read_to_string(&self.marker) {
            Ok(text) => match text.trim() {
                KEYSTORE_BACKEND_OS => Ok(Some(Mode::Os)),
                KEYSTORE_BACKEND_FILE => Ok(Some(Mode::File)),
                other => Err(KeyStoreError::Corrupt(format!(
                    "密钥库后端标记 {} 内容为空或非法（{other:?}），可删除该文件后重试",
                    self.marker.display()
                ))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(KeyStoreError::Io(e)),
        }
    }

    /// 取得（必要时选定并持久化）后端。
    fn resolve(&self) -> Result<(Mode, &dyn SecretBackend), KeyStoreError> {
        let Some(primary) = self.primary.as_deref() else {
            return Ok((Mode::File, &NoBackend));
        };
        let mut guard = self.mode.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mode) = *guard {
            return Ok((mode, primary));
        }
        // 已有标记的快路径不加锁；无标记则在跨进程文件锁内重读、选择、写标记。
        let mode = match self.read_marker()? {
            Some(mode) => mode,
            None => self.select_locked(primary)?,
        };
        if mode == Mode::Os {
            self.cleanup_legacy(primary);
        }
        *guard = Some(mode);
        Ok((mode, primary))
    }

    /// 持有 `keystore.lock` 独占锁完成首次选择，避免两进程同时首次启动互相覆盖。
    fn select_locked(&self, primary: &dyn SecretBackend) -> Result<Mode, KeyStoreError> {
        let dir = self.marker.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(KEYSTORE_LOCK_NAME))?;
        lock.lock()?;
        // 锁内重读：另一进程可能刚选定。锁随 lock 句柄关闭释放。
        let result = match self.read_marker() {
            Ok(Some(mode)) => Ok(mode),
            Ok(None) => self.select_backend(primary),
            Err(e) => Err(e),
        };
        let _ = lock.unlock();
        result
    }

    fn write_marker(&self, value: &str) -> Result<(), KeyStoreError> {
        atomic_write(&self.marker, value.as_bytes(), false)?;
        Ok(())
    }

    /// os 模式下清理残留的文件库密钥：仅当系统库里同名条目可读且值相同才删。
    fn cleanup_legacy(&self, primary: &dyn SecretBackend) {
        for name in [KEY_DEVICE, KEY_USER] {
            let Ok(Some(file_v)) = self.fallback.load(name) else {
                continue;
            };
            if matches!(primary.get(name), Ok(Some(os_v)) if os_v == file_v) {
                if let Err(e) = self.fallback.delete(name) {
                    self.warn_once("密钥已在系统密钥库，但删除旧文件失败", &e);
                }
            }
        }
    }

    /// 标记不存在时的首次选定（调用方须已持有文件锁）。
    fn select_backend(&self, primary: &dyn SecretBackend) -> Result<Mode, KeyStoreError> {
        let mut legacy = Vec::new();
        for name in [KEY_DEVICE, KEY_USER] {
            if let Some(v) = self.fallback.load(name)? {
                legacy.push((name, v));
            }
        }
        if legacy.is_empty() {
            return match probe(primary) {
                Ok(()) => {
                    self.write_marker(KEYSTORE_BACKEND_OS)?;
                    Ok(Mode::Os)
                }
                Err(e) => {
                    tracing::warn!("系统密钥库不可用，改用文件密钥库: {e}");
                    self.write_marker(KEYSTORE_BACKEND_FILE)?;
                    Ok(Mode::File)
                }
            };
        }
        // 迁移：先全部写入并读回比对，全部一致才写标记（已 fsync）；
        // 旧文件由 resolve 随后的 cleanup_legacy 在确认系统库值一致后删除。
        let migrated: Result<(), KeyStoreError> = legacy.iter().try_for_each(|(name, v)| {
            primary.set(name, v)?;
            match primary.get(name)? {
                Some(back) if back == *v => Ok(()),
                _ => Err(KeyStoreError::Corrupt(format!(
                    "密钥 {name} 写入系统密钥库后读回不一致"
                ))),
            }
        });
        match migrated {
            Ok(()) => {
                self.write_marker(KEYSTORE_BACKEND_OS)?;
                Ok(Mode::Os)
            }
            Err(e) => {
                tracing::warn!("密钥迁入系统密钥库失败，保留文件密钥库: {e}");
                for (name, v) in &legacy {
                    // 仅当条目值等于本次写入的值才回滚，避免误删他人写入。
                    if matches!(primary.get(name), Ok(Some(cur)) if cur == *v) {
                        let _ = primary.remove(name);
                    }
                }
                self.write_marker(KEYSTORE_BACKEND_FILE)?;
                Ok(Mode::File)
            }
        }
    }
}

/// 探测系统密钥库：写入、读回比对、删除一个探测条目。
/// 条目名带进程号与随机数，避免多进程互相干扰；删除失败只告警，不影响“可用”结论。
fn probe(primary: &dyn SecretBackend) -> Result<(), KeyStoreError> {
    const PROBE_VALUE: &[u8] = b"lanmsg";
    let name = format!(
        "{KEYSTORE_PROBE_NAME}-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    );
    primary.set(&name, PROBE_VALUE)?;
    let back = primary.get(&name)?;
    if let Err(e) = primary.remove(&name) {
        tracing::warn!("删除探测条目失败（忽略）: {e}");
    }
    if back.as_deref() == Some(PROBE_VALUE) {
        Ok(())
    } else {
        Err(KeyStoreError::Corrupt("探测条目读回不一致".into()))
    }
}

/// 无系统后端时的占位（resolve 在 File 模式下不会调用它）。
struct NoBackend;

impl SecretBackend for NoBackend {
    fn get(&self, _: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        Ok(None)
    }
    fn set(&self, _: &str, _: &[u8]) -> Result<(), KeyStoreError> {
        Ok(())
    }
    fn remove(&self, _: &str) -> Result<(), KeyStoreError> {
        Ok(())
    }
}

/// 系统库出错时统一包装，提示用户解锁后重试。
fn os_unavailable(e: KeyStoreError) -> KeyStoreError {
    KeyStoreError::Io(std::io::Error::other(format!(
        "系统密钥库暂不可用，请解锁后重试: {e}"
    )))
}

impl KeyStore for OsKeyStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        match self.resolve()? {
            (Mode::File, _) => self.fallback.load(name),
            (Mode::Os, os) => os.get(name).map_err(os_unavailable),
        }
    }

    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        match self.resolve()? {
            (Mode::File, _) => self.fallback.save(name, bytes),
            (Mode::Os, os) => os.set(name, bytes).map_err(os_unavailable),
        }
    }

    fn delete(&self, name: &str) -> Result<(), KeyStoreError> {
        match self.resolve()? {
            (Mode::File, _) => self.fallback.delete(name),
            (Mode::Os, os) => os.remove(name).map_err(os_unavailable),
        }
    }
}

/// 构造平台密钥库：服务名 `LanMsg-<blake3(规范化 data_dir) 前 8 位 hex>`，
/// 文件库目录为 `<data_dir>/keys`，后端标记为 `<data_dir>/keystore.backend`。
///
/// 先创建数据目录再规范化，保证服务名稳定。注意：移动数据目录会改变服务名；
/// 标记为 `os` 时系统库里的旧条目将不可见，需先在原路径运行一次导出（本版本不实现）。
pub fn os_keystore(data_dir: &Path) -> Arc<dyn KeyStore> {
    let _ = std::fs::create_dir_all(data_dir);
    let canonical = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let digest = blake3::hash(canonical.to_string_lossy().as_bytes());
    let service = format!("LanMsg-{}", &digest.to_hex()[..8]);
    Arc::new(OsKeyStore::new(
        Some(Box::new(KeyringBackend::new(service))),
        FileKeyStore::new(data_dir.join("keys")),
        data_dir.join(KEYSTORE_BACKEND_MARKER),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;

    /// 可配置的内存后端：模拟可用/不可用/写入后读不回的系统密钥库。
    #[derive(Clone, Default)]
    struct FakeBackend {
        map: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        fail_get: Arc<AtomicBool>,
        fail_set: Arc<AtomicBool>,
        /// set 返回 Ok 但并不真正保存。
        drop_writes: Arc<AtomicBool>,
        sets: Arc<AtomicUsize>,
        /// 第一次 set 前睡 100ms，制造两实例的时间重叠。
        delay_first_set: Arc<AtomicBool>,
    }

    fn unavailable() -> KeyStoreError {
        KeyStoreError::Io(std::io::Error::other("模拟：系统密钥库不可用"))
    }

    impl FakeBackend {
        fn failing() -> Self {
            let b = Self::default();
            b.fail_get.store(true, Ordering::Relaxed);
            b.fail_set.store(true, Ordering::Relaxed);
            b
        }
    }

    impl SecretBackend for FakeBackend {
        fn get(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
            if self.fail_get.load(Ordering::Relaxed) {
                return Err(unavailable());
            }
            Ok(self.map.lock().unwrap().get(name).cloned())
        }
        fn set(&self, name: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
            if self.fail_set.load(Ordering::Relaxed) {
                return Err(unavailable());
            }
            if self.delay_first_set.swap(false, Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            self.sets.fetch_add(1, Ordering::Relaxed);
            if !self.drop_writes.load(Ordering::Relaxed) {
                self.map.lock().unwrap().insert(name.into(), bytes.to_vec());
            }
            Ok(())
        }
        fn remove(&self, name: &str) -> Result<(), KeyStoreError> {
            if self.fail_set.load(Ordering::Relaxed) {
                return Err(unavailable());
            }
            self.map.lock().unwrap().remove(name);
            Ok(())
        }
    }

    fn keys_dir(dir: &Path) -> PathBuf {
        dir.join("keys")
    }

    fn marker(dir: &Path) -> PathBuf {
        dir.join(KEYSTORE_BACKEND_MARKER)
    }

    fn open(dir: &Path, b: &FakeBackend) -> OsKeyStore {
        OsKeyStore::new(
            Some(Box::new(b.clone())),
            FileKeyStore::new(keys_dir(dir)),
            marker(dir),
        )
    }

    fn marker_text(dir: &Path) -> String {
        std::fs::read_to_string(marker(dir)).unwrap()
    }

    #[test]
    fn uses_primary_when_available() {
        let dir = tempfile::tempdir().unwrap();
        let mem = FakeBackend::default();
        let ks = open(dir.path(), &mem);
        ks.save(KEY_DEVICE, b"secret").unwrap();
        assert_eq!(
            mem.get(KEY_DEVICE).unwrap().as_deref(),
            Some(&b"secret"[..])
        );
        assert!(!keys_dir(dir.path()).join(KEY_DEVICE).exists());
        assert_eq!(marker_text(dir.path()), KEYSTORE_BACKEND_OS);
        assert_eq!(
            ks.load(KEY_DEVICE).unwrap().as_deref(),
            Some(&b"secret"[..])
        );
        ks.delete(KEY_DEVICE).unwrap();
        assert_eq!(ks.load(KEY_DEVICE).unwrap(), None);
        // 探测条目已清理。
        assert!(mem
            .map
            .lock()
            .unwrap()
            .keys()
            .all(|k| !k.starts_with(KEYSTORE_PROBE_NAME)));
    }

    #[test]
    fn falls_back_to_file_when_primary_unavailable_at_selection() {
        let dir = tempfile::tempdir().unwrap();
        let ks = open(dir.path(), &FakeBackend::failing());
        ks.save(KEY_DEVICE, b"secret").unwrap();
        assert!(keys_dir(dir.path()).join(KEY_DEVICE).exists());
        assert_eq!(marker_text(dir.path()), KEYSTORE_BACKEND_FILE);
        assert_eq!(
            ks.load(KEY_DEVICE).unwrap().as_deref(),
            Some(&b"secret"[..])
        );
        ks.delete(KEY_DEVICE).unwrap();
        assert_eq!(ks.load(KEY_DEVICE).unwrap(), None);
    }

    #[test]
    fn no_primary_uses_file_only() {
        let dir = tempfile::tempdir().unwrap();
        let ks = OsKeyStore::new(
            None,
            FileKeyStore::new(keys_dir(dir.path())),
            marker(dir.path()),
        );
        ks.save("a", b"1").unwrap();
        assert_eq!(ks.load("a").unwrap().as_deref(), Some(&b"1"[..]));
        assert!(!marker(dir.path()).exists());
    }

    #[test]
    fn fresh_install_probe_selects_os_or_file() {
        // 可用 → os，重开沿用标记（即便此后系统库坏了也不改选）。
        let dir = tempfile::tempdir().unwrap();
        let mem = FakeBackend::default();
        open(dir.path(), &mem).load(KEY_DEVICE).unwrap();
        assert_eq!(marker_text(dir.path()), KEYSTORE_BACKEND_OS);
        let broken = FakeBackend::failing();
        assert!(open(dir.path(), &broken).load(KEY_DEVICE).is_err());

        // 不可用 → file，重开沿用标记（此后系统库恢复也只用文件库）。
        let dir = tempfile::tempdir().unwrap();
        open(dir.path(), &FakeBackend::failing())
            .save(KEY_DEVICE, b"v")
            .unwrap();
        assert_eq!(marker_text(dir.path()), KEYSTORE_BACKEND_FILE);
        let healthy = FakeBackend::default();
        let ks = open(dir.path(), &healthy);
        assert_eq!(ks.load(KEY_DEVICE).unwrap().as_deref(), Some(&b"v"[..]));
        assert_eq!(healthy.sets.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn legacy_file_keys_migrated_with_readback() {
        let dir = tempfile::tempdir().unwrap();
        let fallback = FileKeyStore::new(keys_dir(dir.path()));
        fallback.save(KEY_DEVICE, b"old-d").unwrap();
        fallback.save(KEY_USER, b"old-u").unwrap();
        let mem = FakeBackend::default();
        let ks = open(dir.path(), &mem);
        assert_eq!(ks.load(KEY_DEVICE).unwrap().as_deref(), Some(&b"old-d"[..]));
        assert_eq!(ks.load(KEY_USER).unwrap().as_deref(), Some(&b"old-u"[..]));
        assert_eq!(mem.get(KEY_USER).unwrap().as_deref(), Some(&b"old-u"[..]));
        assert!(!keys_dir(dir.path()).join(KEY_DEVICE).exists());
        assert!(!keys_dir(dir.path()).join(KEY_USER).exists());
        assert_eq!(marker_text(dir.path()), KEYSTORE_BACKEND_OS);
    }

    #[test]
    fn set_ok_but_readback_missing_keeps_file() {
        let dir = tempfile::tempdir().unwrap();
        FileKeyStore::new(keys_dir(dir.path()))
            .save(KEY_DEVICE, b"v")
            .unwrap();
        let mem = FakeBackend::default();
        mem.drop_writes.store(true, Ordering::Relaxed);
        let ks = open(dir.path(), &mem);
        assert_eq!(ks.load(KEY_DEVICE).unwrap().as_deref(), Some(&b"v"[..]));
        assert!(keys_dir(dir.path()).join(KEY_DEVICE).exists());
        assert_eq!(marker_text(dir.path()), KEYSTORE_BACKEND_FILE);
    }

    #[test]
    fn migration_set_failure_keeps_file_and_marks_file() {
        let dir = tempfile::tempdir().unwrap();
        FileKeyStore::new(keys_dir(dir.path()))
            .save("device.key", b"v")
            .unwrap();
        let ks = open(dir.path(), &FakeBackend::failing());
        assert_eq!(ks.load(KEY_DEVICE).unwrap().as_deref(), Some(&b"v"[..]));
        assert!(keys_dir(dir.path()).join(KEY_DEVICE).exists());
        assert_eq!(marker_text(dir.path()), KEYSTORE_BACKEND_FILE);
    }

    #[test]
    fn os_marked_backend_error_returns_err_not_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(marker(dir.path()), KEYSTORE_BACKEND_OS).unwrap();
        // 文件库里即便有同名文件也不得回退。
        FileKeyStore::new(keys_dir(dir.path()))
            .save(KEY_DEVICE, b"stale")
            .unwrap();
        let ks = open(dir.path(), &FakeBackend::failing());
        assert!(ks.load(KEY_DEVICE).is_err());
        assert!(ks.save(KEY_DEVICE, b"x").is_err());
        assert!(ks.delete(KEY_DEVICE).is_err());
        // 文件库内容未被触碰。
        assert_eq!(
            FileKeyStore::new(keys_dir(dir.path()))
                .load(KEY_DEVICE)
                .unwrap()
                .as_deref(),
            Some(&b"stale"[..])
        );
    }

    #[test]
    fn file_marked_never_touches_primary() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(marker(dir.path()), KEYSTORE_BACKEND_FILE).unwrap();
        let mem = FakeBackend::default();
        let ks = open(dir.path(), &mem);
        ks.save("k", b"v").unwrap();
        assert_eq!(ks.load("k").unwrap().as_deref(), Some(&b"v"[..]));
        assert_eq!(mem.sets.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn invalid_marker_is_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(marker(dir.path()), "bogus").unwrap();
        let ks = open(dir.path(), &FakeBackend::default());
        assert!(ks.load(KEY_DEVICE).is_err());
    }

    #[test]
    fn empty_marker_is_error_with_path_hint() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(marker(dir.path()), "").unwrap();
        let ks = open(dir.path(), &FakeBackend::default());
        let msg = ks.load(KEY_DEVICE).unwrap_err().to_string();
        assert!(
            msg.contains(&marker(dir.path()).display().to_string()),
            "{msg}"
        );
        assert!(msg.contains("可删除该文件后重试"), "{msg}");
        // 不得自动当作无标记：标记原样保留。
        assert_eq!(marker_text(dir.path()), "");
    }

    /// 两个独立实例（共享 data_dir 与假后端）同时首次 resolve。
    fn race_two_instances(legacy: Option<&[u8]>) {
        let dir = tempfile::tempdir().unwrap();
        if let Some(v) = legacy {
            FileKeyStore::new(keys_dir(dir.path()))
                .save(KEY_DEVICE, v)
                .unwrap();
        }
        let mem = FakeBackend::default();
        mem.delay_first_set.store(true, Ordering::Relaxed);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (path, mem, barrier) = (dir.path().to_path_buf(), mem.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let ks = open(&path, &mem);
                    barrier.wait();
                    ks.resolve().map(|(m, _)| m).unwrap()
                })
            })
            .collect();
        let modes: Vec<Mode> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(modes[0], modes[1]);
        assert_eq!(modes[0], Mode::Os);
        assert_eq!(marker_text(dir.path()), KEYSTORE_BACKEND_OS);
        let ks = open(dir.path(), &mem);
        match legacy {
            Some(v) => {
                assert_eq!(ks.load(KEY_DEVICE).unwrap().as_deref(), Some(v));
                assert!(!keys_dir(dir.path()).join(KEY_DEVICE).exists());
            }
            None => {
                ks.save(KEY_DEVICE, b"k").unwrap();
                assert_eq!(ks.load(KEY_DEVICE).unwrap().as_deref(), Some(&b"k"[..]));
            }
        }
        assert!(mem
            .map
            .lock()
            .unwrap()
            .keys()
            .all(|k| !k.starts_with(KEYSTORE_PROBE_NAME)));
    }

    #[test]
    fn concurrent_first_resolve_fresh_install_agrees() {
        race_two_instances(None);
    }

    #[test]
    fn concurrent_first_resolve_with_legacy_migrates_once() {
        race_two_instances(Some(b"old"));
    }

    /// 一个实例成功迁移、另一个实例的后端报错：无论先后，不丢钥且标记与位置一致。
    #[test]
    fn concurrent_migration_with_erroring_peer_never_loses_key() {
        for _ in 0..20 {
            let dir = tempfile::tempdir().unwrap();
            FileKeyStore::new(keys_dir(dir.path()))
                .save(KEY_DEVICE, b"old")
                .unwrap();
            let good = FakeBackend::default();
            // 共享同一份存储，但故障开关独立。
            let bad = FakeBackend::failing();
            let bad = FakeBackend {
                map: good.map.clone(),
                ..bad
            };
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let spawn = |b: FakeBackend| {
                let (path, barrier) = (dir.path().to_path_buf(), barrier.clone());
                std::thread::spawn(move || {
                    let ks = open(&path, &b);
                    barrier.wait();
                    ks.load(KEY_DEVICE)
                })
            };
            let (h1, h2) = (spawn(good.clone()), spawn(bad));
            let (r1, r2) = (h1.join().unwrap(), h2.join().unwrap());
            for r in [r1, r2].into_iter().flatten() {
                assert_eq!(r.as_deref(), Some(&b"old"[..]));
            }
            let file_has = keys_dir(dir.path()).join(KEY_DEVICE).exists();
            let os_has = good.get(KEY_DEVICE).unwrap().as_deref() == Some(&b"old"[..]);
            match marker_text(dir.path()).as_str() {
                KEYSTORE_BACKEND_OS => assert!(os_has),
                KEYSTORE_BACKEND_FILE => assert!(file_has && !os_has),
                other => panic!("非法标记 {other}"),
            }
        }
    }

    #[test]
    fn os_mode_cleans_stale_legacy_files_only_when_equal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(marker(dir.path()), KEYSTORE_BACKEND_OS).unwrap();
        let fb = FileKeyStore::new(keys_dir(dir.path()));
        fb.save(KEY_DEVICE, b"same").unwrap();
        fb.save(KEY_USER, b"stale").unwrap();
        let mem = FakeBackend::default();
        mem.set(KEY_DEVICE, b"same").unwrap();
        mem.set(KEY_USER, b"different").unwrap();
        open(dir.path(), &mem).load(KEY_DEVICE).unwrap();
        assert!(!keys_dir(dir.path()).join(KEY_DEVICE).exists());
        assert!(keys_dir(dir.path()).join(KEY_USER).exists());
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

    /// 在 tokio 多线程运行时内调用真实后端：不可用时应返回 Err 而非 panic/死锁。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn keyring_backend_in_tokio_multi_thread_does_not_panic() {
        let backend = KeyringBackend::new(format!("LanMsg-test-rt-{}", std::process::id()));
        let _ = backend.get("probe");
        let _ = backend.set("probe", b"x");
        let _ = backend.remove("probe");
    }

    /// 同上，current_thread 运行时。
    #[tokio::test]
    async fn keyring_backend_in_tokio_current_thread_does_not_panic() {
        let backend = KeyringBackend::new(format!("LanMsg-test-rt1-{}", std::process::id()));
        let _ = backend.get("probe");
    }
}
