use thiserror::Error;

use crate::decompress::sevenzip::ExitCode;

#[derive(Error, Debug)]
pub enum EzzError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Log(#[from] log::SetLoggerError),
    #[error("7-Zip 退出码 {0:?}")]
    Sevenzip(ExitCode),
    #[error("7-Zip 退出码无效")]
    InvalidExitCode,
    #[error("密码错误")]
    WrongPassword,
    #[error("未找到密码库")]
    PasswordDbNotFound,
    #[error("密码库中无匹配密码")]
    NoMatchedPassword,
    #[error("文件路径或文件名错误")]
    PathError,
}

pub type EzzResult<T> = Result<T, EzzError>;
