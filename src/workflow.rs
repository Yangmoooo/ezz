use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use log::warn;
use thiserror::Error;

use crate::password_store::PasswordStore;
use crate::seven_zip::{ArchiveScan, SevenZip};

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

enum DetectedInputFormat {
    /// 常规归档：`scan` 是探测阶段已经做过的**无密码**扫描，直接复用（R4）。
    RegularArchive {
        scan: ArchiveScan,
    },
    Steganographier {
        embedded: PathBuf,
    },
}

impl DetectedInputFormat {
    /// 准备实际要解压的输入，并带回一个已经做过的扫描（没有则为 `None`）。
    fn prepare(
        self,
        seven_zip: &SevenZip,
        input: &Path,
        prepared: &Path,
    ) -> Result<(PathBuf, Option<ArchiveScan>), ExtractionError> {
        match self {
            Self::RegularArchive { scan } => Ok((input.to_path_buf(), Some(scan))),
            Self::Steganographier { embedded } => {
                fs::create_dir(prepared).map_err(|error| {
                    file_system_error("create special-format workspace", prepared, error)
                })?;
                let archive = seven_zip.extract_embedded_archive(input, prepared, &embedded)?;
                // 内嵌归档是 `-t#` 直接拷贝出来的单个文件，理论上不会出现链接或特殊文件；
                // 仍然过一遍丢弃检查（结果忽略：这一步的失败由后面的 `is_file` 捕获）。
                let _ = discard_unsafe_entries(prepared)?;
                if !archive.is_file() {
                    return Err(ExtractionError::UnsupportedInput(input.to_path_buf()));
                }
                match seven_zip.scan(&archive, "") {
                    Ok(scan) => Ok((archive, Some(scan))),
                    Err(_) => Err(ExtractionError::UnsupportedInput(input.to_path_buf())),
                }
            }
        }
    }
}

/// 探测输入格式：先试 Steganographier（`-t#`），再按普通归档扫描（设计 §6.2）。
///
/// 两个探测函数就是全部格式集合：`SevenZip` 只有一个实现，不为假设中的第三种格式保留一个
/// trait（设计 §9：不得为假设中的扩展扩大接口）。
fn detect_input_format(
    seven_zip: &SevenZip,
    input: &Path,
) -> Result<DetectedInputFormat, ExtractionError> {
    if let Some(format) = detect_steganographier(seven_zip, input)? {
        return Ok(format);
    }
    if let Some(format) = detect_regular_archive(seven_zip, input)? {
        return Ok(format);
    }

    Err(ExtractionError::UnsupportedInput(input.to_path_buf()))
}

/// 视频文件里内嵌的归档（`-t#`）。不是视频、或找不到受支持的内嵌归档时返回 `None`。
fn detect_steganographier(
    seven_zip: &SevenZip,
    input: &Path,
) -> Result<Option<DetectedInputFormat>, ExtractionError> {
    let is_video = input
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("mp4") || extension.eq_ignore_ascii_case("mkv")
        });
    if !is_video {
        return Ok(None);
    }

    seven_zip
        .embedded_archive(input)
        .map(|embedded| embedded.map(|embedded| DetectedInputFormat::Steganographier { embedded }))
}

