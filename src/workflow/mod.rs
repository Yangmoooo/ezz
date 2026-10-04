//! 解压工作流：一个输入，一次完整的行为契约（设计 §5、§9）。
//!
//! 调用方（两个桌面适配器）不应了解 7-Zip 命令行、特殊中间文件、密码排序、工作目录或
//! 目录整理细节。这个模块负责编排，细节分散在几个子模块里：
//!
//! - `archive_set`：分卷归档的识别与完整性检查（§6.3）；
//! - `input_format`：普通归档与 Steganographier 的探测（§6.1、§6.2）；
//! - `commit`：事务式提交、命名冲突、平台元数据（§5.1–§5.3）；
//! - `safety`：不可信条目的判据、不安全条目的丢弃、逃逸不变量（§5.4）。

use std::fs;
use std::path::{Path, PathBuf};

use log::warn;
use thiserror::Error;

use crate::password_store::PasswordStore;
use crate::seven_zip::{ArchiveScan, SevenZip};

mod archive_set;
mod commit;
mod input_format;
pub(crate) mod safety;

#[cfg(test)]
mod tests;

use archive_set::resolve_archive_set;
use commit::commit_output;
use input_format::detect_input_format;
use safety::{directory_snapshot, discard_unsafe_entries, validate_escape_invariant};

