use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::{default, env, fmt};

use crate::types::{EzzError, EzzResult};

const VAULT_NAME: &str = "ezz.vault";
const VAULT_CACHE_SIZE: usize = 3;

pub struct Record {
    pub freq: u32,
    pub pwd: String,
}

pub struct VaultData {
    pub cache: Vec<usize>,    // 最近使用过的密码行号，0 和 1 无效
    pub records: Vec<Record>, // 密码及其频率
}

impl VaultData {
    pub fn update(&mut self, num: usize) {
        let mut pos = num - 2;
        self.records[pos].freq += 1;
        while pos > 0 && self.records[pos].freq >= self.records[pos - 1].freq {
            self.records.swap(pos, pos - 1);
            pos -= 1;
        }
        if let Some(pos) = self.cache.iter().position(|&n| n == num) {
            self.cache.remove(pos);
        }
        self.cache.insert(0, pos + 2);
        self.cache.truncate(VAULT_CACHE_SIZE);
    }
}

pub struct Vault(PathBuf);

impl Vault {
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self(path.into())
    }

    pub fn exists(&self) -> bool {
        self.0.exists()
    }

    pub fn init(&self) -> EzzResult<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.0)?;
        let content = ["0"; VAULT_CACHE_SIZE].join(" ") + "\n";
        file.write_all(content.as_bytes())?;
        Ok(())
    }

    pub fn load(&self) -> EzzResult<VaultData> {
        let mut lines = BufReader::new(File::open(&self.0)?)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.is_empty());
        let cache: Vec<usize> = lines
            .next()
            .ok_or(EzzError::VaultError)?
            .split_whitespace()
            .filter_map(|num| num.parse::<usize>().ok())
            .take(VAULT_CACHE_SIZE)
            .collect::<Vec<usize>>();
        let records: Vec<Record> = lines
            .filter_map(|line| {
                line.split_once(',').and_then(|(freq, pwd)| {
                    freq.parse::<u32>().ok().map(|f| Record {
                        freq: f,
                        pwd: pwd.to_string(),
                    })
                })
            })
            .collect();
        Ok(VaultData { cache, records })
    }

    pub fn add(&self, pwd: &str) -> EzzResult<()> {
        let mut file = OpenOptions::new().append(true).open(&self.0)?;
        writeln!(file, "0,{pwd}")?;
        Ok(())
    }

    pub fn save(&self, data: &VaultData) -> EzzResult<()> {
        let mut writer = BufWriter::new(File::create(&self.0)?);
        for num in &data.cache {
            write!(writer, "{} ", num)?;
        }
        writeln!(writer)?;
        for Record { freq, pwd } in &data.records {
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
