use log::{error, info};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};

#[cfg(target_os = "linux")]
use super::arch::linux::*;
#[cfg(target_os = "windows")]
use super::arch::windows::*;
use super::cleanup::derive_dir;
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

pub fn command_t(zz: &str, archive: &Path, pwd: &str) -> EzzResult<Output> {
    let pwd_switch = format!("-p{}", pwd);
    let archive_name = archive.to_string_lossy().into_owned();
    let mut cmd = Command::new(zz);
    cmd.arg("t")
        .arg(&pwd_switch)
        .args(["-bso0", "-bsp0"])
        .arg(&archive_name);
    set_creation_flags(&mut cmd);
    Ok(cmd.output()?)
}

pub fn command_x(zz: &str, archive: &Path, pwd: &str) -> EzzResult<Output> {
    let dir = derive_dir(archive)?;
    let output_switch = format!("-o{}", dir.to_string_lossy().into_owned());
    let pwd_switch = format!("-p{}", pwd);
    let archive_name = archive.to_string_lossy().into_owned();
    let mut cmd = Command::new(zz);
    cmd.arg("x")
        .args([&output_switch, &pwd_switch])
        .args(["-aoa", "-spe", "-bso0", "-bsp0"])
        .arg(&archive_name);
    set_creation_flags(&mut cmd);
    Ok(cmd.output()?)
}

pub fn command_for_stego(zz: &str, video: &Path) -> EzzResult<Output> {
    let dir = video.parent().ok_or(EzzError::PathError)?;
    let output_switch = format!("-o{}", dir.to_string_lossy().into_owned());
    let video_name = video.to_string_lossy().into_owned();
    let mut cmd = Command::new(zz);
    cmd.arg("x")
        .arg(&output_switch)
        .args(["-aoa", "-t#", "-bso0", "-bsp0"])
        .args([&video_name, "2.zip"]);
    set_creation_flags(&mut cmd);
    Ok(cmd.output()?)
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

pub fn handle_output(output: Output) -> EzzResult<()> {
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
            let stderr = normalize_stderr(decode_7z_output(&output.stderr));
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
