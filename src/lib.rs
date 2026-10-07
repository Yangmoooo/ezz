#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
compile_error!("ezz only supports Windows x64");

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
