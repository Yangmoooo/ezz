#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod cli;
mod decompress;
mod error;
#[macro_use]
mod notify;

use clap::Parser;
use log::{error, info, LevelFilter};
use simplelog::{ConfigBuilder, WriteLogger};
use std::env;
use std::fs::File;

use cli::Args;
use decompress::{extract, ExtractRes};
use error::EzzError as Error;
use notify::Msg;

fn main() {
    if let Err(e) = init_logger() {
        notify!(Msg::Err, "初始化日志失败：\n{e:?}");
        return;
    }

    match run() {
        Ok(res) => {
            notify!(
                Msg::Ok,
                "解压成功：\n{} 等 {} 个文件已提取",
                res.first_file,
                res.file_count
            );
            info!(
                "Done. {} and other {} files extracted",
                res.first_file,
                res.file_count - 1
            );
        }
        Err(e) => {
            notify!(Msg::Err, "解压失败：\n{e}");
            match e {
                Error::Io(e) => error!("I/O: {e:?}"),
                Error::Log(e) => error!("Log: {e:?}"),
                Error::SevenzError(e) => error!("7zip: {e:?}"),
                Error::InvalidExitCode => error!("7zip: {e:?}"),
                _ => error!("{e:?}"),
            }
        }
    }
}

fn init_logger() -> Result<(), Error> {
    let log_config = ConfigBuilder::new()
        .set_time_offset_to_local()
        .expect("Failed to set log time offset")
        .build();
    let log_path = env::current_exe()?.with_file_name("ezz.log");
    WriteLogger::init(
        LevelFilter::Info,
        log_config,
        File::options().append(true).create(true).open(log_path)?,
    )?;
    Ok(())
}

fn run() -> Result<ExtractRes, Error> {
    let args = Args::parse();
    let archive = &args.archive;
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));

    notify!(Msg::Info, "解压开始：\n正在处理 {archive:?}");
    info!("ezz {version} started, processing: {archive:?}");

    extract(archive, args.pw.as_deref(), args.db.as_deref())
}
