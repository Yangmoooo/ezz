#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
)))]
compile_error!("ezz v3 only supports Windows and macOS");

mod engine;
mod explorer;
mod password_store;
mod process;
mod seven_zip;
mod workflow;

/// `cargo xtask prepare` 下载的引擎版本；测试从这里取，避免多处硬编码各说各话。
pub const SEVEN_ZIP_VERSION: &str = "26.02";

pub use engine::locate_engine;
pub use workflow::{
    EngineOperation, ExtractionError, ExtractionOutcome, ExtractionWarning, ExtractionWorkflow,
    PasswordPrompt, PasswordResponse,
};
