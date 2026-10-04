//! 子进程构造出口。
//!
//! 所有 ezz 启动的外部程序（`7zz`、`icacls`）都必须经过这里。GUI 进程不得产生任何
//! 控制台窗口（设计 §11.1），而 Windows 上子进程默认会为自己新建一个控制台。

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
