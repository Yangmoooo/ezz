mod arch;
mod archive;
mod cleanup;
pub mod sevenzz;
mod vault;

use std::fs;
use std::path::PathBuf;

use crate::types::{EzzError, EzzResult};
#[cfg(target_os = "windows")]
use arch::windows::dialog::PasswordDialog;
pub use archive::Archive;
use sevenzz::Sevenzz;
pub use vault::Vault;

impl Archive {
    pub fn extract(&self, pwd: Option<&str>, vault: &Vault) -> EzzResult<String> {
        let zz = Sevenzz::construct_from_embed()?;

        let mut archive = self.clone();
        if archive.is_stego() {
            zz.command_for_stego(&archive)?;
            archive.remove()?;
            archive = archive.with_name("2.zip");
        }

        let inner_file = zz.command_l(&archive)?;
        let file_name = if let Some(password) = pwd {
            archive.extract_with_pwd(&zz, password, &inner_file)?
        } else {
            archive.extract_with_vault(&zz, vault, &inner_file)?
        };
        archive.remove()?;

        zz.deconstruct()?;
        Ok(file_name)
    }

    fn extract_with_pwd(&self, zz: &Sevenzz, pwd: &str, inner: &str) -> EzzResult<String> {
        zz.command_t(self, pwd, inner)?;
        zz.command_x(self, pwd)?;
        flatten_dir(self.derive_dir()?)
    }

    fn extract_with_vault(&self, zz: &Sevenzz, vault: &Vault, inner: &str) -> EzzResult<String> {
        let mut pairs = vault.parse()?;
        for (idx, (freq, pwd)) in pairs.iter_mut().enumerate() {
            match self.extract_with_pwd(zz, pwd, inner) {
                Ok(result) => {
                    *freq += 1;
                    bubble_up(idx, &mut pairs);
                    vault.save(&mut pairs)?;
                    return Ok(result);
                }
                Err(EzzError::WrongPassword) => continue,
                Err(e) => return Err(e),
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(pwd) = PasswordDialog::ask_password()? {
                let result = self.extract_with_pwd(zz, &pwd, inner)?;
                pairs.push((1, pwd));
                bubble_up(pairs.len() - 1, &mut pairs);
                vault.save(&mut pairs)?;
                Ok(result)
            } else {
                Err(EzzError::NoMatchedPassword)
            }
        }

        #[cfg(target_os = "linux")]
        {
            Err(EzzError::NoMatchedPassword)
        }
    }
}

impl Archive {
    pub fn derive_dir(&self) -> EzzResult<PathBuf> {
        Ok(self.get_path().with_file_name(self.get_stem()?))
    }
}

fn flatten_dir(dir: PathBuf) -> EzzResult<String> {
    if !dir.is_dir() {
        return Err(EzzError::PathError);
    }

    let mut result_name = dir
        .file_name()
        .ok_or(EzzError::PathError)?
        .to_string_lossy()
        .into_owned();

    let entries: Vec<PathBuf> = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    if entries.len() == 1 {
        let entry = entries.first().ok_or(EzzError::PathError)?;
        let entry_name = entry.file_name().ok_or(EzzError::PathError)?;
        result_name = entry_name.to_string_lossy().into_owned();

        let target_path = dir.with_file_name(entry_name);
        // 若为 `.zip.7z` 这种嵌套的情况，内层压缩包名称可能会与解压目录冲突，故使用临时名称
        let temp_path = target_path.with_extension("tmp");

        if target_path.try_exists()? {
            if target_path.is_dir() {
                fs::rename(entry, &temp_path)?;
            } else {
                // 内层压缩包与当前压缩包同名时也会进入此分支
                return Err(EzzError::PathError);
            }
        } else {
            fs::rename(entry, target_path)?;
        }

        fs::remove_dir(&dir)?;
        if temp_path.try_exists()? {
            fs::rename(temp_path, dir)?;
        }
    }
    Ok(result_name)
}

fn bubble_up(index: usize, pairs: &mut [(u32, String)]) {
    let mut i = index;
    while i > 0 && pairs[i].0 >= pairs[i - 1].0 {
        pairs.swap(i, i - 1);
        i -= 1;
    }
}
