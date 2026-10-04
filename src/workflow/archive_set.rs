//! 分卷归档的识别与完整性检查。
//!
//! 三个分卷家族（`.001`、`.partN.rar`、`.z01`+`.zip`）共用同一套扫描与缺号检查。

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use super::{ExtractionError, file_system_error};

pub(super) struct ArchiveSet {
    pub(super) primary: PathBuf,
    pub(super) sources: Vec<PathBuf>,
    pub(super) output_stem: OsString,
}

/// 同一逻辑归档的卷：序号 → 路径（按键有序，缺号检查依赖这一点）。
type VolumeSet = BTreeMap<u32, PathBuf>;

/// 扫一遍归档所在目录，挑出属于同一个逻辑归档的卷。
///
/// `sequence_of` 返回序号即收录该文件，返回 `None` 表示与本次输入无关。
fn scan_volumes(
    parent: &Path,
    mut sequence_of: impl FnMut(&Path) -> Option<u32>,
) -> Result<VolumeSet, ExtractionError> {
    let mut volumes = VolumeSet::new();
    let entries = fs::read_dir(parent)
        .map_err(|error| file_system_error("scan archive volumes in", parent, error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| file_system_error("scan archive volume in", parent, error))?;
        let path = entry.path();
        if let Some(sequence) = sequence_of(&path) {
            volumes.insert(sequence, path);
        }
    }
    Ok(volumes)
}

/// 1 到最高序号之间不得缺号，缺号是致命失败。
///
/// 最高序号以 `selected`（用户点中的那一卷）为下限，目录里只剩它自己时也能通过检查。
fn require_contiguous(
    volumes: &VolumeSet,
    selected: u32,
    missing: impl Fn(u32) -> PathBuf,
) -> Result<(), ExtractionError> {
    let last = volumes.keys().next_back().copied().unwrap_or(selected);
    for number in 1..=last {
        if !volumes.contains_key(&number) {
            return Err(ExtractionError::MissingVolume(missing(number)));
        }
    }
    Ok(())
}

pub(super) fn resolve_archive_set(selected: &Path) -> Result<ArchiveSet, ExtractionError> {
    if let Some(sequence) = numeric_extension(selected) {
        return resolve_numeric_archive_set(selected, sequence);
    }
    if let Some(volume) = rar_volume_name(selected) {
        return resolve_rar_archive_set(selected, &volume);
    }
    if let Some(sequence) = zip_volume_sequence(selected) {
        return resolve_zip_archive_set(selected, Some(sequence));
    }
    if has_zip_extension(selected) {
        return resolve_zip_archive_set(selected, None);
    }

    Ok(ArchiveSet {
        primary: selected.to_path_buf(),
        sources: vec![selected.to_path_buf()],
        output_stem: archive_stem(selected),
    })
}

fn resolve_numeric_archive_set(
    selected: &Path,
    sequence: u32,
) -> Result<ArchiveSet, ExtractionError> {
    let first = selected.with_extension("001");
    if !first.is_file() {
        return Err(ExtractionError::MissingVolume(first));
    }

    let parent = selected.parent().expect("absolute input parent");
    let prefix = selected.file_stem().expect("volume file stem");
    let volumes = scan_volumes(parent, |path| {
        if path.file_stem() == Some(prefix) {
            numeric_extension(path)
        } else {
            None
        }
    })?;
    require_contiguous(&volumes, sequence, |number| {
        selected.with_extension(format!("{number:03}"))
    })?;

    Ok(ArchiveSet {
        primary: first,
        sources: volumes.into_values().collect(),
        output_stem: archive_stem(&selected.with_extension("")),
    })
}

struct RarVolumeName {
    prefix: String,
    sequence: u32,
    width: usize,
    extension: String,
}

fn resolve_rar_archive_set(
    selected: &Path,
    selected_volume: &RarVolumeName,
) -> Result<ArchiveSet, ExtractionError> {
    let parent = selected.parent().expect("absolute input parent");
    let volumes = scan_volumes(parent, |path| {
        rar_volume_name(path)
            .filter(|volume| {
                volume.prefix == selected_volume.prefix
                    && volume
                        .extension
                        .eq_ignore_ascii_case(&selected_volume.extension)
            })
            .map(|volume| volume.sequence)
    })?;
    require_contiguous(&volumes, selected_volume.sequence, |number| {
        rar_volume_path(parent, selected_volume, number)
    })?;

    Ok(ArchiveSet {
        primary: volumes.get(&1).expect("first RAR volume checked").clone(),
        sources: volumes.into_values().collect(),
        output_stem: OsString::from(&selected_volume.prefix),
    })
}

