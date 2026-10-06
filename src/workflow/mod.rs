//! 解压工作流：一次调用处理一个输入。
//!
//! 调用方不需要了解 7-Zip 命令行、中间文件、密码顺序、工作目录或目录整理。这里只做编排：
//!
//! - `archive_set`：分卷归档的识别与完整性检查；
//! - `input_format`：普通归档与 Steganographier 的探测；
//! - `output`：结果目录的占用与平台元数据剔除。

use std::fs;
use std::path::{Path, PathBuf};

use log::warn;
use thiserror::Error;

use crate::password_store::PasswordStore;
use crate::seven_zip::{ArchiveScan, ExtractionVerdict, SevenZip};

mod archive_set;
mod input_format;
mod output;

#[cfg(test)]
mod tests;

use archive_set::resolve_archive_set;
use input_format::detect_input_format;
use output::{claim_output_directory, is_empty, remove_platform_metadata};

/// 提取阶段连续报密码错的重试上限：没有上限时，没有任何密码能解开的包会让弹窗无限重现。
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
    /// 7-Zip 以退出码 1，或忽略危险链接的退出码 2 结束：结果已提交，但引擎报了警告。
    EngineWarnings {
        message: String,
    },
    /// 从结果里剔除的平台元数据条目数（`__MACOSX`、`.DS_Store`）。
    PlatformMetadataRemoved {
        removed: usize,
    },
    /// 剔除平台元数据时出错：结果仍然有效，只是没清理干净。
    PlatformMetadataRemovalFailed {
        message: String,
    },
    /// 剔除平台元数据后结果里什么都没剩下。
    EmptyAfterMetadataRemoval,
    /// 引擎报告数据损坏的条目：只报告，7-Zip 写出的内容照旧提交。
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
    fn request_password(&self, previous_attempt_failed: bool) -> Option<PasswordResponse>;
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
        operation: EngineOperation,
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
}

impl ExtractionError {
    /// 通知里用的短分类。
    ///
    /// 通知不贴引擎原文（可能很长、会被系统截断），完整内容在日志里。
    pub fn summary(&self) -> &'static str {
        match self {
            Self::InputNotFound(_) => "Input file not found",
            Self::InputNotFile(_) => "Input is not a file",
            Self::EngineNotFound(_) => "7-Zip executable not found",
            Self::EngineLaunch { .. } => "Could not start 7-Zip",
            Self::EngineFailed { operation, .. } => match operation {
                EngineOperation::Extract => "7-Zip could not extract the archive",
                EngineOperation::List => "7-Zip could not read the archive",
                EngineOperation::Test => "7-Zip could not verify the password",
                EngineOperation::ScanEmbedded => "7-Zip could not scan the file",
                EngineOperation::ExtractEmbedded => "7-Zip could not extract the embedded archive",
            },
            Self::UnsupportedInput(_) => "Not a supported archive",
            Self::MissingVolume(_) => "Archive volume is missing",
            Self::WrongPassword => "Wrong password",
            Self::PasswordRequired(_) => "No password provided",
            Self::FileSystem { .. } => "File system error",
        }
    }
}

/// 引擎调用的种类：通知文案按它区分，新增种类时编译器会要求补齐 `summary`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineOperation {
    Extract,
    List,
    Test,
    ScanEmbedded,
    ExtractEmbedded,
}

