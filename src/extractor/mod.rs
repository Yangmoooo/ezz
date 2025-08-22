mod archive;
mod cleanup;
mod platform;
pub mod sevenzz;
mod wordlist;

use std::fs;
use std::path::{Path, PathBuf};

use crate::types::{EzzError, EzzResult};
pub use archive::Archive;
#[cfg(target_os = "linux")]
use platform::linux::explorer;
#[cfg(target_os = "windows")]
use platform::windows::{dialog::PasswordDialog, explorer};
use sevenzz::Sevenzz;
pub use wordlist::{Record, Wordlist, WordlistData};

impl Archive {
    pub fn extract(&self, pw: Option<&str>, wordlist: &Wordlist) -> EzzResult<String> {
        let zz = Sevenzz::initialize()?;

        let archive = if self.is_stegano {
            // 还原 Steganographier 的隐藏格式
            zz.command_x_stegano(self)?;
            self.remove()?;
            &self.with_name("2.zip")
        } else {
            self
        };

        let inner_file = zz.command_l(archive)?;
        let file_name = if let Some(password) = pw {
            archive.extract_with_pw(&zz, password, &inner_file)?
        } else {
            match archive.extract_with_pw(&zz, "", &inner_file) {
                Ok(name) => name,
                Err(EzzError::WrongPassword) => {
                    archive.extract_with_wordlist(&zz, wordlist, &inner_file)?
                }
                Err(e) => return Err(e),
            }
        };

        archive.remove()?;
        explorer::refresh_dir(archive.get_parent()?.to_str().ok_or(EzzError::PathError)?);
        Ok(file_name)
    }

    fn extract_with_pw(&self, zz: &Sevenzz, pw: &str, inner: &str) -> EzzResult<String> {
        zz.command_t(self, pw, inner)?;
        zz.command_x(self, pw)?;
        flatten_dir(&self.derive_dir())
    }

    fn extract_with_wordlist(
        &self,
        zz: &Sevenzz,
        wordlist: &Wordlist,
        inner: &str,
    ) -> EzzResult<String> {
        let mut data = wordlist.load()?;

        type PasswordTestFn = fn(&Archive, &Sevenzz, &WordlistData, &str) -> EzzResult<usize>;
        let mut try_extract = |test_fn: PasswordTestFn| -> EzzResult<Option<String>> {
            match test_fn(self, zz, &data, inner) {
                Ok(num) => {
                    zz.command_x(self, &data.records[num - 2].pw)?;
                    data.update(num);
                    wordlist.save(&data)?;
                    Ok(Some(flatten_dir(&self.derive_dir())?))
                }
                Err(EzzError::NoMatchedPassword) => Ok(None),
                Err(e) => Err(e),
            }
        };

        if let Some(result) = try_extract(Self::test_with_cache)? {
            return Ok(result);
        }
        if let Some(result) = try_extract(Self::test_with_records)? {
            return Ok(result);
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(pw) = PasswordDialog::ask_password()? {
                let result = self.extract_with_pw(zz, &pw, inner)?;
                data.records.push(Record { freq: 1, pw });
                data.update(data.records.len());
                wordlist.save(&data)?;
                return Ok(result);
            }
        }

        Err(EzzError::NoMatchedPassword)
    }
}

impl Archive {
    fn test_with_cache(&self, zz: &Sevenzz, data: &WordlistData, inner: &str) -> EzzResult<usize> {
        for &num in &data.cache {
            if let Some(Record { pw, .. }) = data.records.get(num - 2) {
                match zz.command_t(self, pw, inner) {
                    Ok(_) => return Ok(num),
                    Err(EzzError::WrongPassword) => continue,
                    Err(e) => return Err(e),
                }
            }
        }
        Err(EzzError::NoMatchedPassword)
    }

    fn test_with_records(
        &self,
        zz: &Sevenzz,
        data: &WordlistData,
        inner: &str,
    ) -> EzzResult<usize> {
        for (idx, Record { pw, .. }) in data.records.iter().enumerate() {
            match zz.command_t(self, pw, inner) {
                Ok(_) => return Ok(idx + 2),
                Err(EzzError::WrongPassword) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(EzzError::NoMatchedPassword)
    }
}

fn flatten_dir(dir: &Path) -> EzzResult<String> {
    if !dir.is_dir() {
        return Err(EzzError::PathError);
    }

    let dir_name = dir
        .file_name()
        .ok_or(EzzError::PathError)?
        .to_string_lossy();
    let mut result_name = dir_name.clone();

    let entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    if entries.len() == 1 {
        let entry = entries.first().ok_or(EzzError::PathError)?;
        let entry_name = entry.file_name().ok_or(EzzError::PathError)?;
        result_name = entry_name.to_string_lossy();

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
            fs::rename(entry, &target_path)?;
        }

        fs::remove_dir(dir)?;
        if temp_path.try_exists()? {
            fs::rename(temp_path, dir)?;
            result_name = dir_name;
        }
    }
    Ok(result_name.into_owned())
}
