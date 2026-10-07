//! 结果落点：结果目录占名、单一顶层项提升与平台元数据剔除。

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use log::warn;

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

/// 结果里唯一的条目；不是恰好一个时返回 `None`。
fn only_entry(directory: &Path) -> Option<fs::DirEntry> {
    let mut entries = match collect_entries(directory) {
        Ok(entries) => entries,
        Err(message) => {
            warn!("could not inspect the result to promote a single entry: {message}");
            return None;
        }
    };
    match entries.len() {
        1 => entries.pop(),
        _ => None,
    }
}

/// 结果里只剩一个条目时，把它提升到归档所在目录（用条目自己的名字），返回提升后的路径。
///
/// 引擎原样写出归档的顶层结构，所以“唯一子项”就是归档的单一顶层项；冲突命名与 §5.3 一致
/// （目录 `name (n)`、文件 `name (n).ext`），绝不覆盖既有条目。
///
/// 返回 `None` 时调用方保留 `<归档名>/…` 形态——那仍是一个完整结果，不需要报错。
pub(super) fn promote_single_entry(output: &Path) -> Option<PathBuf> {
    let parent = output.parent()?;
    let entry = only_entry(output)?;
    let directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
    let name = entry.file_name();

    let (wrapper, target) = match vacant_name(parent, output, &name, directory) {
        Ok(chosen) => chosen,
        Err(message) => {
            warn!("could not promote the single result entry: {message}");
            return None;
        }
    };
    // 占名目录可能已经被挪到临时名下，源路径按它现在的位置算。
    let source = wrapper.join(&name);

    if let Err(error) = fs::rename(&source, &target) {
        warn!(
            "could not promote {} to {}: {error}",
            source.display(),
            target.display()
        );
        // 让位用的临时名要还原，别把结果留在临时名下。
        if wrapper != output {
            let _ = fs::rename(&wrapper, output);
        }
        return None;
    }

    // 已经空了的占名目录不该留在结果旁边。
    if let Err(error) = fs::remove_dir(&wrapper) {
        warn!("could not remove {}: {error}", wrapper.display());
    }
    Some(target)
}

/// 取一个空闲的目标名，返回（占名目录现在的位置、目标路径）。
///
/// 这个名字正被占名目录自己占着时（`bundle.zip` 内含单一目录 `bundle/`，或本次撞名退让成
/// `bundle (1)`），先把占名目录挪成临时名让位——它就在目标名的位置上。
fn vacant_name(
    parent: &Path,
    output: &Path,
    name: &OsStr,
    directory: bool,
) -> Result<(PathBuf, PathBuf), String> {
    for candidate in promoted_candidates(parent, name, directory) {
        if !candidate.exists() {
            return Ok((output.to_path_buf(), candidate));
        }
        if !same_name(
            candidate.file_name().unwrap_or_default(),
            output.file_name().unwrap_or_default(),
        ) {
            continue;
        }
        let scratch =
            vacant_scratch_path(parent).ok_or_else(|| "no free temporary name".to_owned())?;
        fs::rename(output, &scratch)
            .map_err(|error| format!("could not move {} aside: {error}", output.display()))?;
        return Ok((scratch, candidate));
    }
    Err("no free result name".to_owned())
}