/// 普通归档：一次无密码扫描同时决定“这是不是归档”和“要不要密码”（R4）。
fn detect_regular_archive(
    seven_zip: &SevenZip,
    input: &Path,
) -> Result<Option<DetectedInputFormat>, ExtractionError> {
    match seven_zip.scan(input, "") {
        Ok(scan) => Ok(Some(DetectedInputFormat::RegularArchive { scan })),
        Err(ExtractionError::UnsupportedInput(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

struct ArchiveSet {
    primary: PathBuf,
    sources: Vec<PathBuf>,
    output_stem: OsString,
}

/// 同一逻辑归档的卷：序号 → 路径（按键有序，缺号检查依赖这一点）。
type VolumeSet = BTreeMap<u32, PathBuf>;

/// 扫一遍归档所在目录，按 `sequence_of` 挑出属于同一个逻辑归档的卷（设计 §6.3）。
///
/// `sequence_of` 返回序号即收录该文件，返回 `None` 表示与本次输入无关。三个分卷家族
/// （`.001`、`.partN.rar`、`.z01`+`.zip`）只在这个闭包里不同。
fn scan_volumes(
    parent: &Path,
    mut sequence_of: impl FnMut(&Path) -> Option<u32>,
) -> Result<VolumeSet, ExtractionError> {
    let mut volumes = VolumeSet::new();
    let entries = fs::read_dir(parent)
        .map_err(|error| file_system_error("scan archive volumes in", parent, error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| file_system_error("scan archive volume in", parent, error))?;
        let path = entry.path();
        if let Some(sequence) = sequence_of(&path) {
            volumes.insert(sequence, path);
        }
    }
    Ok(volumes)
}

/// 1 到最高序号之间不得缺号：缺号是致命失败（`MissingVolume`）。
///
/// `selected` 是用户实际点中的那一卷：目录里只剩它自己时，缺号检查也必须以它为最高序号。
fn require_contiguous(
    volumes: &VolumeSet,
    selected: u32,
    missing: impl Fn(u32) -> PathBuf,
) -> Result<(), ExtractionError> {
    let last = volumes.keys().next_back().copied().unwrap_or(selected);
    for number in 1..=last {
        if !volumes.contains_key(&number) {
            return Err(ExtractionError::MissingVolume(missing(number)));
        }
    }
    Ok(())
}

fn resolve_archive_set(selected: &Path) -> Result<ArchiveSet, ExtractionError> {
    if let Some(sequence) = numeric_extension(selected) {
        return resolve_numeric_archive_set(selected, sequence);
    }
    if let Some(volume) = rar_volume_name(selected) {
        return resolve_rar_archive_set(selected, &volume);
    }
    if let Some(sequence) = zip_volume_sequence(selected) {
        return resolve_zip_archive_set(selected, Some(sequence));
    }
    if has_zip_extension(selected) {
        return resolve_zip_archive_set(selected, None);
    }

    Ok(ArchiveSet {
        primary: selected.to_path_buf(),
        sources: vec![selected.to_path_buf()],
        output_stem: archive_stem(selected),
    })
}

fn resolve_numeric_archive_set(
    selected: &Path,
    sequence: u32,
) -> Result<ArchiveSet, ExtractionError> {
    let first = selected.with_extension("001");
    if !first.is_file() {
        return Err(ExtractionError::MissingVolume(first));
    }

    let parent = selected.parent().expect("absolute input parent");
    let prefix = selected.file_stem().expect("volume file stem");
    let volumes = scan_volumes(parent, |path| {
        if path.file_stem() == Some(prefix) {
            numeric_extension(path)
        } else {
            None
        }
    })?;
    require_contiguous(&volumes, sequence, |number| {
        selected.with_extension(format!("{number:03}"))
    })?;

    Ok(ArchiveSet {
        primary: first,
        sources: volumes.into_values().collect(),
        output_stem: archive_stem(&selected.with_extension("")),
    })
}

struct RarVolumeName {
    prefix: String,
    sequence: u32,
    width: usize,
    extension: String,
}

fn resolve_rar_archive_set(
    selected: &Path,
    selected_volume: &RarVolumeName,
) -> Result<ArchiveSet, ExtractionError> {
    let parent = selected.parent().expect("absolute input parent");
    let volumes = scan_volumes(parent, |path| {
        rar_volume_name(path)
            .filter(|volume| {
                volume.prefix == selected_volume.prefix
                    && volume
                        .extension
                        .eq_ignore_ascii_case(&selected_volume.extension)
            })
            .map(|volume| volume.sequence)
    })?;
    require_contiguous(&volumes, selected_volume.sequence, |number| {
        rar_volume_path(parent, selected_volume, number)
    })?;

    Ok(ArchiveSet {
        primary: volumes.get(&1).expect("first RAR volume checked").clone(),
        sources: volumes.into_values().collect(),
        output_stem: OsString::from(&selected_volume.prefix),
    })
}

fn rar_volume_name(path: &Path) -> Option<RarVolumeName> {
    let name = path.file_name()?.to_str()?;
    let bytes = name.as_bytes();
    if bytes.len() < 10 || !bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".rar") {
        return None;
    }
    let part = bytes[..bytes.len() - 4]
        .windows(5)
        .rposition(|window| window.eq_ignore_ascii_case(b".part"))?;
    let digits = &name[part + 5..name.len() - 4];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    Some(RarVolumeName {
        prefix: name[..part].to_owned(),
        sequence: digits.parse().ok()?,
        width: digits.len(),
        extension: name[name.len() - 3..].to_owned(),
    })
}

fn rar_volume_path(parent: &Path, volume: &RarVolumeName, sequence: u32) -> PathBuf {
    parent.join(format!(
        "{}.part{:0width$}.{}",
        volume.prefix,
        sequence,
        volume.extension,
        width = volume.width
    ))
}

fn resolve_zip_archive_set(
    selected: &Path,
    selected_sequence: Option<u32>,
) -> Result<ArchiveSet, ExtractionError> {
    let parent = selected.parent().expect("absolute input parent");
    let stem = selected.file_stem().expect("volume file stem");
    // 编号卷（`.z01`…）与末尾的无编号 `.zip` 是两类：后者不带序号，单独收集。
    let mut final_volume = None;
    let volumes = scan_volumes(parent, |path| {
        if path.file_stem() != Some(stem) {
            return None;
        }
        if let Some(sequence) = zip_volume_sequence(path) {
            return Some(sequence);
        }
        if has_zip_extension(path) {
            final_volume = Some(path.to_path_buf());
        }
        None
    })?;

    // 单个 `.zip`：目录里没有编号卷，选中的就是它自己。
    if selected_sequence.is_none() && volumes.is_empty() {
        return Ok(ArchiveSet {
            primary: selected.to_path_buf(),
            sources: vec![selected.to_path_buf()],
            output_stem: archive_stem(selected),
        });
    }

    let Some(final_volume) = final_volume else {
        return Err(ExtractionError::MissingVolume(
            selected.with_extension("zip"),
        ));
    };
    require_contiguous(&volumes, selected_sequence.unwrap_or(0), |number| {
        selected.with_extension(format!("z{number:02}"))
    })?;

    let mut sources: Vec<_> = volumes.into_values().collect();
    sources.push(final_volume.clone());
    Ok(ArchiveSet {
        primary: final_volume,
        sources,
        output_stem: archive_stem(selected),
    })
}

fn archive_stem(path: &Path) -> OsString {
    path.file_stem()
        .unwrap_or_else(|| OsStr::new("archive"))
        .to_os_string()
}

fn zip_volume_sequence(path: &Path) -> Option<u32> {
    let extension = path.extension()?.to_str()?;
    let bytes = extension.as_bytes();
    (bytes.len() == 3
        && matches!(bytes[0], b'z' | b'Z')
        && bytes[1..].iter().all(u8::is_ascii_digit))
    .then(|| extension[1..].parse().ok())
    .flatten()
}

fn has_zip_extension(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn numeric_extension(path: &Path) -> Option<u32> {
    let extension = path.extension()?.to_str()?;
    (extension.len() == 3 && extension.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| extension.parse().ok())
        .flatten()
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

/// 丢弃不安全的条目并报告（设计 §5.4）。
///
/// 不安全条目**不得**让整个输入失败：这里删除它们并把相对路径交给调用方，由调用方记入
/// 结构化警告。判据：
///
/// - 符号链接：解析不到目标，或解析后离开工作目录 → 删除（无法验证的链接一律不信）。
/// - 符号链接：目标是绝对路径 → 删除（工作目录是临时的，提交后必然是死链）。
/// - 特殊文件（设备、FIFO、socket 等）→ 删除。
fn discard_unsafe_entries(root: &Path) -> Result<Vec<PathBuf>, ExtractionError> {
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| file_system_error("resolve extraction directory", root, error))?;
    let mut directories = vec![root.to_path_buf()];
    let mut discarded = Vec::new();

    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| file_system_error("inspect extracted directory", &directory, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                file_system_error("inspect extracted entry in", &directory, error)
            })?;
            let path = entry.path();
            // 用 `DirEntry::file_type` 而不是 `fs::symlink_metadata`：前者在 Windows 上直接来自
            // `read_dir` 已经拿到的属性（零系统调用），在 Unix 上通常来自 `d_type`；两者都不跟
            // 跟踪链接，语义一致。实测（5000 个文件）：`symlink_metadata` 每条目 44.5 µs，
            // 换成它之后剩下的只有不可省的目录遍历（8 ms）。
            let file_type = entry
                .file_type()
                .map_err(|error| file_system_error("inspect extracted entry", &path, error))?;

            if file_type.is_symlink() {
                let target = fs::read_link(&path)
                    .map_err(|error| file_system_error("read symbolic link", &path, error))?;
                let escapes = target.is_absolute()
                    || match fs::canonicalize(&path) {
                        Ok(resolved) => !resolved.starts_with(&canonical_root),
                        // 解析不到目标（死链）：可能是 `../..` 拼出来的逃逸，一律不信。
                        Err(_) => true,
                    };
                if escapes {
                    remove_symbolic_link(&path)?;
                    discarded.push(relative_to(root, &path));
                }
            } else if file_type.is_dir() {
                directories.push(path);
            } else if !file_type.is_file() {
                fs::remove_file(&path)
                    .map_err(|error| file_system_error("remove special file", &path, error))?;
                discarded.push(relative_to(root, &path));
            }
        }
    }

    Ok(discarded)
}

/// 删除符号链接：Windows 上目录链接必须用 `remove_dir`。
fn remove_symbolic_link(path: &Path) -> Result<(), ExtractionError> {
    if fs::remove_file(path).is_ok() {
        return Ok(());
    }
    fs::remove_dir(path).map_err(|error| file_system_error("remove symbolic link", path, error))
}

fn relative_to(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// 归档所在目录的条目快照：名称 → 修改时间（设计 §5.4）。
type DirectorySnapshot = Vec<(OsString, Option<SystemTime>)>;

fn directory_snapshot(
    directory: &Path,
    ignore: &Path,
) -> Result<DirectorySnapshot, ExtractionError> {
    let ignored_name = ignore.file_name();
    let mut snapshot = Vec::new();
    let entries = fs::read_dir(directory)
        .map_err(|error| file_system_error("snapshot directory", directory, error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| file_system_error("snapshot entry in", directory, error))?;
        if Some(entry.file_name().as_os_str()) == ignored_name {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        snapshot.push((entry.file_name(), modified));
    }
    snapshot.sort();
    Ok(snapshot)
}

/// 逃逸不变量（设计 §5.4）：解压不得在归档所在目录留下任何新增或改动。
///
/// 发现任何差异就按致命失败处理：回滚工作目录（`TempDir` 的 Drop）且不清理原归档。
fn validate_escape_invariant(
    directory: &Path,
    ignore: &Path,
    before: &DirectorySnapshot,
) -> Result<(), ExtractionError> {
    let after = directory_snapshot(directory, ignore)?;
    if &after == before {
        return Ok(());
    }

    Err(ExtractionError::UnsafeOutput {
        path: directory.to_path_buf(),
        reason: "extraction changed entries outside its workspace".to_owned(),
    })
}

fn absolute_path(path: &Path) -> Result<PathBuf, ExtractionError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| file_system_error("resolve absolute path for", path, error))
}

/// 提交结果（设计 §5.2 / §5.3）。
struct Committed {
    path: PathBuf,
    /// 被剔除的平台元数据条目名。
    removed_metadata: Vec<String>,
    /// 剔除后没有任何有效内容：提交的是一个空目录（§5.1 门 3）。
    empty: bool,
}

fn commit_output(
    input: &Path,
    extracted: &Path,
    output_stem: &OsStr,
) -> Result<Committed, ExtractionError> {
    let removed_metadata = remove_platform_metadata(extracted)?;
    let mut entries = fs::read_dir(extracted)
        .map_err(|error| file_system_error("read extracted contents from", extracted, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| file_system_error("read extracted entry from", extracted, error))?;

    let parent = input.parent().expect("validated input parent");
    match entries.len() {
        // 剔除平台元数据后没有有效内容：输出为空是事实，不是错误（§5.1 门 3）。
        // 提交一个以归档命名的空目录，让结果仍有一个最终实际路径。
        0 => {
            let path = commit_empty_directory(parent, output_stem)?;
            Ok(Committed {
                path,
                removed_metadata,
                empty: true,
            })
        }
        1 => {
            let entry = entries.pop().expect("one extracted entry");
            // 命名规则看条目**类型**：目录用 `name (1)`，文件用 `name (1).ext`（§5.3）。
            let is_directory = entry
                .file_type()
                .map_err(|error| {
                    file_system_error("inspect extracted entry", &entry.path(), error)
                })?
                .is_dir();
            let kind = if is_directory {
                CommitKind::Directory
            } else {
                CommitKind::File
            };
            let path = commit_with_unique_name(&entry.path(), parent, &entry.file_name(), kind)?;
            Ok(Committed {
                path,
                removed_metadata,
                empty: false,
            })
        }
        _ => {
            let path =
                commit_with_unique_name(extracted, parent, output_stem, CommitKind::Directory)?;
            Ok(Committed {
                path,
                removed_metadata,
                empty: false,
            })
        }
    }
}

/// 提交一个空目录：没有内容要搬，直接把名字占下来（§5.1 门 3）。
fn commit_empty_directory(parent: &Path, name: &OsStr) -> Result<PathBuf, ExtractionError> {
    for candidate in unique_destination_candidates(parent, name, CommitKind::Directory) {
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(file_system_error(
                    "create empty result directory",
                    &candidate,
                    error,
                ));
            }
        }
    }

    unreachable!("u64 destination sequence exhausted")
}

