use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::types::{EzzError, EzzResult};

pub fn locate_db() -> EzzResult<PathBuf> {
    let name = "ezz.db.txt";
    let ezz_path = env::current_exe()?;
    let home_dir = home::home_dir().ok_or(EzzError::FilePathError)?;
    let dirs = [ezz_path.parent().ok_or(EzzError::FilePathError)?, &home_dir];

    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|path| path.exists())
        .ok_or(EzzError::PasswordDbNotFound)
}

pub fn parse_db(db: &Path) -> EzzResult<Vec<(u32, String)>> {
    let entries = BufReader::new(File::open(db)?)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| {
            line.split_once(',')
                .and_then(|(freq, pw)| freq.parse::<u32>().ok().map(|f| (f, pw.to_string())))
        })
        .collect();
    Ok(entries)
}

pub fn update_db(db: &Path, entries: &mut Vec<(u32, String)>) -> EzzResult<()> {
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    let mut writer = BufWriter::new(File::create(db)?);
    for (freq, pw) in entries {
        writeln!(writer, "{freq},{pw}")?;
    }
    Ok(())
}
