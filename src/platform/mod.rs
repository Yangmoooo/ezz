mod common;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{run, show_fatal_error};
#[cfg(target_os = "windows")]
pub use windows::{run, show_fatal_error};
