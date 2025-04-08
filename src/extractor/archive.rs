use std::fmt;
use std::path::{Path, PathBuf};

use crate::types::{EzzError, EzzResult};

#[derive(Copy, Clone)]
pub enum VolumeType {
    Single,
    Rar, // such as `.part1.rar` `.part2.rar`
    Num, // such as `.7z.001` `.7z.002` or `.zip.001` `.zip.002`
    Zip, // such as `.zip` `.z01` `.z02`
}

#[derive(Clone)]
pub struct Archive {
    path: PathBuf,
    pub volume: VolumeType,
    pub is_stegano: bool, // 当文件后缀为 mp4 或 mkv 时，将作为 Steganographier 的隐写格式处理
}

impl Archive {
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        let path: PathBuf = path.into();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let volume = match ext {
            "001" => VolumeType::Num,
            "rar" if stem.ends_with(".part1") => VolumeType::Rar,
            "zip" if path.with_extension("z01").exists() => VolumeType::Zip,
            _ => VolumeType::Single,
        };
        let is_stegano = matches!(ext.to_ascii_lowercase().as_str(), "mp4" | "mkv");

        Self {
            path,
            volume,
            is_stegano,
        }
    }

    pub fn with_name(&self, name: &str) -> Self {
        Self::new(self.path.with_file_name(name))
    }

    pub fn get_path(&self) -> &Path {
        &self.path
    }

    pub fn get_parent(&self) -> EzzResult<&Path> {
        self.path.parent().ok_or(EzzError::PathError)
    }
}

impl Archive {
    pub fn derive_dir(&self) -> PathBuf {
        match self.volume {
            VolumeType::Single | VolumeType::Zip => self.path.with_extension(""),
            VolumeType::Num | VolumeType::Rar => self.path.with_extension("").with_extension(""),
        }
    }
}

impl fmt::Debug for Archive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.path)
    }
}
