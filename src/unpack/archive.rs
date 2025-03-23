use std::fmt;
use std::path::PathBuf;

use crate::types::{EzzError, EzzResult};

pub enum VolumeType {
    Single,
    Rar, // such as `.part1.rar` `.part2.rar`
    Num, // such as `.7z.001` `.7z.002` or `.zip.001` `.zip.002`
    Zip, // such as `.zip` `.z01` `.z02`
}

#[derive(Clone)]
pub struct Archive(PathBuf);

impl Archive {
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self(path.into())
    }

    pub fn with_name(&self, name: &str) -> Self {
        Self(self.0.with_file_name(name))
    }

    pub fn get_path(&self) -> &PathBuf {
        &self.0
    }

    pub fn get_stem(&self) -> EzzResult<String> {
        self.0
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_owned())
            .ok_or(EzzError::PathError)
    }

    pub fn get_extension(&self) -> EzzResult<String> {
        self.0
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_owned())
            .ok_or(EzzError::PathError)
    }

    pub fn get_parent(&self) -> EzzResult<PathBuf> {
        self.0
            .parent()
            .map(|p| p.to_owned())
            .ok_or(EzzError::PathError)
    }
}

impl Archive {
    pub fn get_volume(&self) -> VolumeType {
        let extension = self.get_extension().unwrap_or_default();
        let stem = self.get_stem().unwrap_or_default();

        match extension.as_str() {
            "001" => VolumeType::Num,
            "rar" if stem.ends_with(".part1") => VolumeType::Rar,
            "zip" if self.get_path().with_extension("z01").exists() => VolumeType::Zip,
            _ => VolumeType::Single,
        }
    }

    pub fn is_hidden(&self) -> bool {
        matches!(
            self.get_extension().map(|ext| ext.to_ascii_lowercase()),
            Ok(ext) if ext == "mp4" || ext == "mkv"
        )
    }
}

impl fmt::Debug for Archive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
