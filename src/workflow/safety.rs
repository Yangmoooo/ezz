//! 路径与文件安全：丢弃无法在最终位置上成立的条目。
//!
//! 逃逸或无法解析的链接、指向结果外的链接、设备/FIFO/socket 这类特殊文件都不进入提交集合。

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{ExtractionError, file_system_error};

/// 丢弃不安全的条目，返回被删除条目的相对路径。
///
/// 不安全条目不会让整个输入失败，调用方把它们记入结构化警告。
///
/// - 符号链接：解析不到目标、解析后离开工作目录、或目标是绝对路径 → 删除；
/// - 特殊文件（设备、FIFO、socket 等）→ 删除。
pub(super) fn discard_unsafe_entries(root: &Path) -> Result<Vec<PathBuf>, ExtractionError> {
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| file_system_error("resolve extraction directory", root, error))?;
    let mut directories = vec![root.to_path_buf()];
    let mut discarded = Vec::new();

    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| file_system_error("inspect extracted directory", &directory, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                file_system_error("inspect extracted entry in", &directory, error)
            })?;
            let path = entry.path();
            // 用 `DirEntry::file_type` 而不是 `fs::symlink_metadata`：前者直接来自 `read_dir`
            // 已拿到的属性（Unix 上通常来自 `d_type`），不额外发起系统调用，且不跟随链接。
            let file_type = entry
                .file_type()
                .map_err(|error| file_system_error("inspect extracted entry", &path, error))?;

            if file_type.is_symlink() {
                let target = fs::read_link(&path)
                    .map_err(|error| file_system_error("read symbolic link", &path, error))?;
                let escapes = target.is_absolute()
                    || match fs::canonicalize(&path) {
                        Ok(resolved) => !resolved.starts_with(&canonical_root),
                        // 解析不到目标（死链）：可能是 `../..` 拼出来的逃逸，一律不信。
                        Err(_) => true,
                    };
                if escapes {
                    remove_symbolic_link(&path)?;
                    discarded.push(relative_to(root, &path));
                }
            } else if file_type.is_dir() {
                directories.push(path);
            } else if !file_type.is_file() {
                fs::remove_file(&path)
                    .map_err(|error| file_system_error("remove special file", &path, error))?;
                discarded.push(relative_to(root, &path));
            }
        }
    }

    Ok(discarded)
}

/// 删除符号链接：Windows 上目录链接必须用 `remove_dir`。
fn remove_symbolic_link(path: &Path) -> Result<(), ExtractionError> {
    if fs::remove_file(path).is_ok() {
        return Ok(());
    }
    fs::remove_dir(path).map_err(|error| file_system_error("remove symbolic link", path, error))
}

fn relative_to(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// 归档所在目录的条目快照：名称 → 修改时间。
pub(super) type DirectorySnapshot = Vec<(OsString, Option<SystemTime>)>;

pub(super) fn directory_snapshot(
    directory: &Path,
    ignore: &Path,
) -> Result<DirectorySnapshot, ExtractionError> {
    let ignored_name = ignore.file_name();
    let mut snapshot = Vec::new();
    let entries = fs::read_dir(directory)
        .map_err(|error| file_system_error("snapshot directory", directory, error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| file_system_error("snapshot entry in", directory, error))?;
        if Some(entry.file_name().as_os_str()) == ignored_name {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        snapshot.push((entry.file_name(), modified));
    }
    snapshot.sort();
    Ok(snapshot)
}

/// 解压不得在归档所在目录留下任何新增或改动。
///
/// 条目路径是否绝对或可能逃逸：绝对路径、盘符前缀，或含 `..` 段。
///
/// 用于报告：7-Zip 会把这类条目重写进工作目录。
pub(crate) fn is_unsafe_archive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.is_empty()
        || path.starts_with(['/', '\\'])
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || path.split(['/', '\\']).any(|component| component == "..")
}

/// 内嵌归档的路径是否是一个安全的相对路径（全部是普通段）。
pub(crate) fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

/// 发现差异即致命失败：不提交，也不清理原归档。
pub(super) fn validate_escape_invariant(
    directory: &Path,
    ignore: &Path,
    before: &DirectorySnapshot,
) -> Result<(), ExtractionError> {
    let after = directory_snapshot(directory, ignore)?;
    if &after == before {
        return Ok(());
    }

    Err(ExtractionError::UnsafeOutput {
        path: directory.to_path_buf(),
        reason: "extraction changed entries outside its workspace".to_owned(),
    })
}

/// 单元测试：这些用例只碰本模块的纯逻辑。
#[cfg(test)]
mod tests {
    use super::*;

    /// 工作目录内的改动不算逃逸，归档所在目录的新条目算。
    #[test]
    fn escape_invariant_detects_changes_outside_the_workspace() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let workspace = sandbox.path().join(".ezz-work-test");
        std::fs::create_dir(&workspace).expect("create workspace");

        let snapshot = directory_snapshot(sandbox.path(), &workspace).expect("snapshot");
        assert!(validate_escape_invariant(sandbox.path(), &workspace, &snapshot).is_ok());

        std::fs::write(workspace.join("file.txt"), b"inside").expect("write inside workspace");
        assert!(
            validate_escape_invariant(sandbox.path(), &workspace, &snapshot).is_ok(),
            "changes inside the workspace are not an escape"
        );

        std::fs::write(sandbox.path().join("escaped.txt"), b"outside").expect("write outside");
        assert!(
            matches!(
                validate_escape_invariant(sandbox.path(), &workspace, &snapshot),
                Err(ExtractionError::UnsafeOutput { .. })
            ),
            "a new entry outside the workspace must break the invariant"
        );
    }
    /// 在不支持符号链接的环境（例如没有权限的 CI）上返回 false，由调用方跳过。
    fn try_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, link).is_ok()
        }
    }
    #[test]
    fn unsafe_symbolic_links_are_discarded_while_safe_ones_are_kept() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let root = sandbox.path().join("extracted");
        std::fs::create_dir_all(root.join("nested")).expect("create root");
        std::fs::write(root.join("target.txt"), b"target").expect("write target");
        std::fs::write(root.join("nested/keep.txt"), b"keep").expect("write kept file");

        let escaping = try_symlink(Path::new("../outside.txt"), &root.join("escaping"));
        let dangling = try_symlink(Path::new("missing.txt"), &root.join("dangling"));
        let absolute = try_symlink(&root.join("target.txt"), &root.join("absolute"));
        let inside = try_symlink(Path::new("target.txt"), &root.join("inside"));
        if !(escaping || dangling || absolute || inside) {
            eprintln!("skipping: this environment cannot create symbolic links");
            return;
        }

        let discarded = discard_unsafe_entries(&root).expect("discard unsafe entries");
        let names: Vec<String> = discarded
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();

        if escaping {
            assert!(names.iter().any(|name| name == "escaping"), "{names:?}");
        }
        if dangling {
            assert!(names.iter().any(|name| name == "dangling"), "{names:?}");
        }
        if absolute {
            assert!(names.iter().any(|name| name == "absolute"), "{names:?}");
        }
        if inside {
            assert!(!names.iter().any(|name| name == "inside"), "{names:?}");
        }
        assert!(
            !names.iter().any(|name| name.contains("keep.txt")),
            "{names:?}"
        );
        assert!(
            root.join("nested/keep.txt").is_file(),
            "safe content must be kept"
        );
        assert!(
            discarded.iter().all(|path| path.is_relative()),
            "reported entries should be relative to the extraction root"
        );
    }
}
