use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Output;

use crate::workflow::ExtractionError;

pub(crate) struct SevenZip {
    executable: PathBuf,
}

impl SevenZip {
    pub(crate) fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    /// 唯一的引擎调用出口（R1/R6）：构造子进程、执行、把启动失败归一成 `EngineLaunch`。
    ///
    /// `crate::process::command` 是全程序唯一的子进程构造点（`CREATE_NO_WINDOW` 在它内部，
    /// §11.1），所以这里同时是“不弹控制台窗口”的唯一落点。调用方只负责拼参数。
    fn execute(
        &self,
        configure: impl FnOnce(&mut std::process::Command),
    ) -> Result<Output, ExtractionError> {
        let mut command = crate::process::command(&self.executable);
        configure(&mut command);
        command
            .output()
            .map_err(|error| ExtractionError::EngineLaunch {
                path: self.executable.clone(),
                message: error.to_string(),
            })
    }

    /// 前置扫描：一次 `l -slt -ba` 同时完成三件事 —— 校验条目路径（§5.4）、判定是否
    /// 需要密码、挑出用于最小化校验的条目（R4/D2）。
    ///
    /// `password` 为空时，“表头加密”是一个**成功**结果而不是错误：需要密码不等于打不开。
    /// 非空密码错误时照常返回 `WrongPassword`，供调用方区分候选密码。
    pub(crate) fn scan(
        &self,
        input: &Path,
        password: &str,
    ) -> Result<ArchiveScan, ExtractionError> {
        let output = self.execute(|command| {
            command
                .arg("l")
                .args(["-slt", "-ba"])
                .arg(password_switch(password))
                .args(["-bsp0", "-sccUTF-8", "-scsUTF-8"])
                .arg(input);
        })?;

        if output.status.success() {
            let listing = String::from_utf8_lossy(&output.stdout);
            return Ok(parse_listing(&listing));
        }

        let message = output_message(&output);
        if is_wrong_password(&message) {
            if password.is_empty() {
                return Ok(ArchiveScan {
                    header_encrypted: true,
                    encrypted: true,
                    sample_entry: None,
                    sanitized: Vec::new(),
                });
            }
            return Err(ExtractionError::WrongPassword);
        }
        if message.contains("Cannot open the file as archive") {
            return Err(ExtractionError::UnsupportedInput(input.to_path_buf()));
        }
        Err(ExtractionError::EngineFailed {
            operation: "list",
            exit_code: output.status.code(),
            message,
        })
    }

