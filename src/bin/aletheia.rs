#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::{env, fs, vec};

const VERSION: &str = "0.1.1"; // 修改时注意同步 workflow 中的附件名称

// apate 会将一个视频文件（称为面具）覆盖真实文件的开头
// 被覆盖的部分会按字节反转后附加到文件末尾
// 最后再加上一个 4 字节的标识符用于标记面具长度

const INDICATOR_LEN: u64 = 4;

fn main() {
    let path: PathBuf = env::args().nth(1).unwrap().into();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();

    let mut buf = [0u8; INDICATOR_LEN as usize];
    file.seek(SeekFrom::End(-(INDICATOR_LEN as i64))).unwrap();
    file.read_exact(&mut buf).unwrap();
    let mask_len = u32::from_le_bytes(buf) as u64;
    let file_len = file.metadata().unwrap().len();
    let data_len = file_len - mask_len - INDICATOR_LEN;

    // 正常情况下，面具长度小于等于真实文件长度
    if mask_len <= data_len {
        let mut buffer = vec![0u8; mask_len as usize];
        file.seek(SeekFrom::Start(data_len)).unwrap();
        file.read_exact(&mut buffer).unwrap();
        buffer.reverse();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&buffer).unwrap();
    } else {
        let mut buffer = vec![0u8; data_len as usize];
        file.seek(SeekFrom::Start(mask_len)).unwrap();
        file.read_exact(&mut buffer).unwrap();
        buffer.reverse();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&buffer).unwrap();
    }
    file.set_len(data_len).unwrap();

    use notify_rust::{Notification, Timeout};
    let _ = Notification::new()
        .summary(&format!("😼 aletheia v{VERSION}"))
        .body("还原 apate 文件结束")
        .timeout(Timeout::Milliseconds(2000))
        .show();
}
