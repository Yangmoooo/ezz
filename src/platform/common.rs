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
            // 标题不带文件名（设计 §3.2）：名字在正文里出现一次就够了。
            super::show_notification("Extraction complete", &success_body(outcome));
        }
        Err(extraction_error) => {
            error!("failed to extract {}: {extraction_error}", input.display());
            super::show_notification("Extraction failed", &failure_body(&name, extraction_error));
        }
    }
}

/// 成功通知的正文：最终实际路径（§5.3 要求），有警告时补一行数量。
///
/// 文件名就在路径里，所以标题不再重复它。
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

/// 失败通知的正文：文件名 + 原因。
///
/// 有的错误变体（如 `InputNotFound`）自带路径，那就不要再补一次名字：名字只出现一次。
fn failure_body(name: &str, error: &ExtractionError) -> String {
    let reason = error.to_string();
    if reason.contains(name) {
        reason
    } else {
        format!("{name}\n{reason}")
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

/// 密码弹窗第一行的说明文字（设计 §7）。
///
/// 文件名单独占一行（Windows 交给 `SS_PATHELLIPSIS`，macOS 交给 `truncate_middle`），
/// 所以这句必须短到不会换行。两个平台的文案都在这里，改词只改一处。
pub fn password_prompt_message(previous_attempt_failed: bool) -> &'static str {
    if previous_attempt_failed {
        "The password was incorrect. Try again."
    } else {
        "Enter the password for:"
    }
}

/// 名字过长时从中间省略（macOS 用；Windows 由系统在绘制时处理）。
///
/// 放在这里而不是 macOS 模块里，是为了让它的用例在任意主机上都能跑。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
///
/// 保留开头与结尾：文件名最有信息量的部分正是这两端（`archive…part3.rar`）。按**字符**
/// 而不是字节计数，避免把 Unicode 名字切成半个字符。
pub fn truncate_middle(name: &str, limit: usize) -> String {
    let characters: Vec<char> = name.chars().collect();
    if characters.len() <= limit || limit < 3 {
        return name.to_owned();
    }

    let keep = limit - 1; // 一个字符留给省略号
    let head = keep.div_ceil(2);
    let tail = keep - head;
    let mut shortened: String = characters[..head].iter().collect();
    shortened.push('…');
    shortened.extend(&characters[characters.len() - tail..]);
    shortened
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_prompt_message_reports_a_previous_failure() {
        assert_eq!(password_prompt_message(false), "Enter the password for:");
        assert_eq!(
            password_prompt_message(true),
            "The password was incorrect. Try again."
        );
    }

    #[test]
    fn truncate_middle_keeps_both_ends() {
        assert_eq!(truncate_middle("archive.7z", 20), "archive.7z");
        assert_eq!(
            truncate_middle("averyveryverylongarchive.7z", 16),
            "averyver…hive.7z"
        );
    }

    #[test]
    fn truncate_middle_counts_characters_not_bytes() {
        let shortened = truncate_middle("归档文件非常长的一个名字.7z", 10);
        assert_eq!(shortened.chars().count(), 10);
        assert!(shortened.starts_with('归'));
        assert!(shortened.ends_with("7z"));
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
    fn failure_body_names_the_input_exactly_once() {
        // 错误消息里没有名字：补上它。
        let body = failure_body("archive.7z", &ExtractionError::WrongPassword);
        assert_eq!(body, "archive.7z\nArchive password is incorrect");

        // 错误消息自带路径：不再补一次名字。
        let missing = PathBuf::from("C:/data/archive.7z");
        let body = failure_body("archive.7z", &ExtractionError::InputNotFound(missing));
        assert_eq!(body, "Input does not exist: C:/data/archive.7z");
        assert_eq!(body.matches("archive.7z").count(), 1, "{body}");
    }
}
