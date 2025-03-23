use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};

use super::archive::Archive;
use super::sevenzz::Sevenzz;
use crate::types::EzzResult;

// 预处理 Steganographier 或 apate 的默认隐藏格式
impl Archive {
    pub fn reveal(&self, zz: &Sevenzz) -> EzzResult<Self> {
        const APATE_DATA_LEN: u32 = 0x11F0BD;
        const APATE_FEATURE: [u8; 16] = [
            0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, 0x6D, 0x70, 0x34, 0x32, 0x00, 0x00,
            0x00, 0x00,
        ];

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.get_path())?;

        let mut buf1 = [0u8; 16];
        file.read_exact(&mut buf1)?;
        let mut buf2 = [0u8; 4];
        file.seek(SeekFrom::End(-4))?;
        file.read_exact(&mut buf2)?;

        if buf1 == APATE_FEATURE && u32::from_le_bytes(buf2) == APATE_DATA_LEN {
            // 使用 apate 一键伪装的隐藏文件
            let mut buf = vec![0u8; APATE_DATA_LEN as usize];
            file.seek(SeekFrom::End(-(APATE_DATA_LEN as i64 + 4)))?;
            file.read_exact(&mut buf)?;
            buf.reverse();
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&buf)?;

            let file_size = file.metadata()?.len();
            file.set_len(file_size - APATE_DATA_LEN as u64 - 4)?;
            Ok(self.clone())
        } else {
            // 使用 Steganographier 的隐藏文件或未隐藏文件
            drop(file);
            zz.command_x_steganor(self)?;
            self.remove()?;
            Ok(self.with_name("2.zip"))
        }
    }
}
