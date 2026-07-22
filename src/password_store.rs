use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const DATABASE_VERSION: u32 = 1;

pub(crate) struct PasswordStore {
    path: PathBuf,
}

impl PasswordStore {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn candidates(&self) -> Result<Vec<String>, String> {
        let mut database = self.load()?;
        database.passwords.sort_by(|left, right| {
            right
                .last_used
                .cmp(&left.last_used)
                .then_with(|| right.uses.cmp(&left.uses))
        });
        Ok(database
            .passwords
            .into_iter()
            .map(|record| record.password)
            .collect())
    }

    pub(crate) fn record_success(&self, password: &str) -> Result<(), String> {
        let mut database = self.load()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs();

        if let Some(record) = database
            .passwords
            .iter_mut()
            .find(|record| record.password == password)
        {
            record.uses = record.uses.saturating_add(1);
            record.last_used = now;
        } else {
            database.passwords.push(PasswordRecord {
                password: password.to_owned(),
                uses: 1,
                last_used: now,
            });
        }

        self.save(&database)
    }

    fn load(&self) -> Result<PasswordDatabase, String> {
        if !self.path.exists() {
            return Ok(PasswordDatabase::default());
        }

        let reader = BufReader::new(File::open(&self.path).map_err(|error| error.to_string())?);
        let database: PasswordDatabase =
            serde_json::from_reader(reader).map_err(|error| error.to_string())?;
        if database.version != DATABASE_VERSION {
            return Err(format!(
                "unsupported password database version {}",
                database.version
            ));
        }
        Ok(database)
    }

    fn save(&self, database: &PasswordDatabase) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "password database has no parent directory".to_owned())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;

        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        {
            let mut writer = BufWriter::new(temporary.as_file_mut());
            serde_json::to_writer_pretty(&mut writer, database)
                .map_err(|error| error.to_string())?;
            writer.write_all(b"\n").map_err(|error| error.to_string())?;
            writer.flush().map_err(|error| error.to_string())?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        set_private_permissions(temporary.path()).map_err(|error| error.to_string())?;
        temporary
            .persist(&self.path)
            .map_err(|error| error.error.to_string())?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct PasswordDatabase {
    version: u32,
    passwords: Vec<PasswordRecord>,
}

impl Default for PasswordDatabase {
    fn default() -> Self {
        Self {
            version: DATABASE_VERSION,
            passwords: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct PasswordRecord {
    password: String,
    uses: u64,
    last_used: u64,
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
}

#[cfg(windows)]
fn set_private_permissions(path: &Path) -> std::io::Result<()> {
    use std::ffi::OsString;
    use std::process::Command;

    let username = std::env::var_os("USERNAME")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "USERNAME is not set"))?;
    let mut account = OsString::new();
    if let Some(domain) = std::env::var_os("USERDOMAIN") {
        account.push(domain);
        account.push("\\");
    }
    account.push(username);
    account.push(":F");

    let status = Command::new("icacls.exe")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(account)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "icacls exited with {status}"
        )))
    }
}
