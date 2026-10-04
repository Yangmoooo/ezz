use std::error::Error;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use ezz::{ExtractionError, ExtractionOutcome, ExtractionWarning};
use log::{error, info, warn};
use simplelog::{Config, ConfigBuilder, LevelFilter, WriteLogger, format_description};

/// 拒绝报告里最多列出的输入路径条数；完整清单始终进日志。
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
            // 密码库与日志同目录：日志不放 ~/Library/Logs（那只对 os_log 有意义）。
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
    info!("Ezz {} started", env!("CARGO_PKG_VERSION"));
    Ok(())
}

/// 日志时间用本地时间且包含日期（`Config::default()` 是 UTC 与 `[HH:MM:SS]`）；
/// 取不到本地时区偏移时保持 UTC。
fn log_config() -> Config {
    let mut builder = ConfigBuilder::new();
    builder.set_time_format_custom(format_description!(
        "[year]-[month]-[day] [hour]:[minute]:[second]"
    ));
    let _ = builder.set_time_offset_to_local();
    builder.build()
}

/// 报告单个输入的结果：成功给出最终实际路径，失败给出原因，警告数量也进通知，明细进日志。
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
            // 标题不带文件名：名字在正文里出现一次就够了。
            super::show_notification("Extraction complete", &success_body(outcome));
        }
        Err(extraction_error) => {
            error!("failed to extract {}: {extraction_error}", input.display());
            super::show_notification("Extraction failed", &failure_body(&name, extraction_error));
        }
    }
}

/// 成功通知的正文：最终实际路径，有警告时补一行数量。
fn success_body(outcome: &ExtractionOutcome) -> String {
    let mut body = outcome.output.display().to_string();
    if !outcome.warnings.is_empty() {
        body.push_str(&format!(
            "\n{} warning(s), see the log for details",
            outcome.warnings.len()
        ));
    }
    body
}

/// 失败通知的正文：文件名 + 错误类型 + 指向日志。
fn failure_body(name: &str, error: &ExtractionError) -> String {
    format!("{name}\n{}. See the log for details.", error.summary())
}

/// 报告"本次调用被跳过"：措辞必须是"已跳过"而不是"失败"，而且走通知而不是模态弹窗。
#[cfg(target_os = "windows")]
pub fn report_skipped(inputs: &[PathBuf]) {
    // 无参数启动时没有可点名的输入：只说"已经在运行"。
    if inputs.is_empty() {
        warn!("skipped this launch: another Ezz instance is already running");
        super::show_notification(
            "Already running",
            "Another Ezz is already running. Please try again later.",
        );
        return;
    }

    warn!(
        "skipped {} input(s): another Ezz instance is already running",
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
    let mut body = format!("Another Ezz is already running:\n{listed}");
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

/// 密码弹窗的提示词：不显示文件名（它在通知与日志里），两个平台共用这一处文案。
pub fn password_prompt_message(previous_attempt_failed: bool) -> &'static str {
    if previous_attempt_failed {
        "The password was incorrect. Try again:"
    } else {
        "Enter the password:"
    }
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

// 通知通道由平台模块提供：Windows 是 WinRT toast，macOS 是 UNUserNotificationCenter。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_prompt_message_reports_a_previous_failure() {
        assert_eq!(password_prompt_message(false), "Enter the password:");
        assert_eq!(
            password_prompt_message(true),
            "The password was incorrect. Try again:"
        );
    }

    fn outcome_with(warnings: Vec<ExtractionWarning>) -> ExtractionOutcome {
        ExtractionOutcome {
            input: PathBuf::from("C:/data/archive.7z"),
            output: PathBuf::from("C:/data/payload"),
            warnings,
        }
    }

    #[test]
    fn success_body_is_the_final_path_and_counts_warnings() {
        // 与平台无关地比较路径：`display()` 在 Windows 上会用正斜杠。
        let plain = outcome_with(Vec::new());
        assert_eq!(success_body(&plain), plain.output.display().to_string());

        let with_warning = outcome_with(vec![ExtractionWarning::EngineWarnings {
            message: "warning".to_owned(),
        }]);
        let body = success_body(&with_warning);
        assert!(
            body.starts_with(&with_warning.output.display().to_string()),
            "{body}"
        );
        assert!(body.contains("\n1 warning(s)"), "{body}");
    }

    #[test]
    fn failure_body_names_the_input_once_and_points_at_the_log() {
        let body = failure_body("archive.7z", &ExtractionError::WrongPassword);
        assert_eq!(body, "archive.7z\nWrong password. See the log for details.");
        assert_eq!(body.matches("archive.7z").count(), 1);
    }

    /// 每个错误变体都要有一句短、纯 ASCII、不泄露引擎原文的分类。
    #[test]
    fn every_error_summary_is_short_and_plain() {
        let errors = [
            ExtractionError::InputNotFound(PathBuf::from("a.7z")),
            ExtractionError::InputNotFile(PathBuf::from("a.7z")),
            ExtractionError::EngineNotFound(PathBuf::from("7zz")),
            ExtractionError::EngineLaunch {
                path: PathBuf::from("7zz"),
                message: "boom".to_owned(),
            },
            ExtractionError::EngineFailed {
                operation: "extract",
                exit_code: Some(2),
                message: "ERROR: nope".to_owned(),
            },
            ExtractionError::EngineFailed {
                operation: "test",
                exit_code: None,
                message: "ERROR: nope".to_owned(),
            },
            ExtractionError::UnsupportedInput(PathBuf::from("a.7z")),
            ExtractionError::MissingVolume(PathBuf::from("a.002")),
            ExtractionError::WrongPassword,
            ExtractionError::PasswordRequired(PathBuf::from("a.7z")),
            ExtractionError::FileSystem {
                operation: "commit",
                path: PathBuf::from("a"),
                message: "denied".to_owned(),
            },
            ExtractionError::UnsafeOutput {
                path: PathBuf::from("a"),
                reason: "escaped".to_owned(),
            },
        ];

        for error in errors {
            let summary = error.summary();
            assert!(!summary.is_empty());
            assert!(summary.len() < 60, "summary is too long: {summary}");
            assert!(summary.is_ascii(), "keep summaries plain: {summary}");
            assert!(
                !summary.contains("ERROR"),
                "engine output leaked: {summary}"
            );
            assert!(
                !summary.ends_with('.'),
                "the caller adds the period: {summary}"
            );
        }
    }

    #[test]
    fn failure_body_never_leaks_the_engine_message() {
        // 引擎原文只进日志，不进通知。
        let verbatim = ExtractionError::EngineFailed {
            operation: "extract",
            exit_code: Some(2),
            message: "ERROR: Data Error : payload/very/long/path.bin".repeat(20),
        };
        let body = failure_body("archive.7z", &verbatim);
        assert_eq!(
            body,
            "archive.7z\n7-Zip could not extract the archive. See the log for details."
        );
        assert!(!body.contains("Data Error"), "{body}");
        assert!(body.len() < 120, "the body must stay short: {body}");
    }
}
