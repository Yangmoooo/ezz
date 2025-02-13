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

    match args.action {
        Action::Extract { archive, pwd, db } => {
            let archive = &archive;
            let version = format!("v{}", env!("CARGO_PKG_VERSION"));

            notify!(Msg::Info, "开始解压······\n正在处理文件 {archive:?}");
            info!("ezz {version} started, processing: {archive:?}");

            extract(archive, pwd.as_deref(), db.as_deref())
        }
        // 将密码添加到数据库里，成功返回空字符串，区别于解压得到的文件名
        Action::Add { pwd, db } => {
            let version = format!("v{}", env!("CARGO_PKG_VERSION"));
            let db = match db {
                Some(path) => path,
                None => locate_db()?,
            };
            let mut file = OpenOptions::new().create(true).append(true).open(&db)?;
            writeln!(file, "0,{pwd}")?;

            notify!(Msg::Info, "密码添加成功！\n已保存至 {db:?}");
            info!("ezz {version} added password: {pwd:?} to {db:?}");

            EzzResult::Ok("".to_string())
        }
    }
}
