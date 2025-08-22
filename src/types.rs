use thiserror::Error;

use crate::extractor::sevenzz::ExitCode;

#[derive(Error, Debug)]
pub enum EzzError {
    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Log(#[from] log::SetLoggerError),

    #[cfg(target_os = "windows")]
    #[error("{0}")]
    Ui(#[from] native_windows_gui::NwgError),

    #[error("{0}")]
    Trash(#[from] trash::Error),

    #[error("{0}")]
    NamedLock(#[from] named_lock::Error),

    #[error("7-Zip ExitCode {0:?}.")]
    Sevenzip(ExitCode),

    #[error("7-Zip ExitCode Invalid.")]
    InvalidExitCode,

    #[error("Wrong Password.")]
    WrongPassword,

    #[error("No Matched Password.")]
    NoMatchedPassword,

    #[error("Wordlist Error.")]
    WordlistError,

    #[error("File Path Error.")]
    PathError,
}

pub type EzzResult<T> = Result<T, EzzError>;
