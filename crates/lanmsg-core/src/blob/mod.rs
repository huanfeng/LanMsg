// P2 Task 11：Blob 发送——MIME 映射、文件哈希、准备流程与数据服务端。
// 发送侧辅助（收集待发文件与文件夹遍历）在本模块，哈希在 hash、
// 服务端在 serve；msg.rs 的 send_files/send_folder 调用本模块函数。
// 注释中文，标识符英文。

pub mod hash;
pub mod mime;
pub mod serve;

use std::path::{Path, PathBuf};

use lanmsg_proto::consts::MAX_FOLDER_ENTRIES;

use crate::error::CoreError;

/// 待发送的单个文件（阻塞收集得到，路径为绝对路径）。
#[derive(Debug, Clone)]
pub struct FileSource {
    pub path: PathBuf,
    /// 文件夹内相对路径（'/' 分隔）；Files 消息为 None。
    pub rel_path: Option<String>,
    /// 文件名（不含目录）。
    pub name: String,
    pub size: u64,
}

/// 收集 Files 消息的待发文件：列表为空或任一路径不存在/不是普通文件 →
/// InvalidArgument。阻塞函数，调用方需在 spawn_blocking 中执行。
pub fn collect_file_sources(paths: &[PathBuf]) -> Result<Vec<FileSource>, CoreError> {
    if paths.is_empty() {
        return Err(CoreError::InvalidArgument("文件列表为空".into()));
    }
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        out.push(file_source(p, None)?);
    }
    Ok(out)
}

/// 收集 Folder 消息的待发文件：根不是目录 → InvalidArgument；遍历（跳过
/// 符号链接）文件数超过 `MAX_FOLDER_ENTRIES` → InvalidArgument。阻塞函数，
/// 调用方需在 spawn_blocking 中执行。返回 (文件夹名, 条目)。
pub fn collect_folder_sources(root: &Path) -> Result<(String, Vec<FileSource>), CoreError> {
    if !root.is_dir() {
        return Err(CoreError::InvalidArgument("文件夹根路径不是目录".into()));
    }
    let entries = walk_folder(root, MAX_FOLDER_ENTRIES)?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    Ok((name, entries))
}

/// 遍历文件夹（跳过符号链接，rel_path 用 '/' 分隔，按目录项名字排序保证
/// 稳定顺序），文件数超过 `max_entries` → InvalidArgument。阻塞函数。
pub fn walk_folder(root: &Path, max_entries: usize) -> Result<Vec<FileSource>, CoreError> {
    let mut out = Vec::new();
    walk_dir(root, Path::new(""), max_entries, &mut out)?;
    Ok(out)
}

fn walk_dir(
    dir: &Path,
    rel: &Path,
    max_entries: usize,
    out: &mut Vec<FileSource>,
) -> Result<(), CoreError> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(CoreError::Io)?
        .collect::<Result<_, _>>()
        .map_err(CoreError::Io)?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let ft = entry.file_type().map_err(CoreError::Io)?;
        // 符号链接一律跳过。
        if ft.is_symlink() {
            continue;
        }
        let child_rel = rel.join(entry.file_name());
        if ft.is_dir() {
            walk_dir(&entry.path(), &child_rel, max_entries, out)?;
        } else if ft.is_file() {
            if out.len() >= max_entries {
                return Err(CoreError::InvalidArgument(format!(
                    "文件夹文件数超过 {max_entries}"
                )));
            }
            out.push(file_source(&entry.path(), Some(rel_to_string(&child_rel)))?);
        }
    }
    Ok(())
}

/// 相对路径转 '/' 分隔字符串。
fn rel_to_string(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

/// 单个文件的元信息收集；不存在或不是普通文件 → InvalidArgument。
fn file_source(path: &Path, rel_path: Option<String>) -> Result<FileSource, CoreError> {
    let meta = std::fs::metadata(path)
        .map_err(|_| CoreError::InvalidArgument(format!("路径不存在: {}", path.display())))?;
    if !meta.is_file() {
        return Err(CoreError::InvalidArgument(format!(
            "不是普通文件: {}",
            path.display()
        )));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(FileSource {
        path: path.to_path_buf(),
        rel_path,
        name,
        size: meta.len(),
    })
}
