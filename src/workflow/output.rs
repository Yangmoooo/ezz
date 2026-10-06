//! 结果落点：唯一目录名与平台元数据剔除。

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use super::{ExtractionError, file_system_error};

/// 占下一个不冲突的结果目录，返回它的路径。
///
/// 候选名字按 `name`, `name (1)`, `name (2)` … 递增，**绝不覆盖既有条目**；目录在这里就被
/// 创建，所以后续的解压不会撞上别人的名字。
pub(super) fn claim_output_directory(
    parent: &Path,
    name: &OsStr,
) -> Result<PathBuf, ExtractionError> {
    for candidate in unique_destination_candidates(parent, name) {
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(file_system_error(
                    "create result directory",
                    &candidate,
                    error,
                ));
            }
        }
    }

    unreachable!("u64 destination sequence exhausted")
}

fn unique_destination_candidates<'a>(
    parent: &'a Path,
    name: &'a OsStr,
) -> impl Iterator<Item = PathBuf> + 'a {
    std::iter::once(parent.join(name)).chain((1_u64..).map(move |sequence| {
        let mut candidate = name.to_os_string();
        candidate.push(format!(" ({sequence})"));
        parent.join(candidate)
    }))
}

/// 递归剔除结果里的平台元数据（`__MACOSX` 目录、`.DS_Store` 文件），返回剔除的条目数。
///
/// 这是 ezz 主动做的结果清理，不是 7-Zip 的消毒行为；只碰结果目录内部。
pub(super) fn remove_platform_metadata(root: &Path) -> Result<usize, ExtractionError> {
    let mut removed = 0;
    let mut directories = vec![root.to_path_buf()];

    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| file_system_error("inspect result entry in", &directory, error))?;
        for entry in entries {
            let entry = entry
                .map_err(|error| file_system_error("inspect result entry in", &directory, error))?;
            let path = entry.path();
            let file_name = entry.file_name();
            let file_type = entry
                .file_type()
                .map_err(|error| file_system_error("inspect result entry", &path, error))?;

            if file_name == "__MACOSX" || file_name == ".DS_Store" {
                if file_type.is_dir() {
                    fs::remove_dir_all(&path).map_err(|error| {
                        file_system_error("remove platform metadata", &path, error)
                    })?;
                } else {
                    fs::remove_file(&path).map_err(|error| {
                        file_system_error("remove platform metadata", &path, error)
                    })?;
                }
                removed += 1;
            } else if file_type.is_dir() {
                directories.push(path);
            }
        }
    }

    Ok(removed)
}

/// 结果目录里是否一个条目都没有。
pub(super) fn is_empty(directory: &Path) -> Result<bool, ExtractionError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| file_system_error("inspect result directory", directory, error))?;
    Ok(entries.next().is_none())
}

/// 单元测试：这些用例只碰本模块的纯逻辑。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_names_grow_without_touching_the_extension() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let names: Vec<String> =
            unique_destination_candidates(sandbox.path(), OsStr::new("archive.tar.gz"))
                .take(3)
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
        assert_eq!(
            names,
            ["archive.tar.gz", "archive.tar.gz (1)", "archive.tar.gz (2)"]
        );
    }

    #[test]
    fn claiming_never_overwrites_an_existing_entry() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        std::fs::write(sandbox.path().join("payload"), b"existing").expect("create existing file");

        let claimed =
            claim_output_directory(sandbox.path(), OsStr::new("payload")).expect("claim directory");

        assert_eq!(
            claimed.file_name().unwrap().to_string_lossy(),
            "payload (1)"
        );
        assert!(claimed.is_dir());
        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("payload")).expect("read existing"),
            "existing"
        );
    }

    #[test]
    fn platform_metadata_is_removed_at_any_depth() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let nested = sandbox.path().join("game/data");
        std::fs::create_dir_all(nested.join("__MACOSX")).expect("create __MACOSX");
        std::fs::write(nested.join("__MACOSX/._save"), b"metadata").expect("write metadata");
        std::fs::write(nested.join(".DS_Store"), b"metadata").expect("write .DS_Store");
        std::fs::write(nested.join(".localized"), b"kept").expect("write kept file");
        // `._foo` 兄弟文件不在剔除范围内，保持原样。
        std::fs::write(nested.join("._save"), b"kept").expect("write sibling");

        let removed = remove_platform_metadata(sandbox.path()).expect("remove metadata");

        assert_eq!(removed, 2);
        assert!(!nested.join("__MACOSX").exists());
        assert!(!nested.join(".DS_Store").exists());
        assert!(nested.join(".localized").is_file());
        assert!(nested.join("._save").is_file());
    }
}
