use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionOutcome {
    pub input: PathBuf,
    pub output: PathBuf,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExtractionError {
    #[error("Input does not exist: {0}")]
    InputNotFound(PathBuf),

    #[error("Input is not a file: {0}")]
    InputNotFile(PathBuf),

    #[error("Input is not a supported archive: {0}")]
    UnsupportedInput(PathBuf),
}

#[derive(Debug, Default)]
pub struct ExtractionWorkflow;

impl ExtractionWorkflow {
    pub fn new() -> Self {
        Self
    }

    pub fn extract(&self, input: impl AsRef<Path>) -> Result<ExtractionOutcome, ExtractionError> {
        let input = input.as_ref();
        if !input.exists() {
            return Err(ExtractionError::InputNotFound(input.to_path_buf()));
        }
        if !input.is_file() {
            return Err(ExtractionError::InputNotFile(input.to_path_buf()));
        }
        Err(ExtractionError::UnsupportedInput(input.to_path_buf()))
    }
}
