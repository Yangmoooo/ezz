mod common;
mod windows;

pub use windows::{run, show_fatal_error};

// 通知通道由平台模块提供：Windows 上是 WinRT toast。
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
