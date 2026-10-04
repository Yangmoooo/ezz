mod common;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{run, show_fatal_error};
#[cfg(target_os = "windows")]
pub use windows::{run, show_fatal_error};

// 平台各自负责通知通道：Windows 是 WinRT toast，macOS 是 UNUserNotificationCenter。
#[cfg(target_os = "macos")]
pub(crate) use macos::show_notification;
#[cfg(target_os = "windows")]
pub(crate) use windows::show_notification;

/// 一次调用的整体结果，决定进程退出码。
///
/// - `Succeeded` → `0`：全部输入都成功；被跳过（另一个实例在运行）与用户取消文件选择器也算成功。
/// - `Failed` → `1`：至少一个输入失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Succeeded,
    Failed,
}
