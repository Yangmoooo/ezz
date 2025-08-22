use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::{default, env, fmt};

use crate::types::{EzzError, EzzResult};

const WORDLIST_NAME: &str = ".ezz.pw";
const WORDLIST_CACHE_SIZE: usize = 3;

pub struct Record {
    pub freq: u32,
    pub pw: String,
}

pub struct WordlistData {
    pub cache: Vec<usize>,    // 最近使用过的密码行号，0 和 1 无效
    pub records: Vec<Record>, // 密码及其频率
}

impl WordlistData {
    pub fn update(&mut self, num: usize) {
        // 更新 records
        let (old, mut new) = (num - 2, num - 2);
        self.records[old].freq += 1;
        while new > 0 && self.records[new].freq >= self.records[new - 1].freq {
            self.records.swap(new, new - 1);
            new -= 1;
        }
        // 更新 cache
        let mut pos = WORDLIST_CACHE_SIZE - 1;
        for i in 0..WORDLIST_CACHE_SIZE {
            let n = self.cache[i];
            if n == num {
                pos = i;
            }
            if new <= n - 2 && n - 2 < old {
                self.cache[i] += 1;
            }
        }
        self.cache.remove(pos);
        self.cache.insert(0, new + 2);
    }
}

pub struct Wordlist(PathBuf);

impl Wordlist {
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
        let content = ["0"; WORDLIST_CACHE_SIZE].join(" ") + "\n";
        file.write_all(content.as_bytes())?;
        Ok(())
    }

    pub fn load(&self) -> EzzResult<WordlistData> {
        let mut lines = BufReader::new(File::open(&self.0)?)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.is_empty());
        let cache: Vec<usize> = lines
            .next()
            .ok_or(EzzError::WordlistError)?
            .split_whitespace()
            .filter_map(|num| num.parse::<usize>().ok())
            .take(WORDLIST_CACHE_SIZE)
            .collect::<Vec<usize>>();
        let records: Vec<Record> = lines
            .filter_map(|line| {
                line.split_once(',').and_then(|(freq, pw)| {
                    freq.parse::<u32>().ok().map(|f| Record {
                        freq: f,
                        pw: pw.to_string(),
                    })
                })
            })
            .collect();
        Ok(WordlistData { cache, records })
    }

    pub fn add(&self, pw: &str) -> EzzResult<()> {
        let mut file = OpenOptions::new().append(true).open(&self.0)?;
        writeln!(file, "0,{pw}")?;
        Ok(())
    }

    pub fn save(&self, data: &WordlistData) -> EzzResult<()> {
        let mut writer = BufWriter::new(File::create(&self.0)?);
        for num in &data.cache {
            write!(writer, "{num} ")?;
        }
        writeln!(writer)?;
        for Record { freq, pw } in &data.records {
            writeln!(writer, "{freq},{pw}")?;
        }
        writer.flush()?;
        Ok(())
    }
}

impl default::Default for Wordlist {
    fn default() -> Self {
        let wordlist_path = env::current_exe()
            .ok()
            .and_then(|ezz_path| ezz_path.parent().map(|p| p.to_path_buf()))
            .and_then(|ezz_dir| {
                let candidate = ezz_dir.join(WORDLIST_NAME);
                if candidate.exists() {
                    Some(candidate)
                } else {
                    None
                }
            })
            .or_else(|| {
                home::home_dir().and_then(|home_dir| {
                    let candidate = home_dir.join(WORDLIST_NAME);
                    if candidate.exists() {
                        Some(candidate)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();

        Wordlist::new(wordlist_path)
    }
}

impl fmt::Debug for Wordlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
