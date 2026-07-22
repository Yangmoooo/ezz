#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
)))]
compile_error!("ezz v3 only supports Windows and macOS");

mod application;
mod password_store;
mod seven_zip;
mod workflow;

pub use application::{BatchReport, DesktopApplication, FileOutcome};
pub use workflow::{
    ExtractionError, ExtractionOutcome, ExtractionWarning, ExtractionWorkflow, PasswordPrompt,
    PasswordResponse,
};
