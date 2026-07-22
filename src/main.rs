#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod platform;

fn main() {
    if let Err(error) = platform::run() {
        platform::show_fatal_error(&error.to_string());
    }
}
