//! 引擎定位：`EZZ_7ZZ` 环境变量 → 与 `ezz` 可执行文件同目录。启动时解析并校验一次。

use std::path::PathBuf;

/// 覆盖引擎路径的环境变量。
pub const OVERRIDE_VARIABLE: &str = "EZZ_7ZZ";

/// 发布物里与主程序并列的引擎文件名。
#[cfg(windows)]
pub const ENGINE_FILE_NAME: &str = "7zz.exe";
#[cfg(not(windows))]
pub const ENGINE_FILE_NAME: &str = "7zz";

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("could not locate the Ezz executable: {0}")]
    ExecutableLocation(String),
    #[error("EZZ_7ZZ points to a path that is not a file: {path}", path = .path.display())]
    OverrideNotAFile { path: PathBuf },
    #[error(
        "the 7-Zip engine is missing: {path} (expected next to the ezz executable, or set EZZ_7ZZ)",
        path = .path.display()
    )]
    NotFound { path: PathBuf },
}

/// 解析并校验引擎路径。
///
/// `EZZ_7ZZ` 一旦设置就以它为准，不静默回退到同目录。
pub fn locate_engine() -> Result<PathBuf, EngineError> {
    if let Some(value) = std::env::var_os(OVERRIDE_VARIABLE) {
        let path = PathBuf::from(value);
        if !path.is_file() {
            return Err(EngineError::OverrideNotAFile { path });
        }
        return Ok(path);
    }

    let executable = std::env::current_exe()
        .map_err(|error| EngineError::ExecutableLocation(error.to_string()))?;
    let directory = executable.parent().ok_or_else(|| {
        EngineError::ExecutableLocation("executable has no parent directory".to_owned())
    })?;
    let path = directory.join(ENGINE_FILE_NAME);
    if !path.is_file() {
        return Err(EngineError::NotFound { path });
    }

    Ok(path)
}
