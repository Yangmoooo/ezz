use log::{error, info};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[cfg(target_os = "linux")]
use super::arch::linux::*;
#[cfg(target_os = "windows")]
use super::arch::windows::*;
use super::sevenz::ExitCode;
use super::ExtractRes;
use crate::error::EzzError as Error;

pub fn is_stego(file: &Path) -> bool {
    matches!(
        file.extension().and_then(|ext| ext.to_str()),
        Some("mp4") | Some("mkv")
    )
}

pub fn setup_7zz() -> Result<String, Error> {
    let sevenz = "zz"; // 正式环境中应为 7z
    let sevenzz_path = env::current_exe()?.with_file_name(SEVENZZ);
    if Command::new(sevenz).arg("--help").status().is_ok() {
        Ok(sevenz.to_string())
    } else {
        if !sevenzz_path.exists() {
            File::create(&sevenzz_path)?.write_all(EMBEDDED_7Z)?;
        }
        Ok(sevenzz_path.to_string_lossy().into_owned())
    }
}

pub fn teardown_7zz() -> Result<(), Error> {
    let sevenzz_path = env::current_exe()?.with_file_name(SEVENZZ);
    if sevenzz_path.exists() {
        fs::remove_file(sevenzz_path)?;
    }
    Ok(())
}

pub fn handle_output(output: Output) -> Result<(), Error> {
    let exit_code = output
        .status
        .code()
        .ok_or(Error::SevenzError(ExitCode::UserStopped))?;
    match ExitCode::try_from(exit_code) {
        Ok(ExitCode::NoError) => {
            info!("7-Zip extract success");
            Ok(())
        }
        Ok(code) => {
            let stderr = normalize_stderr(decode_7z_output(&output.stderr));
            if code == ExitCode::FatalError && stderr.contains("Wrong password") {
                Err(Error::WrongPassword)
            } else {
                error!("7-Zip stderr: {stderr}");
                Err(Error::SevenzError(code))
            }
        }
        Err(_) => Err(Error::InvalidExitCode),
    }
}

pub fn normalize_stderr(stderr: String) -> String {
    stderr
        .trim_end_matches('\n')
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn derive_dir(archive: &Path) -> Result<PathBuf, Error> {
    let archive_stem = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(Error::FileNameError)?;
    let dir = archive
        .parent()
        .ok_or(Error::FilePathError)?
        .join(archive_stem);
    Ok(dir)
}

pub fn delete_dir(dir: &Path) -> Result<(), Error> {
    if dir.exists() && dir.is_dir() {
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn flatten_dir(dir: &Path) -> Result<ExtractRes, Error> {
    if !dir.is_dir() {
        return Err(Error::FilePathError);
    }
    let parent = dir.parent().ok_or(Error::FilePathError)?;
    let entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();

    if entries.len() <= 2 {
        for entry in &entries {
            let new_path = parent.join(entry.file_name().ok_or(Error::FileNameError)?);
            fs::rename(entry, new_path)?;
        }
        fs::remove_dir(dir)?;
    }

    let file_name = entries[0]
        .file_name()
        .ok_or(Error::FileNameError)?
        .to_string_lossy()
        .into_owned();
    Ok(ExtractRes {
        first_file: file_name,
        file_count: entries.len(),
    })
}
