use windows::Win32::UI::Shell::{SHCNE_UPDATEDIR, SHCNF_PATH, SHChangeNotify};
use windows::core::HSTRING;

// 手动通知 Windows Explorer 刷新指定目录
// 这是因为 Trash 库的回收站删除不会像文件系统删除一样自动触发 SHChangeNotify
pub fn refresh_dir(path: &str) {
    let hstring_path = HSTRING::from(path);
    unsafe {
        SHChangeNotify(
            SHCNE_UPDATEDIR,
            SHCNF_PATH,
            Some(hstring_path.as_ptr() as *const _),
            Some(std::ptr::null_mut()),
        );
    }
}
