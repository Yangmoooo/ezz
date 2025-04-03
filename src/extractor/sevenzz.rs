use log::{error, info};
use regex::Regex;
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};

use super::Archive;
#[cfg(target_os = "linux")]
use super::platform::linux::exe::*;
#[cfg(target_os = "windows")]
use super::platform::windows::exe::*;
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

pub struct Sevenzz(String);

impl Sevenzz {
    pub fn construct_from_embed() -> EzzResult<Self> {
        let zz_path = env::current_exe()?.with_file_name(SEVENZZ);
        if !zz_path.try_exists()? {
            let mut sevenzz = File::create(&zz_path)?;
            sevenzz.write_all(EMBEDDED_7Z)?;
            set_exemode(&zz_path)?;
        }
        Ok(Self(zz_path.to_string_lossy().into_owned()))
    }

    pub fn deconstruct(&self) -> EzzResult<()> {
        let zz_path = Path::new(&self.0);
        if zz_path.try_exists()? {
            fs::remove_file(zz_path)?;
        }
        Ok(())
    }

    // 返回压缩包内第一个非目录文件以供测试，若压缩包文件名被加密，则返回空字符串
    pub fn command_l(&self, archive: &Archive) -> EzzResult<String> {
        let archive_name = archive.get_path().to_string_lossy().into_owned();
        let mut cmd = Command::new(&self.0);
        cmd.arg("l")
            .arg("-p")
            .args(["-bse0", "-bsp0"])
            .args(["-sccUTF-8", "-scsUTF-8"])
            .arg(&archive_name);
        set_creation_flags(&mut cmd);
        Ok(find_first_file(cmd.output()?))
    }

    pub fn command_t(&self, archive: &Archive, pwd: &str, inner: &str) -> EzzResult<()> {
        let pwd_switch = format!("-p{}", pwd);
        let archive_name = archive.get_path().to_string_lossy().into_owned();
        let mut cmd = Command::new(&self.0);
        cmd.arg("t")
            .arg(&pwd_switch)
            .args(["-bso0", "-bsp0"])
            .args(["-sccUTF-8", "-scsUTF-8"])
            .arg(&archive_name);
        if !inner.is_empty() {
            cmd.arg(inner);
        }
        set_creation_flags(&mut cmd);
        check_torx_output(cmd.output()?, true)
    }

    pub fn command_x(&self, archive: &Archive, pwd: &str) -> EzzResult<()> {
        let dir = archive.derive_dir();
        let output_switch = format!("-o{}", dir.to_string_lossy().into_owned());
        let pwd_switch = format!("-p{}", pwd);
        let archive_name = archive.get_path().to_string_lossy().into_owned();
        let mut cmd = Command::new(&self.0);
        cmd.arg("x")
            .args([&output_switch, &pwd_switch])
            .arg("-aot")
            .arg("-spe")
            .args(["-bso0", "-bsp0"])
            .args(["-sccUTF-8", "-scsUTF-8"])
            .arg(&archive_name);
        set_creation_flags(&mut cmd);
        check_torx_output(cmd.output()?, false)
    }

    pub fn command_x_steganor(&self, video: &Archive) -> EzzResult<()> {
        let parent = video.get_parent()?;
        let output_switch = format!("-o{}", parent.to_string_lossy().into_owned());
        let video_name = video.get_path().to_string_lossy().into_owned();
        let mut cmd = Command::new(&self.0);
        cmd.arg("x")
            .arg(&output_switch)
            .arg("-t#")
            .arg("-aot")
            .args(["-bso0", "-bsp0"])
            .args(["-sccUTF-8", "-scsUTF-8"])
            .args([&video_name, "2.zip"]);
        set_creation_flags(&mut cmd);
        check_torx_output(cmd.output()?, false)
    }
}

fn find_first_file(output: Output) -> String {
    if output.status.code() != Some(0) {
        return String::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let re =
        Regex::new(r"^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}) (.{5}) +(\d+) +(\d+|\s*) +(.+)$")
            .unwrap();

    let mut in_file_list = false;
    for line in stdout.lines() {
        if line.starts_with("-------------------") {
            in_file_list = !in_file_list;
            continue;
        }
        if !in_file_list {
            continue;
        }
        if let Some(caps) = re.captures(line) {
            if let (Some(attr), Some(size), Some(file_name)) =
                (caps.get(3), caps.get(4), caps.get(6))
            {
                if !attr.as_str().starts_with('D') && size.as_str() != "0" {
                    return file_name.as_str().trim().to_owned();
                }
            }
        }
    }
    String::new()
}

fn check_torx_output(output: Output, is_test: bool) -> EzzResult<()> {
    let exit_code = output
        .status
        .code()
        .ok_or(EzzError::Sevenzip(ExitCode::UserStopped))?;
    let cmd = if is_test { "Test" } else { "eXtract" };
    match ExitCode::try_from(exit_code) {
        Ok(ExitCode::NoError) => {
            info!("7-Zip {cmd} successful");
            Ok(())
        }
        Ok(code) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if code == ExitCode::FatalError && stderr.contains("Wrong password") {
                Err(EzzError::WrongPassword)
            } else {
                error!("7-Zip {cmd} failed, stderr: {stderr}");
                Err(EzzError::Sevenzip(code))
            }
        }
        Err(_) => Err(EzzError::InvalidExitCode),
    }
}

impl fmt::Debug for Sevenzz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