impl std::fmt::Display for EngineOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Extract => "extract",
            Self::List => "list",
            Self::Test => "test",
            Self::ScanEmbedded => "scan embedded data in",
            Self::ExtractEmbedded => "extract embedded archive from",
        })
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

        let parent = input.parent().ok_or_else(|| ExtractionError::FileSystem {
            operation: "resolve parent of",
            path: input.to_path_buf(),
            message: "input has no parent directory".to_owned(),
        })?;
        // 内嵌归档的临时目录要活到解压结束，所以绑在名字上而不是 `_`。
        let (archive_input, scan, _scratch) = detect_input_format(&seven_zip, input, parent)?;
        let mut password =
            self.resolve_password(&seven_zip, &archive_input, &scan, &selected_input)?;

        // 先把结果目录占下来：名字冲突在引擎开始写之前就解决了。
        let output = claim_output_directory(parent, &archive_set.output_stem)?;
        let verdict = match self.extract_with_retries(
            &seven_zip,
            &archive_input,
            &output,
            &scan,
            &mut password,
            &selected_input,
        ) {
            Ok(verdict) => verdict,
            // 失败时不留下半成品：这个目录是刚建的，还没有提交。
            Err(error) => {
                let _ = fs::remove_dir_all(&output);
                return Err(error);
            }
        };

        let mut warnings = Vec::new();
        let removed_metadata = match remove_platform_metadata(&output) {
            Ok(0) => false,
            Ok(removed) => {
                warnings.push(ExtractionWarning::PlatformMetadataRemoved { removed });
                true
            }
            Err(error) => {
                warnings.push(ExtractionWarning::PlatformMetadataRemovalFailed {
                    message: error.to_string(),
                });
                false
            }
        };
        if removed_metadata && is_empty(&output)? {
            warnings.push(ExtractionWarning::EmptyAfterMetadataRemoval);
        }
        if let Some(message) = verdict.engine_warning {
            warnings.push(ExtractionWarning::EngineWarnings { message });
        }
        if !verdict.failed_entries.is_empty() {
            warnings.push(ExtractionWarning::FailedEntries {
                entries: verdict.failed_entries,
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
        let sources = archive_set.sources;
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

    /// 解压到已经占下的结果目录，密码错误时重新要密码。
    ///
    /// 失败的结果目录由调用方清理（要么删掉，要么是在重试前由这里清空）。
    fn extract_with_retries(
        &self,
        seven_zip: &SevenZip,
        archive_input: &Path,
        output: &Path,
        scan: &ArchiveScan,
        password: &mut ResolvedPassword,
        selected_input: &Path,
    ) -> Result<ExtractionVerdict, ExtractionError> {
        let mut retries = 0;
        loop {
            match seven_zip.extract(archive_input, output, &password.value) {
                Ok(verdict) => return Ok(verdict),
                // 混合加密归档：归一化为密码错误并重新弹窗。
                Err(ExtractionError::WrongPassword) => {
                    retries += 1;
                    if retries > MAX_PASSWORD_RETRIES {
                        warn!(
                            "gave up after {retries} password attempts while extracting {}",
                            archive_input.display()
                        );
                        return Err(ExtractionError::WrongPassword);
                    }
                    // 上一次尝试可能已经写进去了一些东西：清空，否则它会留在最终结果里。
                    let _ = fs::remove_dir_all(output);
                    fs::create_dir(output).map_err(|error| {
                        file_system_error("recreate result directory", output, error)
                    })?;
                    *password = self.prompt_for_password(
                        seven_zip,
                        archive_input,
                        scan,
                        selected_input,
                        true,
                    )?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// 决定本次要用哪个密码。
    ///
    /// 校验最小化：`scan` 判定不需要密码时不校验；需要校验时，表头加密用一次列表（只解表头），
    /// 内容加密只用采样条目测试。
    fn resolve_password(
        &self,
        seven_zip: &SevenZip,
        archive_input: &Path,
        scan: &ArchiveScan,
        selected_input: &Path,
    ) -> Result<ResolvedPassword, ExtractionError> {
        if !scan.encrypted {
            return Ok(ResolvedPassword::empty());
        }

        if let Some(store) = &self.password_store {
            // 读取失败不会让输入失败：`candidates` 已记录警告并回退为空候选。
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

        self.prompt_for_password(seven_zip, archive_input, scan, selected_input, false)
    }

    /// 弹窗取密码并校验；`previous_attempt_failed` 控制提示文案。
    fn prompt_for_password(
        &self,
        seven_zip: &SevenZip,
        archive_input: &Path,
        scan: &ArchiveScan,
        selected_input: &Path,
        mut previous_attempt_failed: bool,
    ) -> Result<ResolvedPassword, ExtractionError> {
        loop {
            let Some(response) = self
                .password_prompt
                .request_password(previous_attempt_failed)
            else {
                return Err(ExtractionError::PasswordRequired(
                    selected_input.to_path_buf(),
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

/// 最小化密码校验。返回 `Ok(false)` 表示密码不对。
///
/// - 表头加密：用一次列表验证（只解表头）；
/// - 内容加密：只测试采样条目，不跑整包 `t`；
/// - 没有可用样本（空归档）：交给提取阶段判定，由重试循环兜底。
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
    fn request_password(&self, _previous_attempt_failed: bool) -> Option<PasswordResponse> {
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
        // 移入回收站不触发 shell 变更通知，不刷新会导致目录里的图标残留。
        crate::explorer::refresh_parents(sources);
        Ok(())
    }
}
