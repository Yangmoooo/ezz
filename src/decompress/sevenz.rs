use std::path::Path;
use std::process::{Command, Output};

#[cfg(target_os = "linux")]
use super::arch::linux::set_creation_flags;
#[cfg(target_os = "windows")]
use super::arch::windows::set_creation_flags;
use super::utils::derive_dir;
use crate::error::EzzError as Error;

#[derive(Debug, PartialEq)]
pub enum ExitCode {
    NoError = 0,
    Warning = 1,
    FatalError = 2,
    CmdLineError = 7,
    NotEnoughMem = 8,
    UserStopped = 255,
}

impl TryFrom<i32> for ExitCode {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ExitCode::NoError),
            1 => Ok(ExitCode::Warning),
            2 => Ok(ExitCode::FatalError),
            7 => Ok(ExitCode::CmdLineError),
            8 => Ok(ExitCode::NotEnoughMem),
            255 => Ok(ExitCode::UserStopped),
            _ => Err(Error::InvalidExitCode),
        }
    }
}

pub fn command_x(zz: &str, archive: &Path, pw: &str) -> Result<Output, Error> {
    let dir = derive_dir(archive)?;
    let output_switch = format!("-o{}", dir.to_string_lossy().into_owned());
    let pw_switch = format!("-p{}", pw);
    let archive_name = archive.to_string_lossy().into_owned();
    let mut cmd = Command::new(zz);
    cmd.arg("x")
        .args([&output_switch, &pw_switch])
        .args(["-aoa", "-sdel", "-spe"])
        .arg(&archive_name);
    set_creation_flags(&mut cmd);
    Ok(cmd.output()?)
}

pub fn command_for_stego(zz: &str, video: &Path) -> Result<Output, Error> {
    let dir = video.parent().ok_or(Error::FilePathError)?;
    let output_switch = format!("-o{}", dir.to_string_lossy().into_owned());
    let video_name = video.to_string_lossy().into_owned();
    let mut cmd = Command::new(zz);
    cmd.arg("x")
        .arg(&output_switch)
        .args(["-aoa", "-sdel", "-t#"])
        .args([&video_name, "2.zip"]);
    set_creation_flags(&mut cmd);
    Ok(cmd.output()?)
}
