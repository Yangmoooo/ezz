use std::fs;
use std::path::{Path, PathBuf};

use super::ExtractRes;
use crate::error::EzzError as Error;

pub fn derive_dir(archive: &Path) -> Result<PathBuf, Error> {
    let archive_stem = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(Error::FileNameError)?;
    let dir = archive
        .parent()
        .ok_or(Error::FilePathError)?
        .join(archive_stem);
    Ok(dir)
}

pub fn remove_dir(dir: &Path) -> Result<(), Error> {
    if dir.exists() && dir.is_dir() {
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn flatten_dir(dir: &Path) -> Result<ExtractRes, Error> {
    if !dir.is_dir() {
        return Err(Error::FilePathError);
    }
    let parent = dir.parent().ok_or(Error::FilePathError)?;
    let entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();

    if entries.len() <= 2 {
        // 处理了压缩包嵌套时可能发生的文件名与文件夹名冲突
        let mut staging_path = None;
        for entry in &entries {
            let new_path = parent.join(entry.file_name().ok_or(Error::FileNameError)?);
            if new_path.exists() && new_path.is_dir() {
                let tmp_path = new_path.with_file_name("tmp");
                fs::rename(entry, &tmp_path)?;
                staging_path = Some(tmp_path);
            } else {
                fs::rename(entry, new_path)?;
            }
        }
        fs::remove_dir(dir)?;
        if let Some(tmp_path) = staging_path {
            fs::rename(tmp_path, dir)?;
        }
    }

    let file_name = entries[0]
        .file_name()
        .ok_or(Error::FileNameError)?
        .to_string_lossy()
        .into_owned();
    Ok(ExtractRes {
        first_file: file_name,
        file_count: entries.len(),
    })
}

enum MultiVolumeKind {
    None,
    Rar,
    Sevenz,
}

pub fn remove_archive(archive: &Path) -> Result<(), Error> {
    if archive.exists() {
        fs::remove_file(archive)?;
    }
    match get_multivolume_kind(archive) {
        MultiVolumeKind::None => Ok(()),
        kind => remove_multivolume(kind, archive, 2),
    }
}

fn get_multivolume_kind(archive: &Path) -> MultiVolumeKind {
    let ext = archive.extension().and_then(|s| s.to_str());
    match ext {
        Some("001") => MultiVolumeKind::Sevenz,
        Some("rar") => {
            let stem = archive.file_stem().and_then(|s| s.to_str());
            match stem {
                Some(stem) if stem.ends_with(".part1") => MultiVolumeKind::Rar,
                _ => MultiVolumeKind::None,
            }
        }
        _ => MultiVolumeKind::None,
    }
}

fn remove_multivolume(kind: MultiVolumeKind, archive: &Path, index: usize) -> Result<(), Error> {
    let parent = archive.parent().ok_or(Error::FilePathError)?;
    let file_stem = archive
        .file_stem()
        .ok_or(Error::FileNameError)?
        .to_string_lossy();
    let mut volume_path = PathBuf::new();
    match kind {
        MultiVolumeKind::Sevenz => {
            let volume_extension = format!("{:03}", index);
            let volume_name = format!("{}.{}", file_stem, volume_extension);
            volume_path = parent.join(volume_name);
        }
        MultiVolumeKind::Rar => {
            let file_stem = file_stem
                .trim_end_matches(char::is_numeric)
                .strip_suffix(".part")
                .ok_or(Error::FileNameError)?;
            let volume_name = format!("{}.part{}.rar", file_stem, index);
            volume_path = parent.join(volume_name);
        }
        MultiVolumeKind::None => {}
    }
    if volume_path.exists() {
        fs::remove_file(&volume_path)?;
        remove_multivolume(kind, archive, index + 1)?;
    }
    Ok(())
}
