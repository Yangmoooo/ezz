//! 文件选择器：`IFileOpenDialog`。COM 已在启动时初始化。

use std::error::Error;
use std::path::PathBuf;

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
    FileOpenDialog, IFileOpenDialog, IShellItemArray, SIGDN_FILESYSPATH,
};
use windows::core::w;

/// 让用户选择要解压的文件。用户取消返回空列表（不是错误）。
pub(super) fn select_files() -> Result<Vec<PathBuf>, Box<dyn Error>> {
    // SAFETY: COM 已在 `initialize_process` 里以 STA 初始化，本函数只在主线程调用。
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;

        // 多选、只接受真实存在的文件系统路径。
        let options = dialog.GetOptions()?
            | FOS_ALLOWMULTISELECT
            | FOS_FILEMUSTEXIST
            | FOS_FORCEFILESYSTEM
            | FOS_PATHMUSTEXIST;
        dialog.SetOptions(options)?;
        dialog.SetTitle(w!("Select files to extract"))?;

        // 取消时 `Show` 返回 HRESULT_FROM_WIN32(ERROR_CANCELLED)：用户什么都没要求。
        if dialog.Show(None).is_err() {
            return Ok(Vec::new());
        }

        let results: IShellItemArray = dialog.GetResults()?;
        let mut paths = Vec::new();
        for index in 0..results.GetCount()? {
            let item = results.GetItemAt(index)?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            paths.push(PathBuf::from(name.to_string()?));
            // SAFETY: `GetDisplayName` 返回由 COM 分配、所有权交给调用方的字符串。
            CoTaskMemFree(Some(name.0.cast()));
        }

        Ok(paths)
    }
}
