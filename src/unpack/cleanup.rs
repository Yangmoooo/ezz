use std::fs;

use super::archive::{Archive, VolumeType};
use crate::types::{EzzError, EzzResult};

impl Archive {
    pub fn remove(&self) -> EzzResult<()> {
        let path = self.get_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        match self.get_volume() {
            VolumeType::Single => Ok(()),
            _ => self.remove_multivolume(2),
        }
    }

    fn remove_multivolume(&self, seq: usize) -> EzzResult<()> {
        let volume = match self.get_volume() {
            VolumeType::Num => self.get_path().with_extension(format!("{:03}", seq)),
            VolumeType::Rar => {
                let stem = self.get_stem()?;
                let file_stem = stem
                    .trim_end_matches(char::is_numeric)
                    .strip_suffix(".part")
                    .ok_or(EzzError::PathError)?;
                self.get_path()
                    .with_file_name(format!("{}.part{}.rar", file_stem, seq))
            }
            VolumeType::Zip => self.get_path().with_extension(format!("z{:02}", seq - 1)),
            VolumeType::Single => unreachable!(),
        };
        if volume.exists() {
            fs::remove_file(&volume)?;
            self.remove_multivolume(seq + 1)?;
        }
        Ok(())
    }
}
