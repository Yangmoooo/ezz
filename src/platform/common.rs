use std::error::Error;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use ezz::{ExtractionError, ExtractionOutcome, ExtractionWarning};
use log::{error, info, warn};
use simplelog::{Config, ConfigBuilder, LevelFilter, WriteLogger, format_description};

/// 拒绝报告里最多列出的输入路径条数。完整清单始终进日志（设计 §3.3）。
#[cfg(target_os = "windows")]
const MAX_REPORTED_INPUTS: usize = 4;

pub struct PlatformPaths {
    pub password_database: PathBuf,
    pub log_file: PathBuf,
}

impl PlatformPaths {
    pub fn discover() -> Result<Self, Box<dyn Error>> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
            let application_support = PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("ezz");
            // 密码库与日志同目录（设计 §10）：Console.app 只认 os_log，所以日志不进
            // ~/Library/Logs，而是和密码库一起放在应用数据目录里。
            Ok(Self {
                password_database: application_support.join("passwords.json"),
                log_file: application_support.join("ezz.log"),
            })
        }

        #[cfg(target_os = "windows")]
        {
            let local =
                PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?);
            let directory = local.join("ezz");
            Ok(Self {
                password_database: directory.join("passwords.json"),
                log_file: directory.join("ezz.log"),
            })
        }
    }
}

pub fn initialize_logging(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let parent = path.parent().ok_or("log file has no parent directory")?;
    fs::create_dir_all(parent)?;
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    WriteLogger::init(LevelFilter::Info, log_config(), file)?;
    info!("ezz {} started", env!("CARGO_PKG_VERSION"));
    Ok(())
}

/// 日志时间必须是本地时间且包含日期（设计 §10）。
///
/// `Config::default()` 是 UTC + `[HH:MM:SS]`：排查"用户在什么时间遇到的这个问题"时毫无用处。
/// 取不到本地时区偏移时保持 UTC —— 日志格式不值得让程序启动失败。
fn log_config() -> Config {
    let mut builder = ConfigBuilder::new();
    builder.set_time_format_custom(format_description!(
        "[year]-[month]-[day] [hour]:[minute]:[second]"
    ));
    let _ = builder.set_time_offset_to_local();
    builder.build()
}

/// 报告**单个输入**的结果（设计 §3.2）。
///
/// v3.0 不发送"开始处理"通知。每个输入处理完成后都要报告一次：成功时给出最终实际
/// 路径（§5.3 要求），失败时给出失败原因；警告数量（如有）一并进入通知，明细进日志。
pub fn report_outcome(input: &Path, result: &Result<ExtractionOutcome, ExtractionError>) {
    let name = display_name(input);
    match result {
        Ok(outcome) => {
            info!(
                "extracted {} to {}",
                outcome.input.display(),
                outcome.output.display()
            );
            for warning in &outcome.warnings {
                log_warning(warning);
            }

            let mut body = outcome.output.display().to_string();
            if !outcome.warnings.is_empty() {
                body.push_str(&format!(
                    "\n{} warning(s), see the log for details",
                    outcome.warnings.len()
                ));
            }
            super::show_notification(&format!("{name} extracted"), &body);
        }
        Err(extraction_error) => {
            error!("failed to extract {}: {extraction_error}", input.display());
            super::show_notification(&format!("{name} failed"), &extraction_error.to_string());
        }
    }
}

/// 报告"本次调用被跳过"（设计 §3.3）。
///
/// 措辞必须是"已跳过"而不是"失败"：另一个 ezz 正在运行，本次输入只是没有被处理，
/// 这不是错误。报告走通知通道，不使用模态弹窗 —— 多选调用可能连续出现多条。
#[cfg(target_os = "windows")]
pub fn report_skipped(inputs: &[PathBuf]) {
    // 无参数启动时没有可点名的输入：只说"已经在运行"。
    if inputs.is_empty() {
        warn!("skipped this launch: another ezz instance is already running");
        super::show_notification(
            "Already running",
            "Another ezz is already running. Please try again later.",
        );
        return;
    }

    warn!(
        "skipped {} input(s): another ezz instance is already running",
        inputs.len()
    );
    for input in inputs {
        warn!("skipped {}", input.display());
    }

    let listed = inputs
        .iter()
        .take(MAX_REPORTED_INPUTS)
        .map(|input| input.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let mut body = format!("Another ezz is already running:\n{listed}");
    if inputs.len() > MAX_REPORTED_INPUTS {
        body.push_str(&format!(
            "\n(and {} more)",
            inputs.len() - MAX_REPORTED_INPUTS
        ));
    }

    let summary = if inputs.len() == 1 {
        "1 file skipped".to_owned()
    } else {
        format!("{} files skipped", inputs.len())
    };
    super::show_notification(&summary, &body);
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn log_warning(warning: &ExtractionWarning) {
    match warning {
        ExtractionWarning::SourceCleanupFailed { sources, message } => warn!(
            "could not move source files to the trash ({}): {message}",
            sources
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ExtractionWarning::PasswordStoreUpdateFailed { path, message } => warn!(
            "could not update password database {}: {message}",
            path.display()
        ),
        ExtractionWarning::EngineWarnings { message } => {
            warn!("7-Zip reported warnings: {message}");
        }
        ExtractionWarning::UnsafeEntriesSkipped {
            discarded,
            sanitized,
        } => {
            for path in discarded {
                warn!("discarded unsafe entry {}", path.display());
            }
            for name in sanitized {
                warn!("sanitized unsafe entry path {name}");
            }
        }
        ExtractionWarning::EmptyAfterMetadataRemoval { removed } => warn!(
            "no content left after removing platform metadata ({})",
            removed.join(", ")
        ),
        ExtractionWarning::FailedEntries { entries } => {
            for entry in entries {
                warn!("entry failed to extract (data corruption): {entry}");
            }
        }
    }
}

// 通知通道由平台模块提供（设计 §12）：Windows 是 WinRT toast，macOS 是 UNUserNotificationCenter。
