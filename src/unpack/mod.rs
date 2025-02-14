mod arch;
mod cleanup;
mod password;
pub mod sevenzip;

use std::path::Path;

use crate::types::{EzzError, EzzResult};
#[cfg(target_os = "windows")]
use arch::windows::dialog::PasswordDialog;
use cleanup::{derive_dir, flatten_dir, remove_archive};
pub use password::locate_db;
use password::{parse_db, update_db};
use sevenzip::*;

pub fn extract(archive: &Path, pwd: Option<&str>, db: Option<&Path>) -> EzzResult<String> {
    let mut archive = archive.to_path_buf();
    let zz = setup_7zz()?;
    log::debug!("7-Zip Path: {zz:?}");

    if is_stego(&archive) {
        log::debug!("Stego file detected: {archive:?}");
        handle_output(command_for_stego(&zz, &archive)?)?;
        remove_archive(&archive)?;
        archive = archive.with_file_name("2.zip");
    }

    let filename = if let Some(password) = pwd {
        extract_with_pwd(&zz, &archive, password)?
    } else {
        extract_with_db(&zz, &archive, db)?
    };
    remove_archive(&archive)?;

    log::debug!("Removing 7-Zip executable");
    teardown_7zz()?;
    Ok(filename)
}

fn is_stego(file: &Path) -> bool {
    matches!(
        file.extension().and_then(|ext| ext.to_str()),
        Some("mp4") | Some("mkv")
    )
}

fn extract_with_pwd(zz: &str, archive: &Path, pwd: &str) -> EzzResult<String> {
    handle_output(command_t(zz, archive, pwd)?)?;
    handle_output(command_x(zz, archive, pwd)?)?;
    let dir = derive_dir(archive)?;
    flatten_dir(&dir)
}

fn extract_with_db(zz: &str, archive: &Path, db: Option<&Path>) -> EzzResult<String> {
    let db = match db {
        Some(path) => path,
        None => &locate_db()?,
    };
    let mut entries = parse_db(db)?;

    for (freq, pwd) in entries.iter_mut() {
        match extract_with_pwd(zz, archive, pwd) {
            Ok(result) => {
                *freq += 1;
                update_db(db, &mut entries)?;
                return Ok(result);
            }
            Err(EzzError::WrongPassword) => continue,
            Err(e) => return Err(e),
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(pwd) = PasswordDialog::ask_password()? {
            let result = extract_with_pwd(zz, archive, &pwd)?;
            entries.push((1, pwd));
            update_db(db, &mut entries)?;
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
