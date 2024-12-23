mod arch;
mod cleanup;
mod password;
pub mod sevenzip;

use std::path::Path;

use crate::types::{EzzError, EzzResult};
use cleanup::*;
use password::*;
use sevenzip::*;

pub fn extract(archive: &Path, pw: Option<&str>, db: Option<&Path>) -> EzzResult<String> {
    let mut archive = archive.to_path_buf();
    let filename = archive
        .file_name()
        .ok_or(EzzError::FileNameError)?
        .to_string_lossy()
        .into_owned();
    let zz = setup_7zz()?;

    if is_stego(&archive) {
        handle_output(command_for_stego(&zz, &archive)?)?;
        remove_archive(&archive)?;
        archive = archive.with_file_name("2.zip");
    }

    if let Some(password) = pw {
        extract_with_pw(&zz, &archive, password)?;
    } else {
        extract_with_db(&zz, &archive, db)?;
    }

    teardown_7zz()?;
    Ok(filename)
}

fn is_stego(file: &Path) -> bool {
    matches!(
        file.extension().and_then(|ext| ext.to_str()),
        Some("mp4") | Some("mkv")
    )
}

fn extract_with_pw(zz: &str, archive: &Path, pw: &str) -> EzzResult<()> {
    let output = command_x(zz, archive, pw)?;
    let dir = derive_dir(archive)?;
    if let Err(e) = handle_output(output) {
        remove_dir(&dir)?;
        return Err(e);
    }
    remove_archive(archive)?;
    flatten_dir(&dir)
}

fn extract_with_db(zz: &str, archive: &Path, db: Option<&Path>) -> EzzResult<()> {
    let db = match db {
        Some(path) => path,
        None => &locate_db()?,
    };
    let mut entries = parse_db(db)?;

    for (freq, pw) in entries.iter_mut() {
        match extract_with_pw(zz, archive, pw) {
            Ok(result) => {
                *freq += 1;
                update_db(db, &mut entries)?;
                return Ok(result);
            }
            Err(EzzError::WrongPassword) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(EzzError::NoMatchedPassword)
}
