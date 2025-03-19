use log::{error, info};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::process::{Command, Output};

use super::Archive;
#[cfg(target_os = "linux")]
use super::arch::linux::*;
#[cfg(target_os = "windows")]
use super::arch::windows::exe::*;
use crate::types::{EzzError, EzzResult};

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
    type Error = EzzError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ExitCode::NoError),
            1 => Ok(ExitCode::Warning),
            2 => Ok(ExitCode::FatalError),
            7 => Ok(ExitCode::CmdLineError),
            8 => Ok(ExitCode::NotEnoughMem),
            255 => Ok(ExitCode::UserStopped),
            _ => Err(EzzError::InvalidExitCode),
        }
    }
}

pub fn setup_7zz() -> EzzResult<String> {
    let zz_path = env::current_exe()?.with_file_name(SEVENZZ);
    if !zz_path.exists() {
        let mut sevenzz = File::create(&zz_path)?;
        sevenzz.write_all(EMBEDDED_7Z)?;
        set_exemode(&zz_path)?;
    }
    Ok(zz_path.to_string_lossy().into_owned())
}

pub fn teardown_7zz() -> EzzResult<()> {
    let zz_path = env::current_exe()?.with_file_name(SEVENZZ);
    if zz_path.exists() {
        fs::remove_file(zz_path)?;
    }
    Ok(())
}

pub fn command_t(zz: &str, archive: &Archive, pwd: &str) -> EzzResult<()> {
    let pwd_switch = format!("-p{}", pwd);
    let archive_name = archive.get_path().to_string_lossy().into_owned();
    let mut cmd = Command::new(zz);
    cmd.arg("t")
        .arg(&pwd_switch)
        .args(["-bso0", "-bsp0"])
        .args(["-sccUTF-8", "-scsUTF-8"])
        .arg(&archive_name);
    set_creation_flags(&mut cmd);
    handle_output(cmd.output()?)
}

pub fn command_x(zz: &str, archive: &Archive, pwd: &str) -> EzzResult<()> {
    let dir = archive.derive_dir()?;
    let output_switch = format!("-o{}", dir.to_string_lossy().into_owned());
    let pwd_switch = format!("-p{}", pwd);
    let archive_name = archive.get_path().to_string_lossy().into_owned();
    let mut cmd = Command::new(zz);
    cmd.arg("x")
        .args([&output_switch, &pwd_switch])
        .arg("-aoa")
        .arg("-spe")
        .args(["-bso0", "-bsp0"])
        .args(["-sccUTF-8", "-scsUTF-8"])
        .arg(&archive_name);
    set_creation_flags(&mut cmd);
    handle_output(cmd.output()?)
}

pub fn command_for_stego(zz: &str, video: &Archive) -> EzzResult<()> {
    let parent = video.get_parent()?;
    let output_switch = format!("-o{}", parent.to_string_lossy().into_owned());
    let video_name = video.get_path().to_string_lossy().into_owned();
    let mut cmd = Command::new(zz);
    cmd.arg("x")
        .arg(&output_switch)
        .arg("-t#")
        .arg("-aoa")
        .args(["-bso0", "-bsp0"])
        .args(["-sccUTF-8", "-scsUTF-8"])
        .args([&video_name, "2.zip"]);
    set_creation_flags(&mut cmd);
    handle_output(cmd.output()?)
}

fn handle_output(output: Output) -> EzzResult<()> {
    let exit_code = output
        .status
        .code()
        .ok_or(EzzError::Sevenzip(ExitCode::UserStopped))?;
    match ExitCode::try_from(exit_code) {
        Ok(ExitCode::NoError) => {
            info!("7-Zip t/x success");
            Ok(())
        }
        Ok(code) => {
            let stderr = normalize_stderr(String::from_utf8_lossy(&output.stderr).into_owned());
            if code == ExitCode::FatalError && stderr.contains("Wrong password") {
                Err(EzzError::WrongPassword)
            } else {
                error!("7-Zip stderr: {stderr}");
                Err(EzzError::Sevenzip(code))
            }
        }
        Err(_) => Err(EzzError::InvalidExitCode),
    }
}

fn normalize_stderr(stderr: String) -> String {
    stderr
        .trim_end_matches('\n')
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ")
}
