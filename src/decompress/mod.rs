mod arch;
mod pwdb;
pub mod sevenz;
mod utils;

use std::path::Path;
use std::fs;

use crate::error::EzzError as Error;
use pwdb::*;
use sevenz::*;
use utils::*;

pub struct ExtractRes {
    pub first_file: String,
    pub file_count: usize,
}

pub fn extract(archive: &Path, pw: Option<&str>, db: Option<&Path>) -> Result<ExtractRes, Error> {
    let mut archive = archive.to_path_buf();
    let zz = setup_7zz()?;

    if is_stego(&archive) {
        handle_output(command_for_stego(&zz, &archive)?)?;
        archive = archive.with_file_name("2.zip");
    }

    let result = pw.map_or_else(
        || extract_with_db(&zz, &archive, db),
        |password| extract_with_pw(&zz, &archive, password),
    );
    teardown_7zz()?;
    result
}

fn extract_with_pw(zz: &str, archive: &Path, pw: &str) -> Result<ExtractRes, Error> {
    let output = command_x(zz, archive, pw)?;
    let dir = derive_dir(archive)?;
    if let Err(e) = handle_output(output) {
        delete_dir(&dir)?;
        return Err(e);
    }
    fs::remove_file(archive)?;
    flatten_dir(&dir)
}

fn extract_with_db(zz: &str, archive: &Path, db: Option<&Path>) -> Result<ExtractRes, Error> {
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
            Err(Error::WrongPassword) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(Error::NoMatchedPassword)
}
