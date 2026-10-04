//! 输入格式探测：普通归档与 Steganographier（设计 §6.1、§6.2）。

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use super::safety::discard_unsafe_entries;
use super::{ExtractionError, file_system_error};
use crate::seven_zip::{ArchiveScan, SevenZip};

pub(super) enum DetectedInputFormat {
    /// 常规归档：`scan` 是探测阶段已经做过的**无密码**扫描，直接复用（R4）。
    RegularArchive {
        scan: ArchiveScan,
    },
    Steganographier {
        embedded: PathBuf,
    },
}

impl DetectedInputFormat {
    /// 准备实际要解压的输入，并带回一个已经做过的扫描（没有则为 `None`）。
    pub(super) fn prepare(
        self,
        seven_zip: &SevenZip,
        input: &Path,
        prepared: &Path,
    ) -> Result<(PathBuf, Option<ArchiveScan>), ExtractionError> {
        match self {
            Self::RegularArchive { scan } => Ok((input.to_path_buf(), Some(scan))),
            Self::Steganographier { embedded } => {
                fs::create_dir(prepared).map_err(|error| {
                    file_system_error("create special-format workspace", prepared, error)
                })?;
                let archive = seven_zip.extract_embedded_archive(input, prepared, &embedded)?;
                // 内嵌归档是 `-t#` 直接拷贝出来的单个文件，理论上不会出现链接或特殊文件；
                // 仍然过一遍丢弃检查（结果忽略：这一步的失败由后面的 `is_file` 捕获）。
                let _ = discard_unsafe_entries(prepared)?;
                if !archive.is_file() {
                    return Err(ExtractionError::UnsupportedInput(input.to_path_buf()));
                }
                match seven_zip.scan(&archive, "") {
                    Ok(scan) => Ok((archive, Some(scan))),
                    Err(_) => Err(ExtractionError::UnsupportedInput(input.to_path_buf())),
                }
            }
        }
    }
}

/// 探测输入格式：先试 Steganographier（`-t#`），再按普通归档扫描（设计 §6.2）。
///
/// 两个探测函数就是全部格式集合：`SevenZip` 只有一个实现，不为假设中的第三种格式保留一个
/// trait（设计 §9：不得为假设中的扩展扩大接口）。
pub(super) fn detect_input_format(
    seven_zip: &SevenZip,
    input: &Path,
) -> Result<DetectedInputFormat, ExtractionError> {
    if let Some(format) = detect_steganographier(seven_zip, input)? {
        return Ok(format);
    }
    if let Some(format) = detect_regular_archive(seven_zip, input)? {
        return Ok(format);
    }

    Err(ExtractionError::UnsupportedInput(input.to_path_buf()))
}

/// 视频文件里内嵌的归档（`-t#`）。不是视频、或找不到受支持的内嵌归档时返回 `None`。
fn detect_steganographier(
    seven_zip: &SevenZip,
    input: &Path,
) -> Result<Option<DetectedInputFormat>, ExtractionError> {
    let is_video = input
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("mp4") || extension.eq_ignore_ascii_case("mkv")
        });
    if !is_video {
        return Ok(None);
    }

    seven_zip
        .embedded_archive(input)
        .map(|embedded| embedded.map(|embedded| DetectedInputFormat::Steganographier { embedded }))
}

/// 普通归档：一次无密码扫描同时决定“这是不是归档”和“要不要密码”（R4）。
fn detect_regular_archive(
    seven_zip: &SevenZip,
    input: &Path,
) -> Result<Option<DetectedInputFormat>, ExtractionError> {
    match seven_zip.scan(input, "") {
        Ok(scan) => Ok(Some(DetectedInputFormat::RegularArchive { scan })),
        Err(ExtractionError::UnsupportedInput(_)) => Ok(None),
        Err(error) => Err(error),
    }
}
