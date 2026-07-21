#[cfg(not(any(target_os = "windows", target_os = "macos")))]
compile_error!("ezz v3 only supports Windows and macOS");

mod application;
mod workflow;

pub use application::{BatchReport, DesktopApplication, FileOutcome};
pub use workflow::{ExtractionError, ExtractionOutcome, ExtractionWorkflow};
