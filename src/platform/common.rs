use std::error::Error;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;

use ezz::{BatchReport, ExtractionWarning};
use log::{error, info, warn};
use notify_rust::Notification;
use simplelog::{Config, LevelFilter, WriteLogger};

pub struct PlatformPaths {
    pub password_database: PathBuf,
    pub log_file: PathBuf,
}

impl PlatformPaths {
    pub fn discover() -> Result<Self, Box<dyn Error>> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
            let home = PathBuf::from(home);
            Ok(Self {
                password_database: home
                    .join("Library")
                    .join("Application Support")
                    .join("ezz")
                    .join("passwords.json"),
                log_file: home
                    .join("Library")
                    .join("Logs")
                    .join("ezz")
                    .join("ezz.log"),
            })
        }

        #[cfg(target_os = "windows")]
        {
            let roaming = PathBuf::from(std::env::var_os("APPDATA").ok_or("APPDATA is not set")?);
            let local =
                PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?);
            Ok(Self {
                password_database: roaming.join("ezz").join("passwords.json"),
                log_file: local.join("ezz").join("logs").join("ezz.log"),
            })
        }
    }
}

pub fn initialize_logging(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let parent = path.parent().ok_or("log file has no parent directory")?;
    fs::create_dir_all(parent)?;
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    WriteLogger::init(LevelFilter::Info, Config::default(), file)?;
    info!("ezz {} started", env!("CARGO_PKG_VERSION"));
    Ok(())
}

pub fn notify_started(count: usize) {
    let body = if count == 1 {
        "Extracting 1 file".to_owned()
    } else {
        format!("Extracting {count} files in order")
    };
    show_notification("ezz", &body);
}

pub fn finish_batch(report: &BatchReport) {
    let succeeded = report
        .files
        .iter()
        .filter(|outcome| outcome.result.is_ok())
        .count();
    let failed = report.files.len() - succeeded;
    let warnings = report
        .files
        .iter()
        .filter_map(|outcome| outcome.result.as_ref().ok())
        .map(|outcome| outcome.warnings.len())
        .sum::<usize>();

    for file in &report.files {
        match &file.result {
            Ok(outcome) => {
                info!(
                    "extracted {} to {}",
                    outcome.input.display(),
                    outcome.output.display()
                );
                for warning in &outcome.warnings {
                    log_warning(warning);
                }
            }
            Err(extraction_error) => {
                error!(
                    "failed to extract {}: {extraction_error}",
                    file.input.display()
                );
            }
        }
    }

    let mut body = format!("{succeeded} succeeded, {failed} failed");
    if warnings > 0 {
        body.push_str(&format!(", {warnings} warnings"));
    }
    show_notification("Extraction complete", &body);
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
    }
}

fn show_notification(summary: &str, body: &str) {
    if let Err(notification_error) = Notification::new()
        .appname("ezz")
        .summary(summary)
        .body(body)
        .show()
    {
        warn!("could not show desktop notification: {notification_error}");
    }
}
