// 缩略图缓存容量控制（设计 §9.5）。

use std::path::Path;
use std::time::SystemTime;

/// 把 `dir` 下的缓存文件总大小压到 `limit` 以内，返回删除的字节数。
///
/// 只统计 `dir` 下的普通文件（不递归，跳过 `tmp-` 前缀的生成中临时文件）；
/// 超限时按访问时间（取不到则修改时间）最旧优先删除。`dir` 不存在返回 `Ok(0)`。
pub fn enforce_thumb_cache_limit(dir: &Path, limit: u64) -> std::io::Result<u64> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut files: Vec<(SystemTime, u64, std::path::PathBuf)> = Vec::new();
    let mut total: u64 = 0;
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("tmp-") {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let when = meta
            .accessed()
            .or_else(|_| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        total += meta.len();
        files.push((when, meta.len(), entry.path()));
    }
    if total <= limit {
        return Ok(0);
    }
    files.sort_by_key(|(when, _, _)| *when);
    let mut freed: u64 = 0;
    for (_, size, path) in files {
        if total <= limit {
            break;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                total -= size;
                freed += size;
            }
            // 已被并发删除：视同已释放，不计入返回值。
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => total -= size,
            Err(e) => return Err(e),
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, FileTimes};
    use std::time::{Duration, SystemTime};

    /// 写一个指定大小、指定访问/修改时间（距今 age 秒前）的文件。
    fn write_aged(dir: &std::path::Path, name: &str, size: usize, age_secs: u64) {
        let f = File::create(dir.join(name)).unwrap();
        f.set_len(size as u64).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_secs);
        f.set_times(FileTimes::new().set_accessed(t).set_modified(t))
            .unwrap();
    }

    #[test]
    fn enforce_limit_deletes_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        write_aged(dir.path(), "a", 100, 300);
        write_aged(dir.path(), "b", 100, 200);
        write_aged(dir.path(), "c", 100, 100);
        let freed = enforce_thumb_cache_limit(dir.path(), 250).unwrap();
        assert_eq!(freed, 100);
        assert!(!dir.path().join("a").exists());
        assert!(dir.path().join("b").exists());
        assert!(dir.path().join("c").exists());
    }

    #[test]
    fn enforce_limit_noop_under_limit_or_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_aged(dir.path(), "a", 100, 30);
        write_aged(dir.path(), "b", 100, 20);
        write_aged(dir.path(), "c", 100, 10);
        assert_eq!(enforce_thumb_cache_limit(dir.path(), 1000).unwrap(), 0);
        assert!(dir.path().join("a").exists());
        let missing = dir.path().join("nope");
        assert_eq!(enforce_thumb_cache_limit(&missing, 10).unwrap(), 0);
    }

    #[test]
    fn enforce_limit_ignores_tmp_files_and_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        write_aged(dir.path(), "tmp-1.jpg", 500, 500);
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        write_aged(dir.path(), "a", 100, 10);
        assert_eq!(enforce_thumb_cache_limit(dir.path(), 100).unwrap(), 0);
        assert!(dir.path().join("tmp-1.jpg").exists());
    }
}