/// 目标名候选：目录是 `name`, `name (1)`, …；文件是 `name.ext`, `name (1).ext`, …。
fn promoted_candidates<'a>(
    parent: &'a Path,
    name: &'a OsStr,
    directory: bool,
) -> impl Iterator<Item = PathBuf> + 'a {
    let path = Path::new(name);
    let (stem, extension) = match (directory, path.file_stem(), path.extension()) {
        (false, Some(stem), Some(extension)) => {
            (stem.to_os_string(), Some(extension.to_os_string()))
        }
        _ => (name.to_os_string(), None),
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

/// 让位用的临时名：只活几毫秒，递增后缀就够（用户恰好占着同名就换下一个）。
fn vacant_scratch_path(parent: &Path) -> Option<PathBuf> {
    (0..100)
        .map(|sequence| parent.join(format!(".ezz-move-{sequence}")))
        .find(|candidate| !candidate.exists())
}

/// 先把条目读完再动手：一边遍历目录一边把条目搬出去会漏掉后面的条目。
fn collect_entries(directory: &Path) -> Result<Vec<fs::DirEntry>, String> {
    fs::read_dir(directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("{}: {error}", directory.display()))
}

/// 名字比较：Windows 文件系统不区分大小写，`BUNDLE` 与 `bundle` 是同一个位置。
fn same_name(left: &OsStr, right: &OsStr) -> bool {
    left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
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

    #[test]
    fn a_single_entry_becomes_the_result() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let output = sandbox.path().join("bundle");
        std::fs::create_dir_all(output.join("inner")).expect("create inner directory");
        std::fs::write(output.join("inner/payload.txt"), b"payload").expect("write payload");

        let promoted = promote_single_entry(&output).expect("promote the single entry");

        assert_eq!(promoted, sandbox.path().join("inner"));
        assert!(promoted.join("payload.txt").is_file());
        assert!(!output.exists());
    }

    /// `bundle.zip` 内含单一目录 `bundle/`：目标名正被占名目录自己占着，要让位。
    #[test]
    fn a_single_entry_wins_over_the_claimed_directory_name() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let output = sandbox.path().join("bundle");
        std::fs::create_dir_all(output.join("bundle/nested")).expect("create nested directory");
        std::fs::write(output.join("bundle/nested/payload.txt"), b"payload")
            .expect("write payload");

        let promoted = promote_single_entry(&output).expect("promote the single entry");

        assert_eq!(promoted, sandbox.path().join("bundle"));
        assert!(promoted.join("nested/payload.txt").is_file());
    }

    #[test]
    fn a_single_file_becomes_the_result_without_losing_its_extension() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let output = sandbox.path().join("bundle");
        std::fs::create_dir(&output).expect("create result directory");
        std::fs::write(output.join("payload.txt"), b"payload").expect("write payload");

        let promoted = promote_single_entry(&output).expect("promote the single entry");

        assert_eq!(promoted, sandbox.path().join("payload.txt"));
        assert!(promoted.is_file());
    }

    #[test]
    fn existing_names_push_the_result_to_the_next_candidate() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        std::fs::create_dir(sandbox.path().join("inner")).expect("create existing directory");
        let output = sandbox.path().join("bundle");
        std::fs::create_dir_all(output.join("inner")).expect("create inner directory");

        assert_eq!(
            promote_single_entry(&output),
            Some(sandbox.path().join("inner (1)"))
        );

        let file = tempfile::tempdir().expect("create test sandbox");
        std::fs::write(file.path().join("payload.txt"), b"existing").expect("write existing");
        let output = file.path().join("bundle");
        std::fs::create_dir(&output).expect("create result directory");
        std::fs::write(output.join("payload.txt"), b"payload").expect("write payload");

        assert_eq!(
            promote_single_entry(&output),
            Some(file.path().join("payload (1).txt"))
        );
        assert_eq!(
            std::fs::read_to_string(file.path().join("payload.txt")).expect("read existing"),
            "existing"
        );
    }

    #[test]
    fn nothing_is_promoted_without_exactly_one_entry() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");

        let empty = sandbox.path().join("empty");
        std::fs::create_dir(&empty).expect("create empty result");
        assert_eq!(promote_single_entry(&empty), None);
        assert!(empty.is_dir());

        let two = sandbox.path().join("two");
        std::fs::create_dir(&two).expect("create result with two entries");
        std::fs::write(two.join("first.txt"), b"1").expect("write first");
        std::fs::write(two.join("second.txt"), b"2").expect("write second");
        assert_eq!(promote_single_entry(&two), None);
        assert!(two.join("first.txt").is_file());
        assert!(two.join("second.txt").is_file());
    }
}
