use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use log::warn;
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

    /// 密码候选，按最近使用时间与使用次数排序。
    ///
    /// 读取失败**不得**让任何输入失败（设计 §7）：记录警告、按空候选继续 —— 用户仍然
    /// 可以在弹窗里输入密码。真正不可解析的文件会在下一次成功保存前被改名保留。
    pub(crate) fn candidates(&self) -> Vec<String> {
        let mut database = match self.load() {
            Ok(database) => database,
            Err(error) => {
                warn!(
                    "ignoring unreadable password database {}: {error}",
                    self.path.display()
                );
                return Vec::new();
            }
        };

        database.passwords.sort_by(|left, right| {
            right
                .last_used
                .cmp(&left.last_used)
                .then_with(|| right.uses.cmp(&left.uses))
        });
        database
            .passwords
            .into_iter()
            .map(|record| record.password)
            .collect()
    }

    pub(crate) fn record_success(&self, password: &str) -> Result<(), String> {
        let mut database = match self.load() {
            Ok(database) => database,
            Err(error) => {
                warn!(
                    "replacing unreadable password database {}: {error}",
                    self.path.display()
                );
                self.quarantine()?;
                PasswordDatabase::default()
            }
        };

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

    /// 把无法解析的密码库（含版本不受支持的）改名保留。
    ///
    /// 不做这一步就会在保存时直接覆盖它，而且用户刚输入的密码会因为"每次都加载失败"
    /// 而永远保存不下来（设计 §7）。改名失败也不阻止保存：坏文件的内容本来就已经不可用。
    fn quarantine(&self) -> Result<(), String> {
        if !self.path.exists() {
            return Ok(());
        }

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let target = self.path.with_extension(format!("json.corrupt-{stamp}"));
        fs::rename(&self.path, &target).map_err(|error| error.to_string())?;
        let _ = set_private_permissions(&target);
        warn!(
            "kept the unreadable password database as {}",
            target.display()
        );
        Ok(())
    }

    /// 读取不需要加锁：写入是原子的，读到的一定是某个完整版本。
    ///
    /// 这里对**磁盘格式**做容错（设计 §7）：`passwords` 元素可以是字符串简写，`version`
    /// 缺失按 1，`uses` / `last_used` 缺失按 0，未知字段忽略。
    fn load(&self) -> Result<PasswordDatabase, String> {
        if !self.path.exists() {
            return Ok(PasswordDatabase::default());
        }

        let reader = BufReader::new(File::open(&self.path).map_err(|error| error.to_string())?);
        let file: PasswordDatabaseFile =
            serde_json::from_reader(reader).map_err(|error| error.to_string())?;
        if file.version != DATABASE_VERSION {
            return Err(format!(
                "unsupported password database version {}",
                file.version
            ));
        }
        Ok(file.into())
    }

    /// 写入是原子的（临时文件 + 持久化重命名），但**不协调并发写入**：
    ///
    /// 两个 ezz 同时保存时，后写的会覆盖先写的（丢失一次密码记录），但不会产生半截文件。
    /// Windows 从启动到退出持有命名互斥体、macOS 由应用单实例保证，正常只有一个 ezz 在跑；
    /// 只有直接运行 macOS bundle 内的二进制时才可能遇到这种降级，属于已接受的残余。
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

/// 内存中的密码库，也是写回磁盘的规范格式。
#[derive(Serialize)]
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

#[derive(Serialize)]
struct PasswordRecord {
    password: String,
    uses: u64,
    last_used: u64,
}

/// 磁盘上的密码库：所有字段都容忍缺失，便于手工编辑。
#[derive(Deserialize)]
struct PasswordDatabaseFile {
    #[serde(default = "database_version")]
    version: u32,
    #[serde(default)]
    passwords: Vec<PasswordEntry>,
}

fn database_version() -> u32 {
    DATABASE_VERSION
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PasswordEntry {
    /// 字符串简写：等价于 `uses = 0`、`last_used = 0`。
    Simple(String),
    Detailed {
        password: String,
        #[serde(default)]
        uses: u64,
        #[serde(default)]
        last_used: u64,
    },
}

impl From<PasswordDatabaseFile> for PasswordDatabase {
    fn from(file: PasswordDatabaseFile) -> Self {
        Self {
            version: file.version,
            passwords: file
                .passwords
                .into_iter()
                .map(|entry| match entry {
                    PasswordEntry::Simple(password) => PasswordRecord {
                        password,
                        uses: 0,
                        last_used: 0,
                    },
                    PasswordEntry::Detailed {
                        password,
                        uses,
                        last_used,
                    } => PasswordRecord {
                        password,
                        uses,
                        last_used,
                    },
                })
                .collect(),
        }
    }
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

    let username = std::env::var_os("USERNAME")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "USERNAME is not set"))?;
    let mut account = OsString::new();
    if let Some(domain) = std::env::var_os("USERDOMAIN") {
        account.push(domain);
        account.push("\\");
    }
    account.push(username);
    account.push(":F");

    // 必须接管子进程输出：`icacls` 会把本地化的结果写进 stdout，让它继承宿主的标准输出
    // 会在控制台里输出乱码（OEM 代码页）。（设计 §12：子进程输出不得写入宿主标准输出。）
    let output = crate::process::command("icacls.exe")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(account)
        .output()?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    Err(std::io::Error::other(if detail.is_empty() {
        format!("icacls exited with {}", output.status)
    } else {
        format!("icacls exited with {}: {detail}", output.status)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(directory: &Path) -> PasswordStore {
        PasswordStore::new(directory.join("passwords.json"))
    }

    fn quarantined_copies(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(directory)
            .expect("read sandbox")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.contains(".corrupt-"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn string_shorthand_entries_are_accepted() {
        let sandbox = tempfile::tempdir().expect("sandbox");
        let store = store_in(sandbox.path());
        fs::write(store.path(), r#"{ "passwords": ["alpha", "beta"] }"#).expect("write");

        assert_eq!(
            store.candidates(),
            vec!["alpha".to_owned(), "beta".to_owned()]
        );
    }

    #[test]
    fn missing_fields_have_defaults_and_unknown_fields_are_ignored() {
        let sandbox = tempfile::tempdir().expect("sandbox");
        let store = store_in(sandbox.path());
        fs::write(
            store.path(),
            r#"{ "note": "ignored", "passwords": [{ "password": "gamma" }] }"#,
        )
        .expect("write");

        assert_eq!(store.candidates(), vec!["gamma".to_owned()]);
    }

    #[test]
    fn unreadable_database_yields_no_candidates_instead_of_failing() {
        let sandbox = tempfile::tempdir().expect("sandbox");
        let store = store_in(sandbox.path());
        fs::write(store.path(), "definitely not json").expect("write");

        assert!(store.candidates().is_empty());
    }

    #[test]
    fn unsupported_version_is_treated_as_unreadable() {
        let sandbox = tempfile::tempdir().expect("sandbox");
        let store = store_in(sandbox.path());
        fs::write(store.path(), r#"{ "version": 99, "passwords": ["alpha"] }"#).expect("write");

        assert!(store.candidates().is_empty());
    }

    #[test]
    fn saving_after_an_unreadable_database_keeps_the_original_and_writes_the_new_one() {
        let sandbox = tempfile::tempdir().expect("sandbox");
        let store = store_in(sandbox.path());
        fs::write(store.path(), "definitely not json").expect("write");

        store.record_success("delta").expect("record password");

        let kept = quarantined_copies(sandbox.path());
        assert_eq!(kept.len(), 1, "expected one quarantined copy: {kept:?}");
        assert_eq!(store.candidates(), vec!["delta".to_owned()]);
        assert_eq!(
            fs::read_to_string(sandbox.path().join(&kept[0])).expect("read quarantined"),
            "definitely not json"
        );
    }
}