/// 提取阶段连续报密码错的重试上限。
///
/// 这个循环只服务于混合加密归档（校验通过、提取仍报密码错）。没有上限时，一个
/// “没有任何单一密码能解开的包”会让弹窗无限重现，用户只能靠取消退出。
const MAX_PASSWORD_RETRIES: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionOutcome {
    pub input: PathBuf,
    pub output: PathBuf,
    pub warnings: Vec<ExtractionWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractionWarning {
    SourceCleanupFailed {
        sources: Vec<PathBuf>,
        message: String,
    },
    PasswordStoreUpdateFailed {
        path: PathBuf,
        message: String,
    },
    /// 7-Zip 以退出码 1（Warning）结束：结果已提交，但引擎报了警告（§5.1）。
    EngineWarnings {
        message: String,
    },
    /// 被丢弃或消毒的条目（§5.4）。
    ///
    /// `sanitized`：路径被 7-Zip 重写进工作目录的条目（`..`、绝对路径、盘符），
    /// 以及被降级成普通文件的危险链接 —— 数据保留，但归档本身有问题。
    /// `discarded`：没有进入提交结果的条目（逃逸或无法解析的链接、特殊文件）。
    UnsafeEntriesSkipped {
        discarded: Vec<PathBuf>,
        sanitized: Vec<String>,
    },
    /// 剔除平台元数据后没有任何有效内容：提交的是一个空目录（§5.1 门 3）。
    EmptyAfterMetadataRemoval {
        removed: Vec<String>,
    },
    /// 引擎报告数据损坏的条目：**只报告，不修改**——7-Zip 写出的内容照旧提交（§5.5 层 1）。
    FailedEntries {
        entries: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordResponse {
    pub password: String,
    pub remember: bool,
    pub keep_original: bool,
}

pub trait PasswordPrompt {
    fn request_password(
        &self,
        input: &Path,
        previous_attempt_failed: bool,
    ) -> Option<PasswordResponse>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExtractionError {
    #[error("Input does not exist: {0}")]
    InputNotFound(PathBuf),

    #[error("Input is not a file: {0}")]
    InputNotFile(PathBuf),

    #[error("7-Zip executable does not exist: {0}")]
    EngineNotFound(PathBuf),

    #[error("Could not start 7-Zip at {path}: {message}")]
    EngineLaunch { path: PathBuf, message: String },

    #[error("7-Zip failed to {operation} with exit code {exit_code:?}: {message}")]
    EngineFailed {
        operation: &'static str,
        exit_code: Option<i32>,
        message: String,
    },

    #[error("Input is not a supported archive: {0}")]
    UnsupportedInput(PathBuf),

    #[error("Archive volume is missing: {0}")]
    MissingVolume(PathBuf),

    #[error("Archive password is incorrect")]
    WrongPassword,

    #[error("Archive password was not provided: {0}")]
    PasswordRequired(PathBuf),

    #[error("Could not {operation} {path}: {message}")]
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        message: String,
    },

    #[error("Unsafe extracted output at {path}: {reason}")]
    UnsafeOutput { path: PathBuf, reason: String },
}

impl ExtractionError {
    /// 通知里用的**短分类**（设计 §3.2）。
    ///
    /// 通知不贴引擎原文：它可能很长、会被系统截断，而截断后的片段既看不懂也不完整。
    /// 完整内容（含输入路径与每条警告）都在日志里，通知只说类型并指向日志。
    pub fn summary(&self) -> &'static str {
        match self {
            Self::InputNotFound(_) => "Input file not found",
            Self::InputNotFile(_) => "Input is not a file",
            Self::EngineNotFound(_) => "7-Zip executable not found",
            Self::EngineLaunch { .. } => "Could not start 7-Zip",
            Self::EngineFailed { operation, .. } => match *operation {
                "extract" => "7-Zip could not extract the archive",
                "list" => "7-Zip could not read the archive",
                "test" => "7-Zip could not verify the password",
                "scan embedded data in" => "7-Zip could not scan the file",
                "extract embedded archive from" => "7-Zip could not extract the embedded archive",
                _ => "7-Zip failed",
            },
            Self::UnsupportedInput(_) => "Not a supported archive",
            Self::MissingVolume(_) => "Archive volume is missing",
            Self::WrongPassword => "Wrong password",
            Self::PasswordRequired(_) => "No password provided",
            Self::FileSystem { .. } => "File system error",
            Self::UnsafeOutput { .. } => "Extraction escaped its workspace",
        }
    }
}

pub struct ExtractionWorkflow {
    seven_zip: PathBuf,
    source_cleaner: Box<dyn SourceCleaner>,
    password_prompt: Box<dyn PasswordPrompt>,
    password_store: Option<PasswordStore>,
}

/// `ExtractionWorkflow::from_parts` 的装配参数：`None` 表示使用生产默认值。
#[cfg(test)]
#[derive(Default)]
struct WorkflowParts {
    seven_zip: PathBuf,
    source_cleaner: Option<Box<dyn SourceCleaner>>,
    password_prompt: Option<Box<dyn PasswordPrompt>>,
    password_store: Option<PathBuf>,
}

impl ExtractionWorkflow {
    pub fn new(seven_zip: impl Into<PathBuf>) -> Self {
        Self {
            seven_zip: seven_zip.into(),
            source_cleaner: Box::new(TrashCleaner),
            password_prompt: Box::new(NoPasswordPrompt),
            password_store: None,
        }
    }

    pub fn with_password_support(
        seven_zip: impl Into<PathBuf>,
        password_store: impl Into<PathBuf>,
        password_prompt: impl PasswordPrompt + 'static,
    ) -> Self {
        Self {
            seven_zip: seven_zip.into(),
            source_cleaner: Box::new(TrashCleaner),
            password_prompt: Box::new(password_prompt),
            password_store: Some(PasswordStore::new(password_store)),
        }
    }

    /// 测试装配点：任意组合三个協作者，未给出的项用生产默认值。
    ///
    /// 生产代码只有 `new`（不支持密码）和 `with_password_support` 两个构造函数；测试需要
    /// 替换協作者才能观察行为，但那些组合不该长在结构体的接口上（§9）。
    #[cfg(test)]
    fn from_parts(parts: WorkflowParts) -> Self {
        Self {
            seven_zip: parts.seven_zip,
            source_cleaner: parts
                .source_cleaner
                .unwrap_or_else(|| Box::new(TrashCleaner)),
            password_prompt: parts
                .password_prompt
                .unwrap_or_else(|| Box::new(NoPasswordPrompt)),
            password_store: parts.password_store.map(PasswordStore::new),
        }
    }

    pub fn extract(&self, input: impl AsRef<Path>) -> Result<ExtractionOutcome, ExtractionError> {
        let input = input.as_ref();
        if !input.exists() {
            return Err(ExtractionError::InputNotFound(input.to_path_buf()));
        }
        if !input.is_file() {
            return Err(ExtractionError::InputNotFile(input.to_path_buf()));
        }
        if !self.seven_zip.is_file() {
            return Err(ExtractionError::EngineNotFound(self.seven_zip.clone()));
        }

        let selected_input = absolute_path(input)?;
        let archive_set = resolve_archive_set(&selected_input)?;
        let input = &archive_set.primary;
        let seven_zip = SevenZip::new(&self.seven_zip);
        let input_format = detect_input_format(&seven_zip, input)?;

        let parent = input.parent().ok_or_else(|| ExtractionError::FileSystem {
            operation: "resolve parent of",
            path: input.to_path_buf(),
            message: "input has no parent directory".to_owned(),
        })?;
        let workspace = tempfile::Builder::new()
            .prefix(".ezz-work-")
            .tempdir_in(parent)
            .map_err(|error| file_system_error("create workspace for", input, error))?;
        let extracted = workspace.path().join("extracted");
        fs::create_dir(&extracted)
            .map_err(|error| file_system_error("create extraction directory", &extracted, error))?;

        let prepared = workspace.path().join("prepared");
        let (archive_input, detected_scan) = input_format.prepare(&seven_zip, input, &prepared)?;

        // 探测阶段已经为同一个文件做过无密码扫描（R4）；只有特殊格式（扫描发生在刚
        // 释放出的内嵌归档上）才需要补一次。这一次扫描同时完成了条目路径校验。
        let scan = match detected_scan {
            Some(scan) => scan,
            None => seven_zip.scan(&archive_input, "")?,
        };
        let mut password =
            self.resolve_password(&seven_zip, &archive_input, &scan, &selected_input)?;

        // 逃逸不变量（§5.4）：解压前后比较归档所在目录的条目快照。工作目录本身已经存在，
        // 快照时把它排除掉，否则它自己的修改时间会被当成逃逸。不依赖对 7-Zip 消毒规则的信任。
        let snapshot = directory_snapshot(parent, workspace.path())?;

        let mut retries = 0;
        let verdict = loop {
            match seven_zip.extract(&archive_input, &extracted, &password.value) {
                Ok(verdict) => break verdict,
                // 校验阶段通过而提取仍报密码错（混合加密归档）：归一化为密码错误并重新弹窗（D2/R4）。
                Err(ExtractionError::WrongPassword) => {
                    retries += 1;
                    if retries > MAX_PASSWORD_RETRIES {
                        warn!(
                            "gave up after {retries} password attempts while extracting {}",
                            archive_input.display()
                        );
                        return Err(ExtractionError::WrongPassword);
                    }
                    password = self.prompt_for_password(
                        &seven_zip,
                        &archive_input,
                        &scan,
                        &selected_input,
                        true,
                    )?;
                }
                Err(error) => return Err(error),
            }
        };

        validate_escape_invariant(parent, workspace.path(), &snapshot)?;
        let discarded = discard_unsafe_entries(&extracted)?;
        let commit = commit_output(input, &extracted, &archive_set.output_stem)?;
        let output = commit.path;
        let sources = archive_set.sources;
        let mut warnings = Vec::new();

        let mut sanitized = scan.sanitized.clone();
        sanitized.extend(verdict.sanitized_links);
        if !discarded.is_empty() || !sanitized.is_empty() {
            warnings.push(ExtractionWarning::UnsafeEntriesSkipped {
                discarded,
                sanitized,
            });
        }
        if let Some(message) = verdict.engine_warning {
            warnings.push(ExtractionWarning::EngineWarnings { message });
        }
        if !verdict.failed_entries.is_empty() {
            warnings.push(ExtractionWarning::FailedEntries {
                entries: verdict.failed_entries,
            });
        }
        if commit.empty {
            warnings.push(ExtractionWarning::EmptyAfterMetadataRemoval {
                removed: commit.removed_metadata,
            });
        }
        if password.remember
            && !password.value.is_empty()
            && let Some(store) = &self.password_store
            && let Err(message) = store.record_success(&password.value)
        {
            warnings.push(ExtractionWarning::PasswordStoreUpdateFailed {
                path: store.path().to_path_buf(),
                message,
            });
        }
        if !password.keep_original
            && let Some(message) = self.source_cleaner.clean(&sources).err()
        {
            warnings.push(ExtractionWarning::SourceCleanupFailed { sources, message });
        }

        Ok(ExtractionOutcome {
            input: selected_input,
            output,
            warnings,
        })
    }

    /// 决定本次要用哪个密码（§5.1 第 5 步）。
    ///
    /// 校验必须最小化（R4）：`scan` 已经判定不需要密码时**不得校验**；需要校验时，表头
    /// 加密用一次列表（只解表头），内容加密只用最小条目测试（只解一个条目）。
    fn resolve_password(
        &self,
        seven_zip: &SevenZip,
        archive_input: &Path,
        scan: &ArchiveScan,
        prompt_input: &Path,
    ) -> Result<ResolvedPassword, ExtractionError> {
        if !scan.encrypted {
            return Ok(ResolvedPassword::empty());
        }

        if let Some(store) = &self.password_store {
            // 读取失败不会让输入失败：`candidates` 内部已记录警告并回退为空候选（§7）。
            for password in store.candidates() {
                if validate_password(seven_zip, archive_input, scan, &password)? {
                    return Ok(ResolvedPassword {
                        value: password,
                        remember: true,
                        keep_original: false,
                    });
                }
            }
        }

        self.prompt_for_password(seven_zip, archive_input, scan, prompt_input, false)
    }

    /// 弹窗取密码并校验；`previous_attempt_failed` 控制提示文案。
    fn prompt_for_password(
        &self,
        seven_zip: &SevenZip,
        archive_input: &Path,
        scan: &ArchiveScan,
        prompt_input: &Path,
        mut previous_attempt_failed: bool,
    ) -> Result<ResolvedPassword, ExtractionError> {
        loop {
            let Some(response) = self
                .password_prompt
                .request_password(prompt_input, previous_attempt_failed)
            else {
                return Err(ExtractionError::PasswordRequired(
                    prompt_input.to_path_buf(),
                ));
            };

            if validate_password(seven_zip, archive_input, scan, &response.password)? {
                return Ok(ResolvedPassword {
                    value: response.password,
                    remember: response.remember,
                    keep_original: response.keep_original,
                });
            }
            previous_attempt_failed = true;
        }
    }
}

/// 最小化密码校验（R4/D2）。返回 `Ok(false)` 表示密码不对。
///
/// - 表头加密：用一次列表验证（只解表头），顺带完成“拿到密码才能看见”的条目路径校验。
/// - 内容加密：只测试采样条目（只解一个条目），不跑整包 `t`。
/// - 没有可用样本（空归档）：无法最小化校验，交给提取阶段判定，由重试循环兜底。
fn validate_password(
    seven_zip: &SevenZip,
    archive_input: &Path,
    scan: &ArchiveScan,
    password: &str,
) -> Result<bool, ExtractionError> {
    let outcome = if scan.header_encrypted {
        seven_zip.scan(archive_input, password).map(|_| ())
    } else {
        match scan.sample_entry.as_deref() {
            Some(entry) => seven_zip.test_password(archive_input, password, Some(entry)),
            None => return Ok(true),
        }
    };

    match outcome {
        Ok(()) => Ok(true),
        Err(ExtractionError::WrongPassword) => Ok(false),
        Err(error) => Err(error),
    }
}

struct ResolvedPassword {
    value: String,
    remember: bool,
    keep_original: bool,
}

impl ResolvedPassword {
    fn empty() -> Self {
        Self {
            value: String::new(),
            remember: false,
            keep_original: false,
        }
    }
}

struct NoPasswordPrompt;

impl PasswordPrompt for NoPasswordPrompt {
    fn request_password(
        &self,
        _input: &Path,
        _previous_attempt_failed: bool,
    ) -> Option<PasswordResponse> {
        None
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, ExtractionError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| file_system_error("resolve absolute path for", path, error))
}

fn file_system_error(
    operation: &'static str,
    path: &Path,
    error: std::io::Error,
) -> ExtractionError {
    ExtractionError::FileSystem {
        operation,
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

trait SourceCleaner {
    fn clean(&self, sources: &[PathBuf]) -> Result<(), String>;
}

struct TrashCleaner;

impl SourceCleaner for TrashCleaner {
    fn clean(&self, sources: &[PathBuf]) -> Result<(), String> {
        trash::delete_all(sources).map_err(|error| error.to_string())?;
        // 移入回收站不会自动触发 shell 变更通知，否则目录里的图标会残留到手动刷新（§11.1）。
        crate::explorer::refresh_parents(sources);
        Ok(())
    }
}
