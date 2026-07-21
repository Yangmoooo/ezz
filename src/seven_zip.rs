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
