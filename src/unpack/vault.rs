use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::{default, env, fmt};

use crate::types::EzzResult;

const VAULT_NAME: &str = "ezz.vault";

pub struct Vault(PathBuf);

impl Vault {
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self(path.into())
    }

    pub fn parse(&self) -> EzzResult<Vec<(u32, String)>> {
        Ok(BufReader::new(File::open(&self.0)?)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| {
                line.split_once(',')
                    .and_then(|(freq, pwd)| freq.parse::<u32>().ok().map(|f| (f, pwd.to_string())))
            })
            .collect())
    }

    pub fn add(&self, pwd: &str) -> EzzResult<()> {
        let mut file = OpenOptions::new().create(true).append(true).open(&self.0)?;
        writeln!(file, "0,{}", pwd)?;
        Ok(())
    }

    pub fn save(&self, pairs: &mut Vec<(u32, String)>) -> EzzResult<()> {
        let mut writer = BufWriter::new(File::create(&self.0)?);
        for (freq, pwd) in pairs {
            writeln!(writer, "{freq},{pwd}")?;
        }
        writer.flush()?;
        Ok(())
    }
}

impl default::Default for Vault {
    fn default() -> Self {
        let vault_path = env::current_exe()
            .ok()
            .and_then(|ezz_path| ezz_path.parent().map(|p| p.to_path_buf()))
            .and_then(|ezz_dir| {
                let candidate = ezz_dir.join(VAULT_NAME);
                if candidate.exists() {
                    Some(candidate)
                } else {
                    None
                }
            })
            .or_else(|| {
                home::home_dir().and_then(|home_dir| {
                    let candidate = home_dir.join(VAULT_NAME);
                    if candidate.exists() {
                        Some(candidate)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();

        Vault::new(vault_path)
    }
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
