//! 资源管理器刷新：移入回收站不触发 shell 变更通知，删除成功后必须显式通知一次。

use std::path::PathBuf;

/// 通知 shell：这些条目所在的目录内容已变化。
///
/// 传入的是**被删除的条目**，不是目录；函数内部去重后只通知父目录。
pub(crate) fn refresh_parents(paths: &[PathBuf]) {
    let mut directories: Vec<PathBuf> = Vec::new();
    for path in paths {
        let Some(parent) = path.parent() else {
            continue;
        };
        if !directories.iter().any(|known| known == parent) {
            directories.push(parent.to_path_buf());
        }
    }

    refresh_directories(&directories);
}

#[cfg(windows)]
fn refresh_directories(directories: &[PathBuf]) {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::UI::Shell::{SHCNE_UPDATEDIR, SHCNF_FLUSH, SHCNF_PATHW, SHChangeNotify};

    for directory in directories {
        let mut wide: Vec<u16> = directory.as_os_str().encode_wide().collect();
        wide.push(0);

        // 安全：`wide` 是以 NUL 结尾的 UTF-16 缓冲区，在调用期间保持存活；
        // `SHCNF_PATHW` 要求 `dwItem1` 指向宽字符路径。`SHCNF_FLUSH` 让通知在
        // 进程退出前被处理完（本程序处理完就退出）。
        unsafe {
            SHChangeNotify(
                SHCNE_UPDATEDIR,
                SHCNF_PATHW | SHCNF_FLUSH,
                Some(wide.as_ptr() as *const core::ffi::c_void),
                None,
            );
        }
    }
}

#[cfg(not(windows))]
fn refresh_directories(_directories: &[PathBuf]) {}
