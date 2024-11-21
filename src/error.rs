use thiserror::Error;

use crate::decompress::sevenz::ExitCode;

#[derive(Error, Debug)]
pub enum EzzError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Log(#[from] log::SetLoggerError),
    #[error("退出码 {0:?}")]
    SevenzError(ExitCode),
    #[error("无效的退出码")]
    InvalidExitCode,
    #[error("密码错误")]
    WrongPassword,
    #[error("未找到密码库")]
    PasswordDbNotFound,
    #[error("密码库中无匹配密码")]
    NoMatchedPassword,
    #[error("文件名错误")]
    FileNameError,
    #[error("文件路径错误")]
    FilePathError,
}
