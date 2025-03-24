use std::path::PathBuf;

use super::archive::{Archive, VolumeType};
use crate::types::EzzResult;

impl Archive {
    pub fn remove(&self) -> EzzResult<()> {
        let path = self.get_path();
        if path.try_exists()? {
            trash::delete(path)?;
        }
        match self.get_volume() {
            VolumeType::Single => Ok(()),
            _ => self.remove_multivolume(2),
        }
    }

    fn remove_multivolume(&self, seq: usize) -> EzzResult<()> {
        let generator: Box<dyn Fn(usize) -> PathBuf> = match self.get_volume() {
            VolumeType::Num => {
                Box::new(|seq| self.get_path().with_extension(format!("{:03}", seq)))
            }
            VolumeType::Rar => Box::new(|seq| {
                self.get_path()
                    .with_extension("")
                    .with_extension(format!("part{}.rar", seq))
            }),
            VolumeType::Zip => {
                Box::new(|seq| self.get_path().with_extension(format!("z{:02}", seq - 1)))
            }
            VolumeType::Single => unreachable!(),
        };
        let volumes = collect_volumes(seq, generator);
        trash::delete_all(volumes)?;

        Ok(())
    }
}

fn collect_volumes<F>(mut seq: usize, generator: F) -> Vec<PathBuf>
where
    F: Fn(usize) -> PathBuf,
{
    let mut volumes: Vec<PathBuf> = Vec::new();
    loop {
        let volume = generator(seq);
        if volume.try_exists().unwrap_or(false) {
            volumes.push(volume);
            seq += 1;
        } else {
            break;
        }
    }
    volumes
}
