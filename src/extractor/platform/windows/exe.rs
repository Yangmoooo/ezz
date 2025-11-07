use std::io;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

pub const EMBEDDED_7Z: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/7zz.exe"));
pub const SEVENZZ: &str = "7zz.exe";

pub fn set_creation_flags(cmd: &mut Command) {
    cmd.creation_flags(0x08000000);
}

pub fn set_exemode(_file: &Path) -> io::Result<()> {
    Ok(())
}
