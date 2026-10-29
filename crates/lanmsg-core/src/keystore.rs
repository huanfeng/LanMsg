// P1 核心骨架：密钥库（C6）。
// KeyStore trait 与 FileKeyStore 实现，密钥按文件名持久化到本地目录。
// 注释中文，标识符英文。

use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use crate::error::KeyStoreError;

/// 密钥库抽象：按名称加载/保存/删除密钥文件。
pub trait KeyStore {
    /// 加载名为 `name` 的密钥文件，不存在返回 Ok(None)。
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, KeyStoreError>;
    /// 保存密钥到名为 `name` 的文件。
    fn save(&self, name: &str, data: &[u8]) -> Result<(), KeyStoreError>;
    /// 删除名为 `name` 的密钥文件。
    fn delete(&self, name: &str) -> Result<(), KeyStoreError>;
}

/// 基于文件系统的密钥库实现。
///
/// 文件名即 `name`，Unix 下文件权限 0600、目录权限 0700。
/// `save` 先写 `name.tmp` 再原子 rename 到 `name`。
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
        let path = self.file_path(name);
        let tmp_path = self.dir.join(format!("{name}.tmp"));
        fs::write(&tmp_path, data)?;
        #[cfg(unix)]
        {
            let _ = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&tmp_path, &path)?;
        #[cfg(unix)]
        {
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
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
