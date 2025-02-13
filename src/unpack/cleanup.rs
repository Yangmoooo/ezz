use std::fs;
use std::path::{Path, PathBuf};

use crate::types::{EzzError, EzzResult};

pub fn derive_dir(archive: &Path) -> EzzResult<PathBuf> {
    let archive_stem = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(EzzError::PathError)?
        .trim_end();
    let dir = archive
        .parent()
        .ok_or(EzzError::PathError)?
        .join(archive_stem);
    Ok(dir)
}

pub fn flatten_dir(dir: &Path) -> EzzResult<String> {
    log::debug!("Flattening directory: {dir:?}");
    if !dir.is_dir() {
        return Err(EzzError::PathError);
    }
    let parent = dir.parent().ok_or(EzzError::PathError)?;
    let entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    let mut filename = dir
        .file_name()
        .ok_or(EzzError::PathError)?
        .to_string_lossy()
        .into_owned();

    if entries.len() == 1 {
        log::debug!("Moving single entry to parent directory");
        let entry = entries.first().ok_or(EzzError::PathError)?;
        let entryname = entry.file_name().ok_or(EzzError::PathError)?;
        filename = entryname.to_string_lossy().into_owned();

        let target_path = parent.join(entryname);
        // 若为 `.zip.7z` 这种嵌套的情况，内层压缩包名称可能会与解压目录冲突，故使用临时名称
        let tmp_path = target_path.with_extension("tmp");

        if target_path.exists() {
            if target_path.is_dir() {
                fs::rename(entry, &tmp_path)?;
            } else {
                return Err(EzzError::PathError);
            }
        } else {
            fs::rename(entry, target_path)?;
        }

        fs::remove_dir(dir)?;
        if tmp_path.exists() {
            fs::rename(tmp_path, dir)?;
        }
    }
    Ok(filename)
}

enum MultiVolumeKind {
    None,
    Rar, // such as `.part1.rar` `.part2.rar`
    Num, // such as `.7z.001` `.7z.002` or `.zip.001` `.zip.002`
    Zip, // such as `.zip` `.z01` `.z02`
}

pub fn remove_archive(archive: &Path) -> EzzResult<()> {
    if archive.exists() {
        fs::remove_file(archive)?;
    }
    match get_multivolume_kind(archive) {
        MultiVolumeKind::None => Ok(()),
        kind => remove_multivolume(kind, archive, 2),
    }
}

fn get_multivolume_kind(archive: &Path) -> MultiVolumeKind {
    let extension = match archive.extension().and_then(|s| s.to_str()) {
        Some("001") => return MultiVolumeKind::Num,
        Some("rar") => "rar",
        Some("zip") => "zip",
        _ => return MultiVolumeKind::None,
    };
    let stem = archive.file_stem().and_then(|s| s.to_str());

    match extension {
        "rar" if stem.is_some_and(|s| s.ends_with(".part1")) => MultiVolumeKind::Rar,
        "zip" => archive
            .parent()
            .and_then(|parent| stem.map(|s| parent.join(format!("{}.z01", s))))
            .filter(|volume| volume.exists())
            .map_or(MultiVolumeKind::None, |_| MultiVolumeKind::Zip),
        _ => MultiVolumeKind::None,
    }
}

fn remove_multivolume(kind: MultiVolumeKind, archive: &Path, seq: usize) -> EzzResult<()> {
    let parent = archive.parent().ok_or(EzzError::PathError)?;
    let file_stem = archive
        .file_stem()
        .ok_or(EzzError::PathError)?
        .to_string_lossy();
    let mut volume_path = PathBuf::new();
    match kind {
        MultiVolumeKind::Num => {
            let volume_extension = format!("{:03}", seq);
            let volume_name = format!("{}.{}", file_stem, volume_extension);
            volume_path = parent.join(volume_name);
        }
        MultiVolumeKind::Rar => {
            let file_stem = file_stem
                .trim_end_matches(char::is_numeric)
                .strip_suffix(".part")
                .ok_or(EzzError::PathError)?;
            let volume_name = format!("{}.part{}.rar", file_stem, seq);
            volume_path = parent.join(volume_name);
        }
        MultiVolumeKind::Zip => {
            let volume_extension = format!("z{:02}", seq - 1);
            let volume_name = format!("{}.{}", file_stem, volume_extension);
            volume_path = parent.join(volume_name);
        }
        MultiVolumeKind::None => {}
    }
    if volume_path.exists() {
        fs::remove_file(&volume_path)?;
        remove_multivolume(kind, archive, seq + 1)?;
    }
    Ok(())
}
