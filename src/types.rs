use thiserror::Error;

use crate::unpack::sevenzz::ExitCode;

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

    #[error("7-Zip 退出码 {0:?}")]
    Sevenzip(ExitCode),

    #[error("7-Zip 退出码无效")]
    InvalidExitCode,

    #[error("密码错误")]
    WrongPassword,

    #[error("无匹配密码")]
    NoMatchedPassword,

    #[error("密码库格式错误")]
    VaultError,

    #[error("文件路径或文件名错误")]
    PathError,
}

pub type EzzResult<T> = Result<T, EzzError>;
