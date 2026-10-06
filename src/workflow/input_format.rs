//! 输入格式探测：普通归档与 Steganographier。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::ExtractionError;
use crate::seven_zip::{ArchiveScan, SevenZip};

/// 探测输入格式，并准备好实际要解压的输入。
///
/// 探测阶段已经为普通归档做过一次无密码扫描，直接复用它，调用方不必再扫。
///
/// Steganographier 的内嵌归档必须先落到磁盘（第二次解压是另一次引擎调用，需要一个真实路径），
/// 所以这里返回一个临时目录：它必须在视频所在卷上（载荷可能有上 GB），并活到解压结束。
pub(super) fn detect_input_format(
    seven_zip: &SevenZip,
    input: &Path,
    parent: &Path,
) -> Result<(PathBuf, ArchiveScan, Option<tempfile::TempDir>), ExtractionError> {
    if let Some(embedded) = detect_steganographier(seven_zip, input)? {
        let scratch = tempfile::Builder::new()
            .prefix(".ezz-tmp-")
            .tempdir_in(parent)
            .map_err(|error| {
                super::file_system_error("create scratch directory for", input, error)
            })?;
        let archive = seven_zip.extract_embedded_archive(input, scratch.path(), &embedded)?;
        if !archive.is_file() {
            return Err(ExtractionError::UnsupportedInput(input.to_path_buf()));
        }
        return match seven_zip.scan(&archive, "") {
            Ok(scan) => Ok((archive, scan, Some(scratch))),
            Err(_) => Err(ExtractionError::UnsupportedInput(input.to_path_buf())),
        };
    }

    match seven_zip.scan(input, "") {
        Ok(scan) => Ok((input.to_path_buf(), scan, None)),
        Err(ExtractionError::UnsupportedInput(_)) => {
            Err(ExtractionError::UnsupportedInput(input.to_path_buf()))
        }
        Err(error) => Err(error),
    }
}

/// 视频文件里内嵌的归档（`-t#`）。不是视频、或找不到受支持的内嵌归档时返回 `None`。
fn detect_steganographier(
    seven_zip: &SevenZip,
    input: &Path,
) -> Result<Option<PathBuf>, ExtractionError> {
    let is_video = input
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("mp4") || extension.eq_ignore_ascii_case("mkv")
        });
    if !is_video {
        return Ok(None);
    }

    seven_zip.embedded_archive(input)
}
