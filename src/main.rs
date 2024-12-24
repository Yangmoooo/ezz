#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod cli;
mod decompress;
mod types;
#[macro_use]
mod notify;

use clap::Parser;
use log::{error, info, LevelFilter};
use simplelog::{ConfigBuilder, WriteLogger};
use std::env;
use std::fs::File;

use cli::Args;
use decompress::extract;
use notify::Msg;
use types::{EzzError, EzzResult};

fn main() {
    if let Err(e) = init_logger() {
        notify!(Msg::Err, "初始化日志失败：\n{e:?}");
        return;
    }

    match run() {
        Ok(filename) => {
            notify!(Msg::Ok, "解压成功：\n已保存至 {filename}",);
            info!("Done. Saved to {filename:?}");
        }
        Err(e) => {
            notify!(Msg::Err, "解压失败：\n{e}");
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
    let archive = &args.archive;
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));

    notify!(Msg::Info, "开始解压：\n正在处理文件 {archive:?}");
    info!("ezz {version} started, processing: {archive:?}");

    extract(archive, args.pw.as_deref(), args.db.as_deref())
}