fn rar_volume_name(path: &Path) -> Option<RarVolumeName> {
    let name = path.file_name()?.to_str()?;
    let bytes = name.as_bytes();
    if bytes.len() < 10 || !bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".rar") {
        return None;
    }
    let part = bytes[..bytes.len() - 4]
        .windows(5)
        .rposition(|window| window.eq_ignore_ascii_case(b".part"))?;
    let digits = &name[part + 5..name.len() - 4];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    Some(RarVolumeName {
        prefix: name[..part].to_owned(),
        sequence: digits.parse().ok()?,
        width: digits.len(),
        extension: name[name.len() - 3..].to_owned(),
    })
}

fn rar_volume_path(parent: &Path, volume: &RarVolumeName, sequence: u32) -> PathBuf {
    parent.join(format!(
        "{}.part{:0width$}.{}",
        volume.prefix,
        sequence,
        volume.extension,
        width = volume.width
    ))
}

fn resolve_zip_archive_set(
    selected: &Path,
    selected_sequence: Option<u32>,
) -> Result<ArchiveSet, ExtractionError> {
    let parent = selected.parent().expect("absolute input parent");
    let stem = selected.file_stem().expect("volume file stem");
    // 编号卷（`.z01`…）与末尾的无编号 `.zip` 是两类：后者不带序号，单独收集。
    let mut final_volume = None;
    let volumes = scan_volumes(parent, |path| {
        if path.file_stem() != Some(stem) {
            return None;
        }
        if let Some(sequence) = zip_volume_sequence(path) {
            return Some(sequence);
        }
        if has_zip_extension(path) {
            final_volume = Some(path.to_path_buf());
        }
        None
    })?;

    // 单个 `.zip`：目录里没有编号卷，选中的就是它自己。
    if selected_sequence.is_none() && volumes.is_empty() {
        return Ok(ArchiveSet {
            primary: selected.to_path_buf(),
            sources: vec![selected.to_path_buf()],
            output_stem: archive_stem(selected),
        });
    }

    let Some(final_volume) = final_volume else {
        return Err(ExtractionError::MissingVolume(
            selected.with_extension("zip"),
        ));
    };
    require_contiguous(&volumes, selected_sequence.unwrap_or(0), |number| {
        selected.with_extension(format!("z{number:02}"))
    })?;

    let mut sources: Vec<_> = volumes.into_values().collect();
    sources.push(final_volume.clone());
    Ok(ArchiveSet {
        primary: final_volume,
        sources,
        output_stem: archive_stem(selected),
    })
}

fn archive_stem(path: &Path) -> OsString {
    path.file_stem()
        .unwrap_or_else(|| OsStr::new("archive"))
        .to_os_string()
}

fn zip_volume_sequence(path: &Path) -> Option<u32> {
    let extension = path.extension()?.to_str()?;
    let bytes = extension.as_bytes();
    (bytes.len() == 3
        && matches!(bytes[0], b'z' | b'Z')
        && bytes[1..].iter().all(u8::is_ascii_digit))
    .then(|| extension[1..].parse().ok())
    .flatten()
}

fn has_zip_extension(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn numeric_extension(path: &Path) -> Option<u32> {
    let extension = path.extension()?.to_str()?;
    (extension.len() == 3 && extension.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| extension.parse().ok())
        .flatten()
}

/// 单元测试：这些用例只碰本模块的纯逻辑。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_suffixes_are_recognized() {
        assert_eq!(numeric_extension(Path::new("data.001")), Some(1));
        assert_eq!(numeric_extension(Path::new("data.003")), Some(3));
        assert_eq!(numeric_extension(Path::new("data.01")), None);
        assert_eq!(numeric_extension(Path::new("data.zip")), None);
        assert_eq!(numeric_extension(Path::new("data")), None);

        assert_eq!(zip_volume_sequence(Path::new("data.z01")), Some(1));
        assert_eq!(zip_volume_sequence(Path::new("data.Z09")), Some(9));
        assert_eq!(zip_volume_sequence(Path::new("data.z1")), None);
        assert_eq!(zip_volume_sequence(Path::new("data.zip")), None);
        assert!(has_zip_extension(Path::new("data.ZIP")));

        assert_eq!(
            archive_stem(Path::new("archive.tar.gz")),
            OsString::from("archive.tar")
        );
        assert_eq!(
            archive_stem(Path::new("no-extension")),
            OsString::from("no-extension")
        );

        let volume = rar_volume_name(Path::new("book.part002.rar")).expect("rar volume name");
        assert_eq!(volume.prefix, "book");
        assert_eq!(volume.sequence, 2);
        assert_eq!(volume.width, 3);
        assert_eq!(volume.extension, "rar");
        assert_eq!(
            rar_volume_path(Path::new("C:/data"), &volume, 7),
            PathBuf::from("C:/data/book.part007.rar")
        );
        assert!(rar_volume_name(Path::new("book.rar")).is_none());
        assert!(rar_volume_name(Path::new("book.part.rar")).is_none());
        assert!(rar_volume_name(Path::new("book.part01.zip")).is_none());
    }
}
