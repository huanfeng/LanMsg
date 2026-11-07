// P1 核心骨架：密钥库（C6）。
// KeyStore trait 与 FileKeyStore 实现，密钥按文件名持久化到本地目录。
// 注释中文，标识符英文。

use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::error::KeyStoreError;

/// 密钥库抽象：按名称加载/保存/删除密钥文件（C6：要求 Send + Sync）。
pub trait KeyStore: Send + Sync {
    /// 加载名为 `name` 的密钥文件，不存在返回 Ok(None)。
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError>;
    /// 保存密钥到名为 `name` 的文件。
    fn save(&self, name: &str, data: &[u8]) -> Result<(), KeyStoreError>;
    /// 删除名为 `name` 的密钥文件。
    fn delete(&self, name: &str) -> Result<(), KeyStoreError>;
}

/// 原子写文件：同目录临时文件写入并 fsync，rename 覆盖目标，Unix 上再 fsync 父目录。
///
/// `mode_0600` 为真时（仅 Unix 生效）创建即指定 0600。任一步失败都会清理临时文件。
pub(crate) fn atomic_write(path: &Path, bytes: &[u8], mode_0600: bool) -> io::Result<()> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let tmp_path = path.with_file_name(format!(
        "{name}.tmp.{}.{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    if mode_0600 {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = mode_0600;
    let result = (|| {
        let mut f = opts.open(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp_path, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    // 持久化 rename 本身；部分平台/文件系统不支持打开目录，失败只忽略（数据已落盘）。
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

/// 基于文件系统的密钥库实现。
///
/// 文件名即 `name`，Unix 下文件权限 0600、目录权限 0700。
/// `save` 经 `atomic_write` 写临时文件、fsync 后原子 rename。
pub struct FileKeyStore {
    dir: PathBuf,
}

impl FileKeyStore {
    /// 创建指向 `dir` 的文件密钥库，不存在则创建目录（0700）。
    ///
    /// 目录创建失败的错误由后续 load/save/delete 暴露，此处保持构造函数不返回 Result。
    pub fn new(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        #[cfg(unix)]
        {
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        }
        Self { dir }
    }

    fn file_path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl KeyStore for FileKeyStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        let path = self.file_path(name);
        match fs::read(&path) {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(KeyStoreError::Io(e)),
        }
    }

    fn save(&self, name: &str, data: &[u8]) -> Result<(), KeyStoreError> {
        atomic_write(&self.file_path(name), data, true)?;
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), KeyStoreError> {
        let path = self.file_path(name);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(KeyStoreError::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m");
        atomic_write(&p, b"one", false).unwrap();
        atomic_write(&p, b"two", true).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn file_keystore_save_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        let ks = FileKeyStore::new(dir.path().join("keys"));
        assert_eq!(ks.load("a").unwrap(), None);
        ks.save("a", b"xyz").unwrap();
        assert_eq!(ks.load("a").unwrap().as_deref(), Some(&b"xyz"[..]));
        ks.delete("a").unwrap();
        assert_eq!(ks.load("a").unwrap(), None);
    }
}
