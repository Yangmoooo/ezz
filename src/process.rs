//! 子进程构造出口：所有外部程序都必须经过这里，且不得分配控制台窗口。

use std::ffi::OsStr;
use std::process::Command;

/// `CREATE_NO_WINDOW`：子进程不分配控制台。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 创建一个不会弹出控制台窗口的子进程。
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    #[cfg(windows)]
    let mut command = Command::new(program);
    #[cfg(not(windows))]
    let command = Command::new(program);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}
