#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod cli;
mod types;
mod unpack;
#[macro_use]
mod notify;

use clap::Parser;
use log::{error, info, LevelFilter};
use simplelog::{ConfigBuilder, WriteLogger};
use std::env;
use std::fs::{File, OpenOptions};
use std::io::Write;

use cli::{Action, Args};
use notify::Msg;
use types::{EzzError, EzzResult};
use unpack::{extract, locate_db};

fn main() {
    if let Err(e) = init_logger() {
        notify!(Msg::Err, "初始化日志失败！\n{e:?}");
        return;
    }

    match run() {
        Ok(filename) => {
            if !filename.is_empty() {
                notify!(Msg::Ok, "解压成功！\n已保存至 {filename}",);
                info!("Done. Saved to {filename:?}");
            }
        }
        Err(e) => {
            notify!(Msg::Err, "解压失败！\n{e}");
            match e {
                EzzError::Io(e) => error!("I/O: {e:?}"),
                EzzError::Log(e) => error!("Log: {e:?}"),
                #[cfg(target_os = "windows")]
                EzzError::Ui(e) => error!("UI: {e:?}"),
                EzzError::Sevenzip(e) => error!("7zip: {e:?}"),
                EzzError::InvalidExitCode => error!("7zip: {e:?}"),
                _ => error!("{e:?}"),
            }
        }
    }
}

fn init_logger() -> EzzResult<()> {
    let log_config = ConfigBuilder::new()
        .set_time_offset_to_local()
        .expect("Failed to set log time offset")
        .build();
    let log_path = env::current_exe()?.with_file_name("ezz.log");
    WriteLogger::init(
        if cfg!(debug_assertions) {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        },
        log_config,
        File::options().append(true).create(true).open(log_path)?,
    )?;
    Ok(())
}

fn run() -> EzzResult<String> {
    let args = Args::parse();
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));

    match &args.action {
        // 将密码添加到数据库，成功时返回空字符串，区别于解压得到的文件名
        Some(Action::Add { pwd, db }) => {
            let db = match db {
                Some(path) => path,
                None => &locate_db()?,
            };
            let mut file = OpenOptions::new().create(true).append(true).open(db)?;
            writeln!(file, "0,{pwd}")?;

            notify!(Msg::Info, "密码添加成功！\n已保存至 {db:?}");
            info!("ezz {version} added password: {pwd:?} to {db:?}");

            EzzResult::Ok("".to_string())
        }
        // 不使用子命令时，默认将传入的参数作为压缩文件路径进行提取
        _ => {
            let archive = match &args.action {
                Some(Action::Extract { archive, .. }) => archive,
                None => args.archive.as_ref().ok_or(EzzError::PathError)?,
                _ => unreachable!(),
            };

            notify!(Msg::Info, "开始解压······\n正在处理文件 {archive:?}");
            info!("ezz {version} started, processing: {archive:?}");

            let (pwd, db) = match &args.action {
                Some(Action::Extract { pwd, db, .. }) => (pwd.as_deref(), db.as_deref()),
                None => (None, None),
                _ => unreachable!(),
            };

            extract(archive, pwd, db)
        }
    }
}