/// 剔除平台元数据（§5.2），返回被剔除的条目名。
fn remove_platform_metadata(extracted: &Path) -> Result<Vec<String>, ExtractionError> {
    let mut removed = Vec::new();
    for name in ["__MACOSX", ".DS_Store"] {
        let path = extracted.join(name);
        if path.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|error| file_system_error("remove platform metadata", &path, error))?;
            removed.push(name.to_owned());
        } else if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| file_system_error("remove platform metadata", &path, error))?;
            removed.push(name.to_owned());
        }
    }
    Ok(removed)
}

#[derive(Clone, Copy)]
enum CommitKind {
    File,
    Directory,
}

/// 把 `source` 提交为 `parent` 下的一个不冲突名字（§5.3）。
///
/// 候选名字按 `name`, `name (1)`, `name (2)` … 递增。若在探测与重命名之间被抢先
/// （Windows 上互斥体已排除跨进程并发；只剩直接运行 macOS bundle 内的二进制这条
/// 开发者路径），就继续递增序号重试，**绝不覆盖既有条目**。
fn commit_with_unique_name(
    source: &Path,
    parent: &Path,
    name: &OsStr,
    kind: CommitKind,
) -> Result<PathBuf, ExtractionError> {
    for candidate in unique_destination_candidates(parent, name, kind) {
        if candidate.exists() {
            continue;
        }

        match fs::rename(source, &candidate) {
            Ok(()) => return Ok(candidate),
            // 竞态：候选名字在探测之后被占用。换下一个序号。
            Err(_) if candidate.exists() => continue,
            Err(error) => {
                return Err(file_system_error(
                    "commit extracted output to",
                    &candidate,
                    error,
                ));
            }
        }
    }

    unreachable!("u64 destination sequence exhausted")
}

