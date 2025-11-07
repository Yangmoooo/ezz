use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

pub const EMBEDDED_7Z: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/7zz"));
pub const SEVENZZ: &str = "7zz";

pub fn set_creation_flags(_cmd: &mut Command) {}

pub fn set_exemode(file: &Path) -> io::Result<()> {
    let mut perms = file.metadata()?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(file, perms)
}
