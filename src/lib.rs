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

pub use engine::locate_engine;
pub use workflow::{
    EngineOperation, ExtractionError, ExtractionOutcome, ExtractionWarning, ExtractionWorkflow,
    PasswordPrompt, PasswordResponse,
};