/// 生成 `name`, `name (1)`, `name (2)` … 的候选目的地。
///
/// 文件保留扩展名（`archive (1).zip`），目录整体递增（`archive.zip (1)`）。
fn unique_destination_candidates<'a>(
    parent: &'a Path,
    name: &'a OsStr,
    kind: CommitKind,
) -> impl Iterator<Item = PathBuf> + 'a {
    let (stem, extension) = match kind {
        CommitKind::File => {
            let name_path = Path::new(name);
            let stem = name_path.file_stem().unwrap_or(name).to_os_string();
            let extension = name_path.extension().map(OsString::from);
            (stem, extension)
        }
        CommitKind::Directory => (name.to_os_string(), None),
    };

    std::iter::once(parent.join(name)).chain((1_u64..).map(move |sequence| {
        let mut candidate = stem.clone();
        candidate.push(format!(" ({sequence})"));
        if let Some(extension) = &extension {
            candidate.push(".");
            candidate.push(extension);
        }
        parent.join(candidate)
    }))
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::Write;
    use std::process::Command;
    use std::sync::Mutex;

    use super::*;

    struct RemoveSource;

    impl SourceCleaner for RemoveSource {
        fn clean(&self, sources: &[PathBuf]) -> Result<(), String> {
            for source in sources {
                std::fs::remove_file(source).map_err(|error| error.to_string())?;
            }
            Ok(())
        }
    }

    struct FailingSourceCleaner;

    impl SourceCleaner for FailingSourceCleaner {
        fn clean(&self, _sources: &[PathBuf]) -> Result<(), String> {
            Err("cleanup unavailable".to_owned())
        }
    }

    struct ScriptedPasswordPrompt {
        responses: Mutex<VecDeque<PasswordResponse>>,
    }

    impl ScriptedPasswordPrompt {
        fn new(responses: impl IntoIterator<Item = PasswordResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
            }
        }
    }

    impl PasswordPrompt for ScriptedPasswordPrompt {
        fn request_password(
            &self,
            _input: &Path,
            _previous_attempt_failed: bool,
        ) -> Option<PasswordResponse> {
            self.responses.lock().unwrap().pop_front()
        }
    }

    struct NoResponsePrompt;

    impl PasswordPrompt for NoResponsePrompt {
        fn request_password(
            &self,
            _input: &Path,
            _previous_attempt_failed: bool,
        ) -> Option<PasswordResponse> {
            None
        }
    }

    /// 大多数用例只需要替换清理器：`RemoveSource` 直接删文件（不发回收站），也不弹密码。
    fn workflow(seven_zip: impl Into<PathBuf>) -> ExtractionWorkflow {
        workflow_with_cleaner(seven_zip, RemoveSource)
    }

    /// 替换清理器（例如让它失败，观察 `SourceCleanupFailed` 警告）。
    fn workflow_with_cleaner(
        seven_zip: impl Into<PathBuf>,
        source_cleaner: impl SourceCleaner + 'static,
    ) -> ExtractionWorkflow {
        ExtractionWorkflow::from_parts(WorkflowParts {
            seven_zip: seven_zip.into(),
            source_cleaner: Some(Box::new(source_cleaner)),
            ..WorkflowParts::default()
        })
    }

    /// 额外替换密码弹窗。
    fn workflow_with(
        seven_zip: impl Into<PathBuf>,
        source_cleaner: impl SourceCleaner + 'static,
        password_prompt: impl PasswordPrompt + 'static,
    ) -> ExtractionWorkflow {
        ExtractionWorkflow::from_parts(WorkflowParts {
            seven_zip: seven_zip.into(),
            source_cleaner: Some(Box::new(source_cleaner)),
            password_prompt: Some(Box::new(password_prompt)),
            ..WorkflowParts::default()
        })
    }

    /// 额外接上密码库。
    fn workflow_with_store(
        seven_zip: impl Into<PathBuf>,
        source_cleaner: impl SourceCleaner + 'static,
        password_prompt: impl PasswordPrompt + 'static,
        password_store: impl Into<PathBuf>,
    ) -> ExtractionWorkflow {
        ExtractionWorkflow::from_parts(WorkflowParts {
            seven_zip: seven_zip.into(),
            source_cleaner: Some(Box::new(source_cleaner)),
            password_prompt: Some(Box::new(password_prompt)),
            password_store: Some(password_store.into()),
        })
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn real_archive_extracts_and_commits_its_single_top_level_file() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("payload.txt");
        let archive = sandbox.path().join("archive.7z");
        std::fs::write(&payload, b"ezz v3 payload").expect("create payload");

        create_archive(&seven_zip, sandbox.path(), &archive, &["payload.txt"]);
        std::fs::remove_file(&payload).expect("remove source payload");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("extract archive");

        assert_eq!(
            outcome,
            ExtractionOutcome {
                input: archive.clone(),
                output: payload.clone(),
                warnings: Vec::new(),
            }
        );
        assert_eq!(
            std::fs::read(&payload).expect("read extracted payload"),
            b"ezz v3 payload"
        );
        assert!(
            !archive.exists(),
            "successful extraction must clean the source"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn cleanup_failure_is_reported_as_a_success_warning() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("payload.txt");
        let archive = sandbox.path().join("archive.7z");
        std::fs::write(&payload, b"ezz v3 payload").expect("create payload");
        create_archive(&seven_zip, sandbox.path(), &archive, &["payload.txt"]);
        std::fs::remove_file(&payload).expect("remove source payload");

        let outcome = workflow_with_cleaner(&seven_zip, FailingSourceCleaner)
            .extract(&archive)
            .expect("cleanup failure must not fail extraction");

        assert_eq!(
            outcome.warnings,
            vec![ExtractionWarning::SourceCleanupFailed {
                sources: vec![archive.clone()],
                message: "cleanup unavailable".to_owned(),
            }]
        );
        assert!(payload.is_file(), "extracted output must stay committed");
        assert!(archive.is_file(), "failed cleanup must preserve the source");
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn damaged_archive_does_not_commit_partial_output_or_clean_the_source() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let first = sandbox.path().join("first.txt");
        let second = sandbox.path().join("second.txt");
        let archive = sandbox.path().join("damaged.7z");
        std::fs::write(&first, vec![b'a'; 4 * 1024]).expect("create first payload");
        std::fs::write(&second, vec![b'b'; 4 * 1024]).expect("create second payload");
        create_archive(
            &seven_zip,
            sandbox.path(),
            &archive,
            &["first.txt", "second.txt"],
        );
        std::fs::remove_file(&first).expect("remove first source payload");
        std::fs::remove_file(&second).expect("remove second source payload");
        let archive_file = std::fs::OpenOptions::new()
            .write(true)
            .open(&archive)
            .expect("open archive for truncation");
        let length = archive_file.metadata().unwrap().len();
        archive_file
            .set_len(length / 2)
            .expect("truncate test archive");

        let result = workflow(&seven_zip).extract(&archive);

        assert!(result.is_err(), "damaged archive must fail");
        assert!(archive.is_file(), "damaged archive must be preserved");
        assert!(
            !first.exists(),
            "partial first output must not be committed"
        );
        assert!(
            !second.exists(),
            "partial second output must not be committed"
        );
        assert_eq!(
            std::fs::read_dir(sandbox.path()).unwrap().count(),
            1,
            "damaged archive must not leave a workspace"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn multiple_top_level_entries_are_committed_in_an_archive_named_directory() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let first = sandbox.path().join("first.txt");
        let second = sandbox.path().join("second.txt");
        let archive = sandbox.path().join("bundle.7z");
        std::fs::write(&first, b"first").expect("create first payload");
        std::fs::write(&second, b"second").expect("create second payload");
        create_archive(
            &seven_zip,
            sandbox.path(),
            &archive,
            &["first.txt", "second.txt"],
        );
        std::fs::remove_file(&first).expect("remove first source payload");
        std::fs::remove_file(&second).expect("remove second source payload");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("extract archive");

        let output = sandbox.path().join("bundle");
        assert_eq!(outcome.output, output);
        assert_eq!(std::fs::read(output.join("first.txt")).unwrap(), b"first");
        assert_eq!(std::fs::read(output.join("second.txt")).unwrap(), b"second");
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn existing_file_is_preserved_and_new_output_gets_a_sequence_suffix() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("payload.txt");
        let archive = sandbox.path().join("archive.7z");
        std::fs::write(&payload, b"new content").expect("create payload");
        create_archive(&seven_zip, sandbox.path(), &archive, &["payload.txt"]);
        std::fs::write(&payload, b"existing content").expect("replace existing payload");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("extract archive without overwriting");

        let sequenced = sandbox.path().join("payload (1).txt");
        assert_eq!(outcome.output, sequenced);
        assert_eq!(std::fs::read(&payload).unwrap(), b"existing content");
        assert_eq!(std::fs::read(&sequenced).unwrap(), b"new content");
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn existing_directory_is_preserved_and_new_output_directory_gets_a_sequence_suffix() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let first = sandbox.path().join("first.txt");
        let second = sandbox.path().join("second.txt");
        let archive = sandbox.path().join("bundle.7z");
        let existing = sandbox.path().join("bundle");
        std::fs::write(&first, b"first").expect("create first payload");
        std::fs::write(&second, b"second").expect("create second payload");
        create_archive(
            &seven_zip,
            sandbox.path(),
            &archive,
            &["first.txt", "second.txt"],
        );
        std::fs::remove_file(&first).expect("remove first source payload");
        std::fs::remove_file(&second).expect("remove second source payload");
        std::fs::create_dir(&existing).expect("create existing directory");
        std::fs::write(existing.join("marker.txt"), b"existing").expect("create marker");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("extract archive without merging directories");

        let sequenced = sandbox.path().join("bundle (1)");
        assert_eq!(outcome.output, sequenced);
        assert_eq!(
            std::fs::read(existing.join("marker.txt")).unwrap(),
            b"existing"
        );
        assert_eq!(
            std::fs::read(sequenced.join("first.txt")).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(sequenced.join("second.txt")).unwrap(),
            b"second"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn platform_metadata_does_not_change_the_top_level_layout() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("payload.txt");
        let ds_store = sandbox.path().join(".DS_Store");
        let metadata = sandbox.path().join("__MACOSX");
        let archive = sandbox.path().join("archive.7z");
        std::fs::write(&payload, b"payload").expect("create payload");
        std::fs::write(&ds_store, b"metadata").expect("create DS_Store");
        std::fs::create_dir(&metadata).expect("create metadata directory");
        std::fs::write(metadata.join("entry"), b"metadata").expect("create metadata entry");
        create_archive(
            &seven_zip,
            sandbox.path(),
            &archive,
            &["payload.txt", ".DS_Store", "__MACOSX"],
        );
        std::fs::remove_file(&payload).expect("remove source payload");
        std::fs::remove_file(&ds_store).expect("remove source DS_Store");
        std::fs::remove_dir_all(&metadata).expect("remove source metadata directory");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("extract archive");

        assert_eq!(outcome.output, payload);
        assert!(!sandbox.path().join(".DS_Store").exists());
        assert!(!sandbox.path().join("__MACOSX").exists());
        assert!(!sandbox.path().join("archive").exists());
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn symbolic_link_that_escapes_the_result_is_discarded_and_reported() {
        use std::os::unix::fs::symlink;

        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let link = sandbox.path().join("escape");
        let archive = sandbox.path().join("archive.7z");
        symlink("../outside", &link).expect("create escaping symlink");
        create_archive(&seven_zip, sandbox.path(), &archive, &["escape"]);
        std::fs::remove_file(&link).expect("remove source symlink");

        // 设计 §5.4 / D1：不安全条目不得否决整个输入。
        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("an escaping link must not fail the whole input");

        let reported = outcome
            .warnings
            .iter()
            .find_map(|warning| match warning {
                ExtractionWarning::UnsafeEntriesSkipped {
                    discarded,
                    sanitized,
                } => Some((discarded, sanitized)),
                _ => None,
            })
            .expect("the discarded entry must be reported");
        assert!(
            reported
                .0
                .iter()
                .chain(reported.1.iter().map(Path::new))
                .any(|entry| entry.to_string_lossy().contains("escape")),
            "the escaping entry must be named in the report: {reported:?}"
        );

        // 提交结果里不得留下逃逸链接（要么没有，要么不再是链接）。
        let committed = outcome.output.join("escape");
        if committed.exists() {
            assert!(
                !std::fs::symlink_metadata(&committed)
                    .expect("inspect committed entry")
                    .file_type()
                    .is_symlink(),
                "an escaping link must not be committed as a link"
            );
        }
        assert!(
            std::fs::symlink_metadata(&link).is_err(),
            "unsafe output must not be committed outside the result"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn parent_directory_entry_is_sanitized_and_reported() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let archive = sandbox.path().join("unsafe.zip");
        let escaped_name = format!("ezz-escaped-{}.txt", std::process::id());
        let escaped = sandbox
            .path()
            .parent()
            .expect("sandbox parent")
            .join(&escaped_name);
        let file = std::fs::File::create(&archive).expect("create unsafe ZIP");
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                format!("../{escaped_name}"),
                zip::write::SimpleFileOptions::default(),
            )
            .expect("start unsafe ZIP entry");
        writer
            .write_all(b"must not escape")
            .expect("write ZIP entry");
        writer.finish().expect("finish unsafe ZIP");

        // 设计 §5.4 / D1：路径需要消毒的条目保留（数据不得丢失），但必须报告。
        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("a sanitized path must not fail the whole input");

        assert!(
            !escaped.exists(),
            "archive entry must not escape the workspace"
        );
        assert!(
            outcome.warnings.iter().any(|warning| matches!(
                warning,
                ExtractionWarning::UnsafeEntriesSkipped { sanitized, .. }
                    if sanitized.iter().any(|entry| entry.contains(&escaped_name))
            )),
            "the sanitized entry must be named in the report: {:?}",
            outcome.warnings
        );
        assert_eq!(
            outcome.output,
            sandbox.path().join(&escaped_name),
            "the sanitized entry must be committed inside the archive directory"
        );
        assert_eq!(
            std::fs::read_to_string(&outcome.output).expect("read committed entry"),
            "must not escape"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn encrypted_archive_uses_prompted_password_and_honors_keep_source() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("secret.txt");
        let archive = sandbox.path().join("secret.7z");
        std::fs::write(&payload, b"classified").expect("create secret payload");
        create_encrypted_archive(
            &seven_zip,
            sandbox.path(),
            &archive,
            "secret.txt",
            "correct horse",
        );
        std::fs::remove_file(&payload).expect("remove source payload");
        let prompt = ScriptedPasswordPrompt::new([PasswordResponse {
            password: "correct horse".to_owned(),
            remember: false,
            keep_original: true,
        }]);

        let outcome = workflow_with(&seven_zip, FailingSourceCleaner, prompt)
            .extract(&archive)
            .expect("extract encrypted archive");

        assert_eq!(std::fs::read(&payload).unwrap(), b"classified");
        assert!(archive.is_file(), "keep source must preserve the archive");
        assert!(outcome.warnings.is_empty(), "cleaner must not be called");
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn content_encrypted_archive_uses_the_prompted_password() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("visible-name.txt");
        let archive = sandbox.path().join("content-encrypted.7z");
        std::fs::write(&payload, b"encrypted content").expect("create secret payload");
        create_content_encrypted_archive(
            &seven_zip,
            sandbox.path(),
            &archive,
            "visible-name.txt",
            "content password",
        );
        std::fs::remove_file(&payload).expect("remove source payload");
        let prompt = ScriptedPasswordPrompt::new([PasswordResponse {
            password: "content password".to_owned(),
            remember: false,
            keep_original: false,
        }]);

        let outcome = workflow_with(&seven_zip, RemoveSource, prompt)
            .extract(&archive)
            .expect("extract content-encrypted archive");

        assert_eq!(outcome.output, payload);
        assert_eq!(std::fs::read(&payload).unwrap(), b"encrypted content");
        assert!(
            !archive.exists(),
            "successful extraction must clean the source"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn password_prompt_can_retry_after_an_incorrect_password() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("secret.txt");
        let archive = sandbox.path().join("secret.7z");
        std::fs::write(&payload, b"classified").expect("create secret payload");
        create_encrypted_archive(
            &seven_zip,
            sandbox.path(),
            &archive,
            "secret.txt",
            "correct horse",
        );
        std::fs::remove_file(&payload).expect("remove source payload");
        let prompt = ScriptedPasswordPrompt::new([
            PasswordResponse {
                password: "wrong".to_owned(),
                remember: false,
                keep_original: false,
            },
            PasswordResponse {
                password: "correct horse".to_owned(),
                remember: false,
                keep_original: false,
            },
        ]);

        workflow_with(&seven_zip, RemoveSource, prompt)
            .extract(&archive)
            .expect("retry with the correct password");

        assert_eq!(std::fs::read(&payload).unwrap(), b"classified");
        assert!(!archive.exists(), "successful retry must clean the source");
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn cancelling_the_password_prompt_preserves_the_archive() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("cancelled-secret.txt");
        let archive = sandbox.path().join("cancelled.7z");
        std::fs::write(&payload, b"cancelled secret").expect("create secret payload");
        create_encrypted_archive(
            &seven_zip,
            sandbox.path(),
            &archive,
            "cancelled-secret.txt",
            "not entered",
        );
        std::fs::remove_file(&payload).expect("remove source payload");

        let result = workflow_with(&seven_zip, RemoveSource, NoResponsePrompt).extract(&archive);

        assert_eq!(
            result,
            Err(ExtractionError::PasswordRequired(archive.clone()))
        );
        assert!(archive.is_file(), "cancelled archive must be preserved");
        assert!(
            !payload.exists(),
            "cancelled archive must not commit output"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn remembered_password_is_used_for_the_next_archive() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let password_store = sandbox.path().join("passwords.json");
        let first_payload = sandbox.path().join("first-secret.txt");
        let first_archive = sandbox.path().join("first.7z");
        std::fs::write(&first_payload, b"first secret").expect("create first payload");
        create_encrypted_archive(
            &seven_zip,
            sandbox.path(),
            &first_archive,
            "first-secret.txt",
            "shared password",
        );
        std::fs::remove_file(&first_payload).expect("remove first source payload");

        let first_prompt = ScriptedPasswordPrompt::new([PasswordResponse {
            password: "shared password".to_owned(),
            remember: true,
            keep_original: false,
        }]);
        workflow_with_store(&seven_zip, RemoveSource, first_prompt, &password_store)
            .extract(&first_archive)
            .expect("extract and remember first password");

        let second_payload = sandbox.path().join("second-secret.txt");
        let second_archive = sandbox.path().join("second.7z");
        std::fs::write(&second_payload, b"second secret").expect("create second payload");
        create_encrypted_archive(
            &seven_zip,
            sandbox.path(),
            &second_archive,
            "second-secret.txt",
            "shared password",
        );
        std::fs::remove_file(&second_payload).expect("remove second source payload");

        workflow_with_store(&seven_zip, RemoveSource, NoResponsePrompt, &password_store)
            .extract(&second_archive)
            .expect("reuse remembered password without a prompt");

        assert_eq!(std::fs::read(&first_payload).unwrap(), b"first secret");
        assert_eq!(std::fs::read(&second_payload).unwrap(), b"second secret");
        assert!(
            password_store.is_file(),
            "remembered password must be persisted"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn numeric_volume_input_finds_the_first_volume_and_cleans_the_complete_set() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("payload.bin");
        let archive = sandbox.path().join("bundle.7z");
        std::fs::write(&payload, vec![0x5a; 8 * 1024]).expect("create volume payload");
        create_split_archive(&seven_zip, sandbox.path(), &archive, "payload.bin");
        std::fs::remove_file(&payload).expect("remove source payload");
        let second_volume = sandbox.path().join("bundle.7z.002");
        assert!(
            second_volume.is_file(),
            "fixture must contain a second volume"
        );

        let outcome = workflow(&seven_zip)
            .extract(&second_volume)
            .expect("extract from a non-first numeric volume");

        assert_eq!(outcome.output, payload);
        assert_eq!(std::fs::read(&payload).unwrap(), vec![0x5a; 8 * 1024]);
        assert!(
            !sandbox.path().join("bundle.7z.001").exists(),
            "first volume must be cleaned"
        );
        assert!(!second_volume.exists(), "selected volume must be cleaned");
        assert!(
            !sandbox.path().join("bundle.7z.003").exists(),
            "remaining volumes must be cleaned"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn numeric_volume_uses_the_logical_archive_name_for_multiple_outputs() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let first_payload = sandbox.path().join("first.bin");
        let second_payload = sandbox.path().join("second.bin");
        let archive = sandbox.path().join("bundle.7z");
        std::fs::write(&first_payload, vec![0x31; 2 * 1024]).expect("create first payload");
        std::fs::write(&second_payload, vec![0x32; 2 * 1024]).expect("create second payload");
        create_split_archive_with_inputs(
            &seven_zip,
            sandbox.path(),
            &archive,
            &["first.bin", "second.bin"],
        );
        std::fs::remove_file(&first_payload).expect("remove first source payload");
        std::fs::remove_file(&second_payload).expect("remove second source payload");
        let selected = sandbox.path().join("bundle.7z.002");

        let outcome = workflow(&seven_zip)
            .extract(&selected)
            .expect("extract multiple files from a non-first volume");

        let output = sandbox.path().join("bundle");
        assert_eq!(outcome.output, output);
        assert_eq!(
            std::fs::read(output.join("first.bin")).unwrap(),
            vec![0x31; 2 * 1024]
        );
        assert_eq!(
            std::fs::read(output.join("second.bin")).unwrap(),
            vec![0x32; 2 * 1024]
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn steganographier_mp4_extracts_its_embedded_zip() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("hidden.txt");
        let embedded = sandbox.path().join("embedded.zip");
        let video = sandbox.path().join("carrier.mp4");
        std::fs::write(&payload, b"hidden payload").expect("create hidden payload");
        create_zip_archive(&seven_zip, sandbox.path(), &embedded, "hidden.txt");
        std::fs::remove_file(&payload).expect("remove source payload");

        let mut carrier = minimal_mp4();
        carrier.extend(std::fs::read(&embedded).expect("read embedded ZIP"));
        std::fs::write(&video, carrier).expect("create Steganographier MP4");
        std::fs::remove_file(&embedded).expect("remove standalone embedded ZIP");

        let outcome = workflow(&seven_zip)
            .extract(&video)
            .expect("extract Steganographier MP4");

        assert_eq!(outcome.output, payload);
        assert_eq!(std::fs::read(&payload).unwrap(), b"hidden payload");
        assert!(
            !video.exists(),
            "successful extraction must clean the video"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn ordinary_mp4_is_rejected_without_modifying_the_source() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let video = sandbox.path().join("ordinary.mp4");
        std::fs::write(&video, minimal_mp4()).expect("create ordinary MP4");

        let result = workflow(&seven_zip).extract(&video);

        assert_eq!(
            result,
            Err(ExtractionError::UnsupportedInput(video.clone()))
        );
        assert!(video.is_file(), "ordinary video must be preserved");
        assert_eq!(
            std::fs::read_dir(sandbox.path()).unwrap().count(),
            1,
            "ordinary video must not create output or leave a workspace"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn archive_with_an_mp4_extension_is_detected_by_content() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("renamed.txt");
        let archive = sandbox.path().join("renamed.mp4");
        std::fs::write(&payload, b"renamed archive").expect("create payload");
        create_zip_archive(&seven_zip, sandbox.path(), &archive, "renamed.txt");
        std::fs::remove_file(&payload).expect("remove source payload");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("extract renamed ZIP");

        assert_eq!(outcome.output, payload);
        assert_eq!(std::fs::read(&payload).unwrap(), b"renamed archive");
        assert!(
            !archive.exists(),
            "successful extraction must clean the source"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn tar_gzip_and_xz_archives_extract_through_the_shared_workflow() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        for (archive_type, extension) in [("tar", "tar"), ("gzip", "gz"), ("xz", "xz")] {
            let sandbox = tempfile::tempdir().expect("create format sandbox");
            let payload = sandbox.path().join(format!("payload-{archive_type}.txt"));
            let archive = sandbox.path().join(format!("archive.{extension}"));
            let content = format!("{archive_type} payload");
            std::fs::write(&payload, &content).expect("create format payload");
            create_typed_archive(
                &seven_zip,
                sandbox.path(),
                &archive,
                payload.file_name().unwrap().to_str().unwrap(),
                archive_type,
            );
            std::fs::remove_file(&payload).expect("remove source payload");

            let outcome = workflow(&seven_zip)
                .extract(&archive)
                .expect("extract archive format");

            let expected_output = if archive_type == "xz" {
                sandbox.path().join("archive")
            } else {
                payload
            };
            assert_eq!(outcome.output, expected_output);
            assert_eq!(std::fs::read_to_string(&expected_output).unwrap(), content);
            assert!(
                !archive.exists(),
                "successful extraction must clean the source"
            );
        }
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn steganographier_mkv_extracts_its_embedded_zip() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let payload = sandbox.path().join("mkv-hidden.txt");
        let embedded = sandbox.path().join("mkv-embedded.zip");
        let video = sandbox.path().join("carrier.mkv");
        std::fs::write(&payload, b"MKV hidden payload").expect("create hidden payload");
        create_zip_archive(&seven_zip, sandbox.path(), &embedded, "mkv-hidden.txt");
        std::fs::remove_file(&payload).expect("remove source payload");

        let mut carrier = minimal_mkv();
        carrier.extend(std::fs::read(&embedded).expect("read embedded ZIP"));
        std::fs::write(&video, carrier).expect("create Steganographier MKV");
        std::fs::remove_file(&embedded).expect("remove standalone embedded ZIP");

        let outcome = workflow(&seven_zip)
            .extract(&video)
            .expect("extract Steganographier MKV");

        assert_eq!(outcome.output, payload);
        assert_eq!(std::fs::read(&payload).unwrap(), b"MKV hidden payload");
        assert!(
            !video.exists(),
            "successful extraction must clean the video"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn rar_non_first_volume_extracts_and_cleans_the_complete_set() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let mut volumes = Vec::new();
        for sequence in 1..=3 {
            let name = format!("rar-multivolume.part{sequence}.rar");
            let source = fixture(&name);
            let destination = sandbox.path().join(&name);
            std::fs::copy(source, &destination).expect("copy RAR volume fixture");
            volumes.push(destination);
        }

        let outcome = workflow(&seven_zip)
            .extract(&volumes[1])
            .expect("extract from second RAR volume");

        let output = sandbox.path().join("LibarchiveAddingTest.html");
        assert_eq!(outcome.output, output);
        let content = std::fs::read(&output).expect("read extracted RAR content");
        assert_eq!(content.len(), 20_111);
        assert!(content.ends_with(b"</BODY>\n</HTML>"));
        assert!(
            volumes.iter().all(|volume| !volume.exists()),
            "successful extraction must clean every RAR volume"
        );
    }

    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn zip_non_first_volume_extracts_and_cleans_the_complete_set() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let first = sandbox.path().join("zip-multivolume.z01");
        let final_volume = sandbox.path().join("zip-multivolume.zip");
        std::fs::copy(fixture("zip-multivolume.z01"), &first)
            .expect("copy first ZIP volume fixture");
        std::fs::copy(fixture("zip-multivolume.zip"), &final_volume)
            .expect("copy final ZIP volume fixture");

        let outcome = workflow(&seven_zip)
            .extract(&first)
            .expect("extract from first ZIP split volume");

        let output = sandbox.path().join("zip-volume-payload.txt");
        assert_eq!(outcome.output, output);
        let content = std::fs::read(&output).expect("read extracted ZIP content");
        assert_eq!(content.len(), 70_000);
        assert!(content.starts_with(b"ezz zip volume payload\n"));
        assert!(!first.exists(), "first ZIP volume must be cleaned");
        assert!(!final_volume.exists(), "final ZIP volume must be cleaned");
    }

    /// 符号链接条目：逃逸的必须被报告且不得以链接形态提交，内部的保留（R4 同批的 D1 行为）。
    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn symbolic_link_entries_do_not_fail_the_input() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let archive = sandbox.path().join("links.zip");
        let file = std::fs::File::create(&archive).expect("create ZIP");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        writer
            .add_symlink("escape-link", "../outside.txt", options)
            .expect("add escaping symlink");
        writer
            .add_symlink("inner-link", "target.txt", options)
            .expect("add inner symlink");
        writer
            .start_file("target.txt", options)
            .expect("start entry");
        writer.write_all(b"payload\n").expect("write entry");
        writer.finish().expect("finish ZIP");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("symbolic links must not fail the whole input");

        let reported = outcome
            .warnings
            .iter()
            .find_map(|warning| match warning {
                ExtractionWarning::UnsafeEntriesSkipped {
                    discarded,
                    sanitized,
                } => Some((discarded, sanitized)),
                _ => None,
            })
            .expect("the escaping link must be reported");
        assert!(
            reported
                .0
                .iter()
                .any(|path| path.to_string_lossy().contains("escape-link"))
                || reported.1.iter().any(|name| name.contains("escape-link")),
            "the escaping link must be named: discarded={:?} sanitized={:?}",
            reported.0,
            reported.1
        );

        let committed_escape = outcome.output.join("escape-link");
        if committed_escape.exists() {
            assert!(
                !std::fs::symlink_metadata(&committed_escape)
                    .expect("inspect committed entry")
                    .file_type()
                    .is_symlink(),
                "an escaping link must not be committed as a link"
            );
        }

        let committed_inner = outcome.output.join("inner-link");
        let is_link = std::fs::symlink_metadata(&committed_inner)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        if is_link {
            assert_eq!(
                std::fs::read_link(&committed_inner).expect("read committed link"),
                Path::new("target.txt"),
                "a link inside the result must be preserved as-is"
            );
        }
    }

    /// 剔除平台元数据后没有内容：降级成功 + 报告（设计 §5.1 门 3）。
    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn archive_with_only_platform_metadata_is_a_reported_degraded_success() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let archive = sandbox.path().join("meta.zip");
        let file = std::fs::File::create(&archive).expect("create ZIP");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("__MACOSX/junk", options)
            .expect("start entry");
        writer.write_all(b"junk\n").expect("write entry");
        writer
            .start_file(".DS_Store", options)
            .expect("start entry");
        writer.write_all(b"ds\n").expect("write entry");
        writer.finish().expect("finish ZIP");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("an archive with only platform metadata must not fail");

        assert!(
            outcome.output.is_dir(),
            "an empty result must still have a final path: {:?}",
            outcome.output
        );
        assert_eq!(
            std::fs::read_dir(&outcome.output)
                .expect("read empty result")
                .count(),
            0,
            "the committed result must be empty"
        );
        assert!(
            outcome.warnings.iter().any(|warning| matches!(
                warning,
                ExtractionWarning::EmptyAfterMetadataRemoval { removed } if !removed.is_empty()
            )),
            "the empty result must be reported: {:?}",
            outcome.warnings
        );
    }

    /// 逃逸不变量本身（设计 §5.4）：工作目录内的改动不算逃逸，归档所在目录的新条目算。
    #[test]
    fn escape_invariant_detects_changes_outside_the_workspace() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let workspace = sandbox.path().join(".ezz-work-test");
        std::fs::create_dir(&workspace).expect("create workspace");

        let snapshot = directory_snapshot(sandbox.path(), &workspace).expect("snapshot");
        assert!(validate_escape_invariant(sandbox.path(), &workspace, &snapshot).is_ok());

        std::fs::write(workspace.join("file.txt"), b"inside").expect("write inside workspace");
        assert!(
            validate_escape_invariant(sandbox.path(), &workspace, &snapshot).is_ok(),
            "changes inside the workspace are not an escape"
        );

        std::fs::write(sandbox.path().join("escaped.txt"), b"outside").expect("write outside");
        assert!(
            matches!(
                validate_escape_invariant(sandbox.path(), &workspace, &snapshot),
                Err(ExtractionError::UnsafeOutput { .. })
            ),
            "a new entry outside the workspace must break the invariant"
        );
    }

    #[test]
    fn volume_suffixes_are_recognized() {
        assert_eq!(numeric_extension(Path::new("data.001")), Some(1));
        assert_eq!(numeric_extension(Path::new("data.003")), Some(3));
        assert_eq!(numeric_extension(Path::new("data.01")), None);
        assert_eq!(numeric_extension(Path::new("data.zip")), None);
        assert_eq!(numeric_extension(Path::new("data")), None);

        assert_eq!(zip_volume_sequence(Path::new("data.z01")), Some(1));
        assert_eq!(zip_volume_sequence(Path::new("data.Z09")), Some(9));
        assert_eq!(zip_volume_sequence(Path::new("data.z1")), None);
        assert_eq!(zip_volume_sequence(Path::new("data.zip")), None);
        assert!(has_zip_extension(Path::new("data.ZIP")));

        assert_eq!(
            archive_stem(Path::new("archive.tar.gz")),
            OsString::from("archive.tar")
        );
        assert_eq!(
            archive_stem(Path::new("no-extension")),
            OsString::from("no-extension")
        );

        let volume = rar_volume_name(Path::new("book.part002.rar")).expect("rar volume name");
        assert_eq!(volume.prefix, "book");
        assert_eq!(volume.sequence, 2);
        assert_eq!(volume.width, 3);
        assert_eq!(volume.extension, "rar");
        assert_eq!(
            rar_volume_path(Path::new("C:/data"), &volume, 7),
            PathBuf::from("C:/data/book.part007.rar")
        );
        assert!(rar_volume_name(Path::new("book.rar")).is_none());
        assert!(rar_volume_name(Path::new("book.part.rar")).is_none());
        assert!(rar_volume_name(Path::new("book.part01.zip")).is_none());
    }

    #[test]
    fn destination_names_follow_the_conflict_rules() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let parent = sandbox.path();

        let files: Vec<String> =
            unique_destination_candidates(parent, OsStr::new("archive.tar.gz"), CommitKind::File)
                .take(3)
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
        assert_eq!(
            files,
            ["archive.tar.gz", "archive.tar (1).gz", "archive.tar (2).gz"]
        );

        // 目录整体递增：不得把最后一个“扩展名”拆开（§5.3）。
        let directories: Vec<String> =
            unique_destination_candidates(parent, OsStr::new("archive.tar"), CommitKind::Directory)
                .take(3)
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
        assert_eq!(
            directories,
            ["archive.tar", "archive.tar (1)", "archive.tar (2)"]
        );
    }

    #[test]
    fn empty_result_never_overwrites_an_existing_entry() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        std::fs::write(sandbox.path().join("payload"), b"existing").expect("create existing file");

        let created =
            commit_empty_directory(sandbox.path(), OsStr::new("payload")).expect("commit empty");

        assert_eq!(
            created.file_name().unwrap().to_string_lossy(),
            "payload (1)"
        );
        assert!(created.is_dir());
        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("payload")).expect("read existing"),
            "existing"
        );
    }

    /// 手写一个最小 tar：`zip` crate 会把反斜杠改写成下划线，所以盘符前缀条目只能这样造。
    fn write_tar(path: &Path, entries: &[(&str, &[u8])]) {
        let mut bytes = Vec::new();
        for (name, data) in entries {
            let mut header = [0_u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[100..108].copy_from_slice(b"0000644\0");
            header[108..116].copy_from_slice(b"0000000\0");
            header[116..124].copy_from_slice(b"0000000\0");
            header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
            header[136..148].copy_from_slice(b"00000000000\0");
            header[156] = b'0';
            header[148..156].copy_from_slice(b"        ");
            let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
            header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
            bytes.extend_from_slice(&header);
            bytes.extend_from_slice(data);
            bytes.extend(std::iter::repeat_n(0_u8, (512 - data.len() % 512) % 512));
        }
        bytes.extend(std::iter::repeat_n(0_u8, 1024));
        std::fs::write(path, &bytes).expect("write tar");
    }

    /// 盘符前缀条目（`C:\drive.txt`）：7-Zip 读取时会把它改写成 `C:_drive.txt`，提取时再
    /// 把非法字符 `:` 换成 `_`。数据必须保留在结果内，且必须报告（§5.4）。
    ///
    /// 用 tar 而不是 zip：`zip` crate 会在写入时就把反斜杠换成下划线，造不出真的盘符条目。
    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn drive_prefixed_entries_are_sanitized_and_reported() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let archive = sandbox.path().join("drive.tar");
        write_tar(
            &archive,
            &[("C:\\drive.txt", b"drive payload"), ("keep.txt", b"keep")],
        );

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("a drive-prefixed entry must not fail the whole input");

        let sanitized = outcome
            .warnings
            .iter()
            .find_map(|warning| match warning {
                ExtractionWarning::UnsafeEntriesSkipped { sanitized, .. } => Some(sanitized),
                _ => None,
            })
            .expect("the drive-prefixed entry must be reported");
        assert!(
            sanitized.iter().any(|name| name.contains("drive.txt")),
            "the entry must be named in the report: {sanitized:?}"
        );

        // 数据不得丢失：必须落在结果内（名字会被消毒成合法文件名）。
        assert!(outcome.output.is_dir(), "{:?}", outcome.output);
        let mut found_payload = false;
        for entry in std::fs::read_dir(&outcome.output).expect("read result") {
            let entry = entry.expect("result entry");
            if entry.file_type().expect("file type").is_file()
                && std::fs::read(entry.path()).expect("read entry") == b"drive payload"
            {
                found_payload = true;
            }
        }
        assert!(found_payload, "the sanitized entry must keep its data");
        assert!(outcome.output.join("keep.txt").is_file());
    }

    /// 绝对路径条目（`/absolute.txt`）：7-Zip 把它重写进工作目录（提交后为 `absolute.txt`）。
    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn absolute_path_entries_are_sanitized_and_reported() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let archive = sandbox.path().join("absolute.zip");
        let file = std::fs::File::create(&archive).expect("create ZIP");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for name in ["/absolute.txt", "keep.txt"] {
            writer.start_file(name, options).expect("start entry");
            writer.write_all(name.as_bytes()).expect("write entry");
        }
        writer.finish().expect("finish ZIP");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("an absolute entry path must not fail the whole input");

        let sanitized = outcome
            .warnings
            .iter()
            .find_map(|warning| match warning {
                ExtractionWarning::UnsafeEntriesSkipped { sanitized, .. } => Some(sanitized),
                _ => None,
            })
            .expect("the absolute entry path must be reported");
        assert!(
            sanitized.iter().any(|name| name.contains("absolute.txt")),
            "the entry must be named in the report: {sanitized:?}"
        );

        assert!(outcome.output.is_dir(), "{:?}", outcome.output);
        assert!(
            outcome.output.join("absolute.txt").is_file(),
            "the sanitized entry must be kept"
        );
        assert!(outcome.output.join("keep.txt").is_file());
    }

    /// 把最后一个条目的压缩数据末字节改坏（中央目录紧跟在数据之后）。
    fn corrupt_last_entry_byte(bytes: &mut [u8]) {
        let eocd = bytes
            .windows(4)
            .rposition(|window| window == b"PK\x05\x06")
            .expect("find end of central directory");
        let directory_offset = u32::from_le_bytes(
            bytes[eocd + 16..eocd + 20]
                .try_into()
                .expect("offset field"),
        ) as usize;
        bytes[directory_offset - 1] ^= 0xFF;
    }

    /// 单个条目损坏 → 降级成功：其余条目照常提交，**损坏条目也照旧提交**，只点名报告。
    ///
    /// 实测：7-Zip 遇到 `CRC Failed` / `Data Error` 时退出码 2，但仍会把（损坏的）内容写进
    /// 输出。按设计 §5.5 的“层 1 透传”原则，ezz 不修改 7-Zip 写出来的东西 —— 命令行用户
    /// 会拿到那个坏文件与一条错误，ezz 用户也同样拿到它，区别只在于 ezz 把它写进结构化警告。
    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn a_corrupted_entry_is_committed_and_reported_while_the_rest_is_kept() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let archive = sandbox.path().join("corrupted.zip");
        let file = std::fs::File::create(&archive).expect("create ZIP");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("good.txt", options).expect("start entry");
        writer.write_all(b"good content").expect("write entry");
        writer
            .start_file("third.txt", options)
            .expect("start entry");
        writer.write_all(b"third content").expect("write entry");
        // 故意写在最后：`corrupt_last_entry_byte` 改坏的正是最后一个条目的数据。
        writer.start_file("bad.txt", options).expect("start entry");
        writer
            .write_all(b"payload-to-corrupt")
            .expect("write entry");
        writer.finish().expect("finish ZIP");

        let mut bytes = std::fs::read(&archive).expect("read ZIP");
        corrupt_last_entry_byte(&mut bytes);
        std::fs::write(&archive, &bytes).expect("write corrupted ZIP");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("a corrupted entry must not fail the whole input");

        let reported = outcome
            .warnings
            .iter()
            .find_map(|warning| match warning {
                ExtractionWarning::FailedEntries { entries } => Some(entries),
                _ => None,
            })
            .expect("the corrupted entry must be reported");
        assert!(
            reported.iter().any(|entry| entry.contains("bad.txt")),
            "the corrupted entry must be named: {reported:?}"
        );

        // 三个顶层项 → 结果是归档名命名的目录（§5.2）。
        assert!(outcome.output.is_dir(), "{:?}", outcome.output);
        assert!(
            outcome.output.join("good.txt").is_file() && outcome.output.join("third.txt").is_file(),
            "healthy entries must still be committed"
        );
        // 层 1 透传（§5.5）：7-Zip 把损坏的条目也写出来了，ezz 不改它写出来的东西，
        // 只是把条目名写进结构化警告。命令行用户与 ezz 用户拿到的结果因此一致。
        assert!(
            outcome.output.join("bad.txt").is_file(),
            "the corrupted entry is written by 7-Zip and must not be removed by ezz"
        );
    }

    /// Unicode 与空格文件名：提交名必须原样保留，冲突时仍按 §5.3 递增序号。
    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn unicode_and_space_names_are_committed_unchanged() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let source = sandbox.path().join("source");
        std::fs::create_dir(&source).expect("create source directory");
        let name = "报告 汇总 (最终).txt";
        std::fs::write(source.join(name), b"content").expect("write source file");
        let archive = sandbox.path().join("unicode.zip");
        create_archive(&seven_zip, &source, &archive, &[name]);

        let workflow = workflow(&seven_zip);
        let first = workflow.extract(&archive).expect("first extraction");
        assert_eq!(first.output.file_name().unwrap().to_string_lossy(), name);
        assert_eq!(
            std::fs::read_to_string(&first.output).expect("read committed file"),
            "content"
        );

        // 原归档被 RemoveSource 删除，重建一次以验证冲突命名。
        create_archive(&seven_zip, &source, &archive, &[name]);
        let second = workflow.extract(&archive).expect("second extraction");
        assert_eq!(
            second.output.file_name().unwrap().to_string_lossy(),
            "报告 汇总 (最终) (1).txt"
        );
    }

    /// 特殊文件（FIFO）必须被丢弃并报告（§5.4）。Windows 上无法构造 FIFO，所以只在 Unix 跑。
    #[cfg(unix)]
    #[test]
    fn special_files_are_discarded_and_reported() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let root = sandbox.path().join("extracted");
        std::fs::create_dir(&root).expect("create root");
        std::fs::write(root.join("keep.txt"), b"keep").expect("write kept file");

        let fifo = root.join("pipe");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("run mkfifo");
        assert!(status.success(), "mkfifo must create the FIFO");

        let discarded = discard_unsafe_entries(&root).expect("discard unsafe entries");

        assert_eq!(
            discarded,
            vec![PathBuf::from("pipe")],
            "the FIFO must be discarded and named"
        );
        assert!(root.join("keep.txt").is_file(), "safe content must be kept");
        assert!(!fifo.exists(), "the FIFO must be removed");
    }

    /// 硬链接不可跨越文件系统，因此它不会变成逃逸向量；归档里的硬链接必须当普通文件处理。
    #[test]
    #[ignore = "requires cargo xtask prepare"]
    fn hard_links_are_extracted_as_regular_files() {
        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let source = sandbox.path().join("source");
        std::fs::create_dir(&source).expect("create source directory");
        std::fs::write(source.join("original.txt"), b"shared").expect("write original");
        std::fs::hard_link(source.join("original.txt"), source.join("linked.txt"))
            .expect("create hard link");

        let archive = sandbox.path().join("hardlinks.7z");
        let status = std::process::Command::new(&seven_zip)
            .current_dir(&source)
            .args(["a", "-t7z", "-snh", "-mx=1", "-bso0", "-bsp0"])
            .arg(&archive)
            .args(["original.txt", "linked.txt"])
            .status()
            .expect("create archive with 7-Zip");
        assert!(status.success(), "7-Zip must create the hard-link archive");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("hard links must not fail the input");

        assert!(
            !outcome
                .warnings
                .iter()
                .any(|warning| matches!(warning, ExtractionWarning::UnsafeEntriesSkipped { .. })),
            "hard links must not be reported as unsafe: {:?}",
            outcome.warnings
        );
        for name in ["original.txt", "linked.txt"] {
            let path = if outcome.output.is_dir() {
                outcome.output.join(name)
            } else {
                outcome.output.clone()
            };
            assert_eq!(
                std::fs::read_to_string(&path).expect("read extracted file"),
                "shared"
            );
        }
    }
    /// 在不支持符号链接的环境（例如没有权限的 CI）上返回 false，由调用方跳过。
    fn try_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, link).is_ok()
        }
    }

    #[test]
    fn unsafe_symbolic_links_are_discarded_while_safe_ones_are_kept() {
        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let root = sandbox.path().join("extracted");
        std::fs::create_dir_all(root.join("nested")).expect("create root");
        std::fs::write(root.join("target.txt"), b"target").expect("write target");
        std::fs::write(root.join("nested/keep.txt"), b"keep").expect("write kept file");

        let escaping = try_symlink(Path::new("../outside.txt"), &root.join("escaping"));
        let dangling = try_symlink(Path::new("missing.txt"), &root.join("dangling"));
        let absolute = try_symlink(&root.join("target.txt"), &root.join("absolute"));
        let inside = try_symlink(Path::new("target.txt"), &root.join("inside"));
        if !(escaping || dangling || absolute || inside) {
            eprintln!("skipping: this environment cannot create symbolic links");
            return;
        }

        let discarded = discard_unsafe_entries(&root).expect("discard unsafe entries");
        let names: Vec<String> = discarded
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();

        if escaping {
            assert!(names.iter().any(|name| name == "escaping"), "{names:?}");
        }
        if dangling {
            assert!(names.iter().any(|name| name == "dangling"), "{names:?}");
        }
        if absolute {
            assert!(names.iter().any(|name| name == "absolute"), "{names:?}");
        }
        if inside {
            assert!(!names.iter().any(|name| name == "inside"), "{names:?}");
        }
        assert!(
            !names.iter().any(|name| name.contains("keep.txt")),
            "{names:?}"
        );
        assert!(
            root.join("nested/keep.txt").is_file(),
            "safe content must be kept"
        );
        assert!(
            discarded.iter().all(|path| path.is_relative()),
            "reported entries should be relative to the extraction root"
        );
    }

    fn create_archive(seven_zip: &Path, directory: &Path, archive: &Path, inputs: &[&str]) {
        let mut command = Command::new(seven_zip);
        command
            .current_dir(directory)
            .args(["a", "-t7z"])
            .arg(archive)
            .args(inputs)
            .args(["-mx=1", "-snl", "-bso0", "-bsp0"]);
        let status = command.status().expect("create archive with 7-Zip");
        assert!(status.success(), "7-Zip must create the test archive");
    }

    fn create_encrypted_archive(
        seven_zip: &Path,
        directory: &Path,
        archive: &Path,
        input: &str,
        password: &str,
    ) {
        let status = Command::new(seven_zip)
            .current_dir(directory)
            .args(["a", "-t7z"])
            .arg(archive)
            .arg(input)
            .arg(format!("-p{password}"))
            .args(["-mhe=on", "-mx=1", "-bso0", "-bsp0"])
            .status()
            .expect("create encrypted archive with 7-Zip");
        assert!(status.success(), "7-Zip must create encrypted test archive");
    }

    fn create_content_encrypted_archive(
        seven_zip: &Path,
        directory: &Path,
        archive: &Path,
        input: &str,
        password: &str,
    ) {
        let status = Command::new(seven_zip)
            .current_dir(directory)
            .args(["a", "-t7z"])
            .arg(archive)
            .arg(input)
            .arg(format!("-p{password}"))
            .args(["-mhe=off", "-mx=1", "-bso0", "-bsp0"])
            .status()
            .expect("create content-encrypted archive with 7-Zip");
        assert!(status.success(), "7-Zip must create encrypted test archive");
    }

    fn create_zip_archive(seven_zip: &Path, directory: &Path, archive: &Path, input: &str) {
        let status = Command::new(seven_zip)
            .current_dir(directory)
            .args(["a", "-tzip"])
            .arg(archive)
            .arg(input)
            .args(["-mx=1", "-bso0", "-bsp0"])
            .status()
            .expect("create ZIP with 7-Zip");
        assert!(status.success(), "7-Zip must create the embedded ZIP");
    }

    fn create_typed_archive(
        seven_zip: &Path,
        directory: &Path,
        archive: &Path,
        input: &str,
        archive_type: &str,
    ) {
        let status = Command::new(seven_zip)
            .current_dir(directory)
            .arg("a")
            .arg(format!("-t{archive_type}"))
            .arg(archive)
            .arg(input)
            .args(["-mx=1", "-bso0", "-bsp0"])
            .status()
            .expect("create typed archive with 7-Zip");
        assert!(status.success(), "7-Zip must create {archive_type}");
    }

    fn minimal_mp4() -> Vec<u8> {
        vec![
            0, 0, 0, 24, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0, 0, 2, 0, b'i', b's',
            b'o', b'm', b'm', b'p', b'4', b'2', 0, 0, 0, 8, b'f', b'r', b'e', b'e',
        ]
    }

    fn minimal_mkv() -> Vec<u8> {
        vec![
            0x1a, 0x45, 0xdf, 0xa3, 0x8f, 0x42, 0x86, 0x81, 0x01, 0x42, 0xf7, 0x81, 0x01, 0x42,
            0xf2, 0x81, 0x04,
        ]
    }

    fn create_split_archive(seven_zip: &Path, directory: &Path, archive: &Path, input: &str) {
        create_split_archive_with_inputs(seven_zip, directory, archive, &[input]);
    }

    fn create_split_archive_with_inputs(
        seven_zip: &Path,
        directory: &Path,
        archive: &Path,
        inputs: &[&str],
    ) {
        let status = Command::new(seven_zip)
            .current_dir(directory)
            .args(["a", "-t7z"])
            .arg(archive)
            .args(inputs)
            .args(["-v1k", "-mx=0", "-bso0", "-bsp0"])
            .status()
            .expect("create split archive with 7-Zip");
        assert!(status.success(), "7-Zip must create split test archive");
    }

    fn prepared_seven_zip() -> PathBuf {
        let binary_name = if cfg!(target_os = "windows") {
            "7zz.exe"
        } else {
            "7zz"
        };
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("ezz-tools")
            .join("26.02")
            .join(binary_name)
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }
}
