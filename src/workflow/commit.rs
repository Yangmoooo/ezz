//! 事务式提交：命名冲突、平台元数据剔除、空结果。
//!
//! 工作目录里有什么由 7-Zip 决定，结果落到哪里、叫什么名字由这里决定。

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use super::{ExtractionError, file_system_error};

/// 提交结果。
pub(super) struct Committed {
    pub(super) path: PathBuf,
    /// 被剔除的平台元数据条目名。
    pub(super) removed_metadata: Vec<String>,
    /// 剔除后没有任何有效内容：提交的是一个空目录。
    pub(super) empty: bool,
}

pub(super) fn commit_output(
    input: &Path,
    extracted: &Path,
    output_stem: &OsStr,
) -> Result<Committed, ExtractionError> {
    let removed_metadata = remove_platform_metadata(extracted)?;
    let mut entries = fs::read_dir(extracted)
        .map_err(|error| file_system_error("read extracted contents from", extracted, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| file_system_error("read extracted entry from", extracted, error))?;

    let parent = input.parent().expect("validated input parent");
    match entries.len() {
        // 输出为空是事实而不是错误：提交一个以归档命名的空目录，让结果仍有一个最终实际路径。
        0 => {
            let path = commit_empty_directory(parent, output_stem)?;
            Ok(Committed {
                path,
                removed_metadata,
                empty: true,
            })
        }
        1 => {
            let entry = entries.pop().expect("one extracted entry");
            // 目录用 `name (1)`，文件用 `name (1).ext`。
            let is_directory = entry
                .file_type()
                .map_err(|error| {
                    file_system_error("inspect extracted entry", &entry.path(), error)
                })?
                .is_dir();
            let kind = if is_directory {
                CommitKind::Directory
            } else {
                CommitKind::File
            };
            let path = commit_with_unique_name(&entry.path(), parent, &entry.file_name(), kind)?;
            Ok(Committed {
                path,
                removed_metadata,
                empty: false,
            })
        }
        _ => {
            let path =
                commit_with_unique_name(extracted, parent, output_stem, CommitKind::Directory)?;
            Ok(Committed {
                path,
                removed_metadata,
                empty: false,
            })
        }
    }
}

/// 提交一个空目录：直接把名字占下来。
fn commit_empty_directory(parent: &Path, name: &OsStr) -> Result<PathBuf, ExtractionError> {
    for candidate in unique_destination_candidates(parent, name, CommitKind::Directory) {
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(file_system_error(
                    "create empty result directory",
                    &candidate,
                    error,
                ));
            }
        }
    }

    unreachable!("u64 destination sequence exhausted")
}

/// 剔除平台元数据，返回被剔除的条目名。
fn remove_platform_metadata(extracted: &Path) -> Result<Vec<String>, ExtractionError> {
    let mut removed = Vec::new();
    for name in ["__MACOSX", ".DS_Store"] {
        let path = extracted.join(name);
        if path.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|error| file_system_error("remove platform metadata", &path, error))?;
            removed.push(name.to_owned());
        } else if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| file_system_error("remove platform metadata", &path, error))?;
            removed.push(name.to_owned());
        }
    }
    Ok(removed)
}

#[derive(Clone, Copy)]
enum CommitKind {
    File,
    Directory,
}

/// 把 `source` 提交为 `parent` 下的一个不冲突名字。
///
/// 候选名字按 `name`, `name (1)`, `name (2)` … 递增，**绝不覆盖既有条目**；名字在探测与
/// 重命名之间被占用时继续递增重试。
fn commit_with_unique_name(
    source: &Path,
    parent: &Path,
    name: &OsStr,
    kind: CommitKind,
) -> Result<PathBuf, ExtractionError> {
    for candidate in unique_destination_candidates(parent, name, kind) {
        if candidate.exists() {
            continue;
        }

        match fs::rename(source, &candidate) {
            Ok(()) => return Ok(candidate),
            // 竞态：候选名字在探测之后被占用。换下一个序号。
            Err(_) if candidate.exists() => continue,
            Err(error) => {
                return Err(file_system_error(
                    "commit extracted output to",
                    &candidate,
                    error,
                ));
            }
        }
    }

    unreachable!("u64 destination sequence exhausted")
}

/// 生成候选目的地：文件保留扩展名（`archive (1).zip`），目录整体递增（`archive.zip (1)`）。
fn unique_destination_candidates<'a>(
    parent: &'a Path,
    name: &'a OsStr,
    kind: CommitKind,
) -> impl Iterator<Item = PathBuf> + 'a {
    let (stem, extension) = match kind {
        CommitKind::File => {
            let name_path = Path::new(name);
            let stem = name_path.file_stem().unwrap_or(name).to_os_string();
            let extension = name_path.extension().map(OsString::from);
            (stem, extension)
        }
        CommitKind::Directory => (name.to_os_string(), None),
    };

    std::iter::once(parent.join(name)).chain((1_u64..).map(move |sequence| {
        let mut candidate = stem.clone();
        candidate.push(format!(" ({sequence})"));
        if let Some(extension) = &extension {
            candidate.push(".");
            candidate.push(extension);
        }
        parent.join(candidate)
    }))
}

/// 单元测试：这些用例只碰本模块的纯逻辑。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_names_follow_the_conflict_rules() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let parent = sandbox.path();

        let files: Vec<String> =
            unique_destination_candidates(parent, OsStr::new("archive.tar.gz"), CommitKind::File)
                .take(3)
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
        assert_eq!(
            files,
            ["archive.tar.gz", "archive.tar (1).gz", "archive.tar (2).gz"]
        );

        // 目录整体递增：不得把最后一个“扩展名”拆开。
        let directories: Vec<String> =
            unique_destination_candidates(parent, OsStr::new("archive.tar"), CommitKind::Directory)
                .take(3)
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
        assert_eq!(
            directories,
            ["archive.tar", "archive.tar (1)", "archive.tar (2)"]
        );
    }
    #[test]
    fn empty_result_never_overwrites_an_existing_entry() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        std::fs::write(sandbox.path().join("payload"), b"existing").expect("create existing file");

        let created =
            commit_empty_directory(sandbox.path(), OsStr::new("payload")).expect("commit empty");

        assert_eq!(
            created.file_name().unwrap().to_string_lossy(),
            "payload (1)"
        );
        assert!(created.is_dir());
        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("payload")).expect("read existing"),
            "existing"
        );
    }
}