    pub(crate) fn embedded_archive(
        &self,
        input: &Path,
    ) -> Result<Option<PathBuf>, ExtractionError> {
        let output = self.execute(|command| {
            command
                .arg("l")
                .args([
                    "-t#",
                    "-slt",
                    "-ba",
                    "-p",
                    "-bsp0",
                    "-sccUTF-8",
                    "-scsUTF-8",
                ])
                .arg(input);
        })?;

        if !output.status.success() {
            let message = output_message(&output);
            if message.contains("Cannot open the file as archive") {
                return Ok(None);
            }
            return Err(ExtractionError::EngineFailed {
                operation: "scan embedded data in",
                exit_code: output.status.code(),
                message,
            });
        }

        Ok(find_embedded_archive(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    pub(crate) fn extract_embedded_archive(
        &self,
        input: &Path,
        output_dir: &Path,
        embedded: &Path,
    ) -> Result<PathBuf, ExtractionError> {
        let output = self.execute(|command| {
            command
                .arg("x")
                .arg("-t#")
                .arg(output_switch(output_dir))
                .args(["-y", "-aoa", "-bso0", "-bsp0", "-sccUTF-8", "-scsUTF-8"])
                .arg(input)
                .arg(embedded);
        })?;

        if !output.status.success() {
            return Err(ExtractionError::EngineFailed {
                operation: "extract embedded archive from",
                exit_code: output.status.code(),
                message: output_message(&output),
            });
        }

        Ok(output_dir.join(embedded))
    }

    /// 校验候选密码。`entry` 为 `Some` 时只测试该条目（R4：不跑整包 `t`）。
    pub(crate) fn test_password(
        &self,
        input: &Path,
        password: &str,
        entry: Option<&str>,
    ) -> Result<(), ExtractionError> {
        let output = self.execute(|command| {
            command
                .arg("t")
                .arg(password_switch(password))
                .args(["-bso0", "-bsp0", "-sccUTF-8", "-scsUTF-8"])
                .arg(input);
            if let Some(entry) = entry {
                command.arg(entry);
            }
        })?;

        if output.status.success() {
            Ok(())
        } else {
            let message = output_message(&output);
            if is_wrong_password(&message) {
                Err(ExtractionError::WrongPassword)
            } else {
                Err(ExtractionError::EngineFailed {
                    operation: "test",
                    exit_code: output.status.code(),
                    message,
                })
            }
        }
    }

    pub(crate) fn extract(
        &self,
        input: &Path,
        output_dir: &Path,
        password: &str,
    ) -> Result<ExtractionVerdict, ExtractionError> {
        let output = self.execute(|command| {
            command
                .arg("x")
                .arg(output_switch(output_dir))
                .arg(password_switch(password))
                .args([
                    "-y",
                    "-aoa",
                    "-spe",
                    "-bso0",
                    "-bsp0",
                    "-sccUTF-8",
                    "-scsUTF-8",
                ])
                .arg(input);
        })?;

        let code = output.status.code();
        let message = output_message(&output);
        if output.status.success() {
            return Ok(ExtractionVerdict::default());
        }
        if is_wrong_password(&message) {
            // 混合加密归档：校验阶段可能通过而提取阶段仍报密码错（D2/R4）。
            return Err(ExtractionError::WrongPassword);
        }

        match classify(code, &message) {
            Classified::Accepted {
                engine_warning,
                sanitized_links,
                failed_entries,
            } => Ok(ExtractionVerdict {
                engine_warning,
                sanitized_links,
                failed_entries,
            }),
            Classified::Fatal => Err(ExtractionError::EngineFailed {
                operation: "extract",
                exit_code: code,
                message,
            }),
        }
    }
}

/// 前置扫描结果。
#[derive(Debug, Default)]
pub(crate) struct ArchiveScan {
    /// 不带密码连列表都做不了：文件名（表头）也加密。
    pub(crate) header_encrypted: bool,
    /// 至少一个条目带 `Encrypted = +`。表头加密时也为 `true`。
    pub(crate) encrypted: bool,
    /// 用于最小化校验的条目：优先取声明加密的、最小的非空文件条目。
    pub(crate) sample_entry: Option<String>,
    /// 路径需要消毒的条目（`..`、绝对路径、盘符前缀）：7-Zip 会把它们重写进工作目录，
    /// 数据保留，但必须报告（设计 §5.4）。
    pub(crate) sanitized: Vec<String>,
}

/// 一次提取的结局：两级结果模型里"降级成功"那一级的细节（设计 §5.1）。
#[derive(Debug, Default)]
pub(crate) struct ExtractionVerdict {
    /// 退出码 1：引擎报了警告，但结果有效。内容是引擎消息。
    pub(crate) engine_warning: Option<String>,
    /// 被引擎消毒的链接条目：7-Zip 拒绝了危险链接目标，把该条目降级成普通文件（实测行为）。
    pub(crate) sanitized_links: Vec<String>,
    /// 引擎报告数据损坏（`CRC Failed` / `Data Error`）的条目。
    ///
    /// 实测：7-Zip 仍会把（损坏的）内容写进输出。层 1 透传（§5.5）：ezz 不修改引擎写出的
    /// 内容，只把这份清单交给调用方报告。
    pub(crate) failed_entries: Vec<String>,
}

enum Classified {
    /// 结果有效：退出码 0，或退出码 1（Warning），或退出码 2 且所有错误行都是可容忍的逐条目失败。
    Accepted {
        engine_warning: Option<String>,
        sanitized_links: Vec<String>,
        failed_entries: Vec<String>,
    },
    /// 其余非零退出码：致命失败，必须回滚。
    Fatal,
}

/// 把一次提取的退出码与引擎消息分成"降级成功"或"致命失败"（纯函数，便于测试）。
fn classify(exit_code: Option<i32>, message: &str) -> Classified {
    match exit_code {
        // 退出码 1（Warning）：结果已提交，但引擎报了警告。
        Some(1) => Classified::Accepted {
            engine_warning: Some(message.to_owned()),
            sanitized_links: Vec::new(),
            failed_entries: Vec::new(),
        },
        Some(2) => match tolerated_exit_two(message) {
            Some((sanitized_links, failed_entries)) => Classified::Accepted {
                engine_warning: None,
                sanitized_links,
                failed_entries,
            },
            None => Classified::Fatal,
        },
        _ => Classified::Fatal,
    }
}

/// 退出码 2 的降级条件（D1/D2）：**所有** `ERROR:` 行都必须是可容忍的逐条目失败。
///
/// 可容忍的两类（都是实测出来的）：
/// - `ERROR: Dangerous link path was ignored : <条目> : <目标>` —— 链接被降级成普通文件；
/// - `ERROR: CRC Failed : <条目>` / `ERROR: Data Error : <条目>` —— 单个条目数据损坏。
///
/// 出现任何其它错误行（包括 `ERROR: Data Error in encrypted file. Wrong password?`，
/// 它的冒号不在前缀之后）都不降级。
fn tolerated_exit_two(message: &str) -> Option<(Vec<String>, Vec<String>)> {
    let mut sanitized = Vec::new();
    let mut failed = Vec::new();
    let mut saw_error = false;

    for line in message.lines() {
        let line = line.trim();
        if !line.starts_with("ERROR:") {
            continue;
        }
        saw_error = true;

        // 注意：`entry_after` 返回 `None` 只表示“不是这个前缀”，所以这里不能用 `?`。
        if let Some(entry) = entry_after(line, "ERROR: Dangerous link path was ignored") {
            sanitized.push(entry);
            continue;
        }
        if let Some(entry) = entry_after(line, "ERROR: CRC Failed") {
            failed.push(entry);
            continue;
        }
        if let Some(entry) = entry_after(line, "ERROR: Data Error") {
            failed.push(entry);
            continue;
        }
        return None;
    }

    saw_error.then_some((sanitized, failed))
}

/// 取出 `ERROR: <前缀> : <条目>[ : <额外>]` 里的条目名。
///
/// 前缀之后必须紧跟冒号，这样 `ERROR: Data Error` 不会误吞
/// `ERROR: Data Error in encrypted file. Wrong password? : x`。
fn entry_after(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?.trim();
    let rest = rest.strip_prefix(':')?.trim();
    let entry = rest.split(" : ").next().unwrap_or(rest).trim();
    (!entry.is_empty()).then(|| entry.to_owned())
}

/// 解析 `l -slt -ba` 输出（条目之间以空行分隔）。
///
/// 这里的判据来自实测，不是文档：目录靠 `Attributes` 的首字符识别（Windows 风格的 `D`
/// 与 Unix 宿主的 `drwx…`，所以两种都要认；26.02 的输出里根本不出现 `Folder = +`）；
/// `Encrypted = -` 也会出现，所以必须精确比较 `+`；0 字节条目 **不能**当校验样本 ——
/// 实测用错密码测空文件也会报 `Everything is Ok`。
fn parse_listing(listing: &str) -> ArchiveScan {
    let mut scan = ArchiveScan::default();
    let mut best_encrypted: Option<(u64, String)> = None;
    let mut best_plain: Option<(u64, String)> = None;

    let mut path: Option<String> = None;
    let mut size = 0_u64;
    let mut is_folder = false;
    let mut is_encrypted = false;

    for line in listing.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(path) = path.take() {
                if is_encrypted {
                    scan.encrypted = true;
                }
                if !is_folder && size > 0 {
                    let slot = if is_encrypted {
                        &mut best_encrypted
                    } else {
                        &mut best_plain
                    };
                    if slot.as_ref().is_none_or(|known| size < known.0) {
                        *slot = Some((size, path));
                    }
                }
            }
            size = 0;
            is_folder = false;
            is_encrypted = false;
            continue;
        }

        if let Some(value) = line.strip_prefix("Path = ") {
            is_folder |= value.ends_with(['/', '\\']);
            if is_unsafe_archive_path(value) {
                scan.sanitized.push(value.to_owned());
            }
            path = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("Size = ") {
            size = value.parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("Attributes = ") {
            let value = value.trim_start();
            is_folder |= value.starts_with(['D', 'd']);
        } else if line.strip_prefix("Folder = ") == Some("+") {
            is_folder = true;
        } else if line.strip_prefix("Encrypted = ") == Some("+") {
            is_encrypted = true;
        }
    }

    scan.sample_entry = best_encrypted.or(best_plain).map(|(_, path)| path);
    scan
}

fn find_embedded_archive(output: &str) -> Option<PathBuf> {
    let mut path: Option<PathBuf> = None;
    let mut archive_type = None;
    let mut offset = None;

    for line in output.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if offset.is_some_and(|offset| offset > 0)
                && archive_type.is_some_and(is_supported_embedded_type)
                && let Some(path) = path.take()
                && is_safe_relative_path(&path)
            {
                return Some(path);
            }
            path = None;
            archive_type = None;
            offset = None;
            continue;
        }

        if let Some(value) = line.strip_prefix("Path = ") {
            path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("Type = ") {
            archive_type = Some(value);
        } else if let Some(value) = line.strip_prefix("Offset = ") {
            offset = value.parse::<u64>().ok();
        }
    }

    None
}

fn is_supported_embedded_type(archive_type: &str) -> bool {
    matches!(
        archive_type.to_ascii_lowercase().as_str(),
        "zip" | "7z" | "rar" | "rar5"
    )
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn is_unsafe_archive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.is_empty()
        || path.starts_with(['/', '\\'])
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || path.split(['/', '\\']).any(|component| component == "..")
}

fn password_switch(password: &str) -> OsString {
    let mut switch = OsString::from("-p");
    switch.push(password);
    switch
}

/// `-o<目录>`：解压目标目录。
fn output_switch(directory: &Path) -> OsString {
    let mut switch = OsString::from("-o");
    switch.push(directory);
    switch
}

fn is_wrong_password(message: &str) -> bool {
    message.contains("Wrong password?") || message.contains("Wrong password")
}

fn output_message(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_one_is_accepted_with_the_engine_message() {
        match classify(Some(1), "something to report") {
            Classified::Accepted {
                engine_warning,
                sanitized_links,
                failed_entries,
            } => {
                assert_eq!(engine_warning.as_deref(), Some("something to report"));
                assert!(sanitized_links.is_empty());
                assert!(failed_entries.is_empty());
            }
            Classified::Fatal => panic!("exit code 1 is a degraded success"),
        }
    }

    #[test]
    fn exit_code_two_with_only_ignored_links_is_accepted_and_named() {
        let message = "ERROR: Dangerous link path was ignored : escape-link : ..\\outside.txt";
        match classify(Some(2), message) {
            Classified::Accepted {
                engine_warning,
                sanitized_links,
                failed_entries,
            } => {
                assert!(engine_warning.is_none());
                assert!(failed_entries.is_empty());
                assert_eq!(sanitized_links, vec!["escape-link".to_owned()]);
            }
            Classified::Fatal => panic!("ignored links are a degraded success"),
        }
    }

    #[test]
    fn exit_code_two_with_corrupted_entries_is_accepted_and_named() {
        let message = "ERROR: CRC Failed : bad.txt\nERROR: Data Error : nested/bad2.txt\n";
        match classify(Some(2), message) {
            Classified::Accepted {
                engine_warning,
                sanitized_links,
                failed_entries,
            } => {
                assert!(engine_warning.is_none());
                assert!(sanitized_links.is_empty());
                assert_eq!(
                    failed_entries,
                    vec!["bad.txt".to_owned(), "nested/bad2.txt".to_owned()]
                );
            }
            Classified::Fatal => panic!("per-entry data failures are a degraded success"),
        }
    }

    #[test]
    fn exit_code_two_with_an_unexpected_error_is_fatal() {
        assert!(matches!(
            classify(Some(2), "ERROR: Something else"),
            Classified::Fatal
        ));
        assert!(matches!(
            classify(Some(2), "ERROR: Headers Error : corrupt.zip"),
            Classified::Fatal
        ));
        // 混合：一条可容忍 + 一条不可容忍 → 致命。
        assert!(matches!(
            classify(Some(2), "ERROR: CRC Failed : a\nERROR: Headers Error : b"),
            Classified::Fatal
        ));
        // 没有 ERROR 行 / 空消息 → 不降级。
        assert!(matches!(classify(Some(2), ""), Classified::Fatal));
        // 密码错误绝不走白名单：它的冒号不在前缀之后。
        assert!(matches!(
            classify(
                Some(2),
                "ERROR: Data Error in encrypted file. Wrong password? : a"
            ),
            Classified::Fatal
        ));
    }

    #[test]
    fn other_exit_codes_are_fatal() {
        assert!(matches!(classify(Some(7), "whatever"), Classified::Fatal));
        assert!(matches!(classify(None, "whatever"), Classified::Fatal));
    }

    #[test]
    fn listed_paths_that_need_sanitizing_are_reported() {
        let listing = "Path = ..\\evil.txt\nSize = 9\nAttributes = A\n\nPath = fine.txt\nSize = 3\nAttributes = A\n\n";
        let scan = parse_listing(listing);

        assert_eq!(scan.sanitized, vec!["..\\evil.txt".to_owned()]);
        assert!(!scan.encrypted);
        assert!(!scan.header_encrypted);
        assert_eq!(scan.sample_entry.as_deref(), Some("fine.txt"));
    }

    #[test]
    fn directories_are_detected_from_both_attribute_styles() {
        // Windows 宿主的 `D…` 与 Unix 宿主的 `drwx…` 都必须认出来，否则目录会被当样本。
        let listing = "Path = win-dir\nSize = 0\nAttributes = D\n\nPath = unix-dir\nSize = 0\nAttributes = drwxr-xr-x\n\nPath = payload.txt\nSize = 4\nAttributes = -rw-r--r--\n\n";
        let scan = parse_listing(listing);
        assert_eq!(scan.sample_entry.as_deref(), Some("payload.txt"));
    }

    #[test]
    fn embedded_archives_are_only_found_for_supported_types_with_an_offset() {
        let supported = "Path = payload.zip\nType = zip\nOffset = 42\n\n";
        assert_eq!(
            find_embedded_archive(supported),
            Some(PathBuf::from("payload.zip"))
        );

        // 偏移为 0 意味着条目就是整个文件，不算“内嵌”。
        assert_eq!(
            find_embedded_archive("Path = payload.zip\nType = zip\nOffset = 0\n\n"),
            None
        );
        // 不支持的内部类型。
        assert_eq!(
            find_embedded_archive("Path = payload.bin\nType = bin\nOffset = 42\n\n"),
            None
        );
        // 逃逸路径不得被认成可释放的内嵌归档。
        assert_eq!(
            find_embedded_archive("Path = ../payload.zip\nType = zip\nOffset = 42\n\n"),
            None
        );
        // 只有部分字段时不得误判。
        assert_eq!(
            find_embedded_archive("Path = payload.zip\nType = zip\n\n"),
            None
        );
    }

    #[test]
    fn every_ignored_link_is_named() {
        let message = "ERROR: Dangerous link path was ignored : first : ../a\nERROR: Dangerous link path was ignored : dir/second : C:\\b\n";
        let (sanitized, failed) = tolerated_exit_two(message).expect("tolerated exit code 2");
        assert_eq!(sanitized, vec!["first".to_owned(), "dir/second".to_owned()]);
        assert!(failed.is_empty());
    }
}
