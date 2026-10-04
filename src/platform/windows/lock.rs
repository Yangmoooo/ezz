//! 每用户会话的命名互斥体：进程启动即持有到退出，同一会话里只能有一个实例在提取。

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};

use super::wide;

/// `Local\` 命名空间与密码库的每用户作用域对齐；名字里不带版本号，跳版本也互斥。
const EXTRACT_MUTEX: &str = "Local\\io.github.yangmoooo.ezz.extract";

pub(super) struct ExtractionLock {
    /// 句柄故意不在进程内关闭：关闭即等于释放。
    _handle: HANDLE,
}

impl ExtractionLock {
    /// 非阻塞尝试获取。返回 `None` 表示另一个 ezz 正在提取，本次调用必须立即拒绝。
    pub(super) fn try_acquire() -> Option<Self> {
        let name = wide(EXTRACT_MUTEX);
        // SAFETY: `name` 是以 NUL 结尾的 UTF-16 缓冲区，在调用期间保持存活；`None` 表示
        // 使用默认安全属性（不需要跨用户共享）。
        let handle =
            unsafe { CreateMutexW(None, false, windows::core::PCWSTR(name.as_ptr())) }.ok()?;

        // SAFETY: `handle` 刚刚由 `CreateMutexW` 返回且有效；等待 0 毫秒即非阻塞尝试。
        let status = unsafe { WaitForSingleObject(handle, 0) };
        // WAIT_OBJECT_0：互斥体现在归本次调用所有。
        // WAIT_ABANDONED：上一个持有者崩溃或被强制结束，互斥体已释放，同样归我们所有。
        // 其余（WAIT_TIMEOUT）：别人正持有 —— 立即放弃，不进入任何等待状态。
        if status == WAIT_OBJECT_0 || status == WAIT_ABANDONED {
            return Some(Self { _handle: handle });
        }

        // SAFETY: 拒绝路径上我们从未获得所有权，这个句柄必须立刻关闭，否则会泄漏。
        unsafe {
            let _ = CloseHandle(handle);
        }
        None
    }
}
