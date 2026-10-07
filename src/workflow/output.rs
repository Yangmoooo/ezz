//! 结果落点：唯一目录名与平台元数据剔除。

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

/// 补上 `-spe` 因结果目录改名而失效的那层重复根目录。
///
/// `-spe` 只在结果目录名与归档内部根目录名相同时生效；目标名被占用而退让成 `name (1)` 时
/// 条件不再成立，结果里就多一层。这里只在这种形态上动手：恰好一个子项、它是目录、且名字
/// 与归档名相同（与 `-spe` 一样忽略大小写）。
///
/// 失败只记警告：这是树形补齐，不值得让整次解压失败；搬了一半就停时，还没搬的条目留在原位置。
pub(super) fn hoist_duplicate_root(output: &Path, archive_stem: &OsStr) {
    let mut entries = match collect_entries(output) {
        Ok(entries) => entries,
        Err(message) => {
            warn!("could not inspect the result to hoist a duplicate root: {message}");
            return;
        }
    };
    if entries.len() != 1 {
        return;
    }
    let inner = entries.pop().expect("one entry");
    let is_directory = inner.file_type().is_ok_and(|kind| kind.is_dir());
    if !is_directory || !same_name(&inner.file_name(), archive_stem) {
        return;
    }
    if let Err(message) = hoist_contents(&inner.path(), output) {
        warn!("could not hoist the duplicate root in the result: {message}");
    }
}

/// 把 `inner` 的直接子项搬到 `output` 下，然后删掉空壳。
fn hoist_contents(inner: &Path, output: &Path) -> Result<(), String> {
    for entry in collect_entries(inner)? {
        let target = output.join(entry.file_name());
        fs::rename(entry.path(), &target)
            .map_err(|error| format!("could not move {}: {error}", target.display()))?;
    }
    fs::remove_dir(inner).map_err(|error| format!("could not remove {}: {error}", inner.display()))
}

/// 先把条目读完再动手：一边遍历目录一边把条目搬出去会漏掉后面的条目。
fn collect_entries(directory: &Path) -> Result<Vec<fs::DirEntry>, String> {
    fs::read_dir(directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("{}: {error}", directory.display()))
}

/// 名字比较：与 `-spe` 在 Windows 上的口径一致，忽略大小写。
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
    fn a_single_same_named_directory_is_hoisted() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let output = sandbox.path().join("bundle");
        let nested = output.join("bundle/nested");
        std::fs::create_dir_all(&nested).expect("create nested directory");
        std::fs::write(nested.join("payload.txt"), b"payload").expect("write payload");

        hoist_duplicate_root(&output, OsStr::new("bundle"));

        assert!(output.join("nested/payload.txt").is_file());
        assert!(!output.join("bundle").exists());
    }

    /// 只有“恰好一个子项 + 目录 + 同名”三者齐备才动手；大小写不同算同名。
    #[test]
    fn hoisting_only_touches_a_same_named_single_directory() {
        let build = |entries: &[(&str, bool)]| {
            let sandbox = tempfile::tempdir().expect("create test sandbox");
            let output = sandbox.path().join("bundle");
            std::fs::create_dir(&output).expect("create result directory");
            for (name, directory) in entries {
                let path = output.join(name);
                if *directory {
                    std::fs::create_dir_all(&path).expect("create directory");
                } else {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).expect("create parent directory");
                    }
                    std::fs::write(&path, b"x").expect("write file");
                }
            }
            sandbox
        };

        // 不止一个子项。
        let two = build(&[("bundle", true), ("other", false)]);
        hoist_duplicate_root(&two.path().join("bundle"), OsStr::new("bundle"));
        assert!(two.path().join("bundle/bundle").is_dir());

        // 单个同名文件。
        let file = build(&[("bundle", false)]);
        hoist_duplicate_root(&file.path().join("bundle"), OsStr::new("bundle"));
        assert!(file.path().join("bundle/bundle").is_file());

        // 名字不同。
        let different = build(&[("inner", true)]);
        hoist_duplicate_root(&different.path().join("bundle"), OsStr::new("bundle"));
        assert!(different.path().join("bundle/inner").is_dir());

        // 只有大小写不同。
        let case = build(&[("BUNDLE/payload.txt", false)]);
        hoist_duplicate_root(&case.path().join("bundle"), OsStr::new("bundle"));
        assert!(case.path().join("bundle/payload.txt").is_file());
    }
}
