use encoding_rs::{GB18030, UTF_8};
use std::io;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use windows::Win32::Globalization::GetACP;

pub const EMBEDDED_7Z: &[u8] = include_bytes!("../../../../assets/7zip/7zz.exe");
pub const SEVENZZ: &str = "7zz.exe";

pub fn decode_7z_output(input: &[u8]) -> String {
    let codepage = unsafe { GetACP() };
    let encoding = match codepage {
        936 => GB18030,
        _ => UTF_8,
    };
    let (decoded, _, _) = encoding.decode(input);
    decoded.into_owned()
}

pub fn set_creation_flags(cmd: &mut Command) {
    cmd.creation_flags(0x08000000);
}

pub fn set_exemode(_file: &Path) -> io::Result<()> {
    Ok(())
}
