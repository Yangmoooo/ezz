#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod cli;
mod extractor;
mod types;
#[macro_use]
mod notify;

use clap::Parser;
use log::{LevelFilter, error, info};
use named_lock::NamedLock;
use simplelog::{ConfigBuilder, WriteLogger, format_description};
use std::env;
use std::fs::File;

use cli::{Action, Args};
use extractor::{Archive, Vault};
use notify::Msg;
use types::{EzzError, EzzResult};

fn main() {
    let lock = match NamedLock::create("ezz") {
        Ok(lock) => lock,
        Err(e) => {
            notify!(Msg::Err, "进程锁创建失败！\n{e:?}");
            return;
        }
    };
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(named_lock::Error::WouldBlock) => {
            notify!(Msg::Err, "程序正在运行，请稍后再试！");
            return;
        }
        Err(e) => {
            notify!(Msg::Err, "进程锁定失败！\n{e:?}");
            return;
        }
    };

    if let Err(e) = init_logger() {
        notify!(Msg::Err, "初始化日志失败！\n{e:?}");
        return;
    }

    match run() {
        Ok(filename) => {
            if !filename.is_empty() {
                notify!(Msg::Ok, "解压成功！\n已保存至：{filename}");
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
        .set_time_format_custom(format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second]"
        ))
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

    match args.action {
        // 将密码添加到数据库，成功时返回空字符串，区别于解压得到的文件名
        Some(Action::Add { pwd, vault }) => {
            let vault = vault.map(Vault::new).unwrap_or_default();
            if !vault.exists() {
                vault.init()?;
                info!("ezz {version} created vault: {vault:?}");
            }

            vault.add(&pwd)?;

            notify!(Msg::Ok, "密码添加成功！\n保管库位于 {vault:?}");
            info!("ezz {version} added password: {pwd:?} to {vault:?}");

            EzzResult::Ok("".to_string())
        }
        // 不使用子命令时，默认将传入的参数作为压缩文件路径进行提取
        _ => {
            let (archive_path, pwd, vault_path) = match args.action {
                Some(Action::Extract {
                    archive,
                    pwd,
                    vault,
                }) => (archive, pwd, vault),
                None => (args.archive.ok_or(EzzError::PathError)?, None, None),
                _ => unreachable!(),
            };

            let archive = Archive::new(archive_path);
            let vault = vault_path.map(Vault::new).unwrap_or_default();
            if !vault.exists() {
                vault.init()?;
                info!("ezz {version} created vault: {vault:?}");
            }

            notify!(Msg::Info, "开始解压······\n正在处理文件 {archive:?}");
            info!("ezz {version} started, processing: {archive:?}");

            archive.extract(pwd.as_deref(), &vault)
        }
    }
}
