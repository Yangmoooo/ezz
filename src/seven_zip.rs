use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::workflow::ExtractionError;

pub(crate) struct SevenZip {
    executable: PathBuf,
}

impl SevenZip {
    pub(crate) fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub(crate) fn probe(&self, input: &Path) -> Result<(), ExtractionError> {
        let mut command = Command::new(&self.executable);
        command
            .arg("l")
            .args(["-slt", "-ba", "-p", "-bsp0", "-sccUTF-8", "-scsUTF-8"])
            .arg(input);
        let output = command
            .output()
            .map_err(|error| ExtractionError::EngineLaunch {
                path: self.executable.clone(),
                message: error.to_string(),
            })?;

        if output.status.success() {
            return Ok(());
        }

        let message = output_message(&output);
        if is_wrong_password(&message) {
            Err(ExtractionError::WrongPassword)
        } else if message.contains("Cannot open the file as archive") {
            Err(ExtractionError::UnsupportedInput(input.to_path_buf()))
        } else {
            Err(ExtractionError::EngineFailed {
                operation: "list",
                exit_code: output.status.code(),
                message,
            })
        }
    }

    pub(crate) fn embedded_archive(
        &self,
        input: &Path,
    ) -> Result<Option<PathBuf>, ExtractionError> {
        let mut command = Command::new(&self.executable);
        command
            .arg("l")
            .args([
                "-t#",
                "-slt",
                "-ba",
                "-p",
                "-bsp0",
                "-sccUTF-8",
                "-scsUTF-8",
            ])
            .arg(input);
        let output = command
            .output()
            .map_err(|error| ExtractionError::EngineLaunch {
                path: self.executable.clone(),
                message: error.to_string(),
            })?;

        if !output.status.success() {
            let message = output_message(&output);
            if message.contains("Cannot open the file as archive") {
                return Ok(None);
            }
            return Err(ExtractionError::EngineFailed {
                operation: "scan embedded data in",
                exit_code: output.status.code(),
                message,
            });
        }

        Ok(find_embedded_archive(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    pub(crate) fn extract_embedded_archive(
        &self,
        input: &Path,
        output_dir: &Path,
        embedded: &Path,
    ) -> Result<PathBuf, ExtractionError> {
        let mut output_switch = OsString::from("-o");
        output_switch.push(output_dir);
        let mut command = Command::new(&self.executable);
        command
            .arg("x")
            .arg("-t#")
            .arg(output_switch)
            .args(["-y", "-aoa", "-bso0", "-bsp0", "-sccUTF-8", "-scsUTF-8"])
            .arg(input)
            .arg(embedded);
        let output = command
            .output()
            .map_err(|error| ExtractionError::EngineLaunch {
                path: self.executable.clone(),
                message: error.to_string(),
            })?;

        if !output.status.success() {
            return Err(ExtractionError::EngineFailed {
                operation: "extract embedded archive from",
                exit_code: output.status.code(),
                message: output_message(&output),
            });
        }

        Ok(output_dir.join(embedded))
    }

    pub(crate) fn test_password(
        &self,
        input: &Path,
        password: &str,
    ) -> Result<(), ExtractionError> {
        let mut command = Command::new(&self.executable);
        command
            .arg("t")
            .arg(password_switch(password))
            .args(["-bso0", "-bsp0", "-sccUTF-8", "-scsUTF-8"])
            .arg(input);
        let output = command
            .output()
            .map_err(|error| ExtractionError::EngineLaunch {
                path: self.executable.clone(),
                message: error.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            let message = output_message(&output);
            if is_wrong_password(&message) {
                Err(ExtractionError::WrongPassword)
            } else {
                Err(ExtractionError::EngineFailed {
                    operation: "test",
                    exit_code: output.status.code(),
                    message,
                })
            }
        }
    }

    pub(crate) fn validate_paths(
        &self,
        input: &Path,
        password: &str,
    ) -> Result<(), ExtractionError> {
        let mut command = Command::new(&self.executable);
        command
            .arg("l")
            .args(["-slt", "-ba"])
            .arg(password_switch(password))
            .args(["-bsp0", "-sccUTF-8", "-scsUTF-8"])
            .arg(input);
        let output = command
            .output()
            .map_err(|error| ExtractionError::EngineLaunch {
                path: self.executable.clone(),
                message: error.to_string(),
            })?;

        if output.status.success() {
            validate_listed_paths(&String::from_utf8_lossy(&output.stdout))
        } else {
            let message = output_message(&output);
            if is_wrong_password(&message) {
                Err(ExtractionError::WrongPassword)
            } else {
                Err(ExtractionError::EngineFailed {
                    operation: "validate archive paths in",
                    exit_code: output.status.code(),
                    message,
                })
            }
        }
    }

    pub(crate) fn extract(
        &self,
        input: &Path,
        output_dir: &Path,
        password: &str,
    ) -> Result<(), ExtractionError> {
        let mut output_switch = OsString::from("-o");
        output_switch.push(output_dir);
        let mut command = Command::new(&self.executable);
        command
            .arg("x")
            .arg(output_switch)
            .arg(password_switch(password))
            .args([
                "-y",
                "-aoa",
                "-spe",
                "-bso0",
                "-bsp0",
                "-sccUTF-8",
                "-scsUTF-8",
            ])
            .arg(input);
        let output = command
            .output()
            .map_err(|error| ExtractionError::EngineLaunch {
                path: self.executable.clone(),
                message: error.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            let message = output_message(&output);
            if message.contains("Dangerous link path was ignored") {
                Err(ExtractionError::UnsafeOutput {
                    path: input.to_path_buf(),
                    reason: message,
                })
            } else {
                Err(ExtractionError::EngineFailed {
                    operation: "extract",
                    exit_code: output.status.code(),
                    message,
                })
            }
        }
    }
}

fn find_embedded_archive(output: &str) -> Option<PathBuf> {
    let mut path: Option<PathBuf> = None;
    let mut archive_type = None;
    let mut offset = None;

    for line in output.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if offset.is_some_and(|offset| offset > 0)
                && archive_type.is_some_and(is_supported_embedded_type)
                && let Some(path) = path.take()
                && is_safe_relative_path(&path)
            {
                return Some(path);
            }
            path = None;
            archive_type = None;
            offset = None;
            continue;
        }

        if let Some(value) = line.strip_prefix("Path = ") {
            path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("Type = ") {
            archive_type = Some(value);
        } else if let Some(value) = line.strip_prefix("Offset = ") {
            offset = value.parse::<u64>().ok();
        }
    }

    None
}

fn is_supported_embedded_type(archive_type: &str) -> bool {
    matches!(
        archive_type.to_ascii_lowercase().as_str(),
        "zip" | "7z" | "rar" | "rar5"
    )
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn validate_listed_paths(output: &str) -> Result<(), ExtractionError> {
    for path in output
        .lines()
        .filter_map(|line| line.strip_prefix("Path = "))
    {
        if is_unsafe_archive_path(path) {
            return Err(ExtractionError::UnsafeOutput {
                path: PathBuf::from(path),
                reason: "archive entry escapes the extraction directory".to_owned(),
            });
        }
    }
    Ok(())
}

fn is_unsafe_archive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.is_empty()
        || path.starts_with(['/', '\\'])
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || path.split(['/', '\\']).any(|component| component == "..")
}

fn password_switch(password: &str) -> OsString {
    let mut switch = OsString::from("-p");
    switch.push(password);
    switch
}

fn is_wrong_password(message: &str) -> bool {
    message.contains("Wrong password?") || message.contains("Wrong password")
}

fn output_message(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}
