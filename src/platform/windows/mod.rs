//! Windows 平台实现（设计 §3.3、§12）。
//!
//! 这里只用一个原生绑定生态：`windows` crate。原先由 `native-windows-gui` 提供的东西
//! 由本模块自己完成：进程初始化、密码弹窗（资源对话框）、文件选择器、通知。

mod dialog;
mod lock;
mod notifications;
mod picker;

use std::error::Error;
use std::path::PathBuf;

use log::warn;
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize};
use windows::Win32::UI::Controls::{
    ICC_STANDARD_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::PCWSTR;

use super::RunOutcome;
use super::common::{PlatformPaths, initialize_logging, report_outcome, report_skipped};
use dialog::WindowsPasswordPrompt;
use lock::ExtractionLock;

pub(crate) use notifications::show_notification;

pub fn run() -> Result<RunOutcome, Box<dyn Error>> {
    initialize_process()?;

    let paths = PlatformPaths::discover()?;
    initialize_logging(&paths.log_file)?;

    // 同一次调用里的多个参数一律处理，不得拒绝（§3.3）。
    let inputs: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();

    // 加锁早于一切用户交互：锁的语义是"每用户会话一个 ezz"，从启动持有到退出。
    // 拿不到就立即拒绝并报告，不进入等待状态（§3.3）。
    let Some(_lock) = ExtractionLock::try_acquire() else {
        report_skipped(&inputs);
        // 被跳过不是失败（§3.3）。
        return Ok(RunOutcome::Succeeded);
    };

    // 引擎在启动时解析并校验一次（§11.1）：缺失就在这里报一次，不让每个输入各报一次。
    // 放在锁之后、选择器之前：被跳过的调用不必抱怨引擎，用户也不会先选完文件再被告知。
    let engine = ezz::locate_engine()?;

    // 没有输入才显示文件选择器；走到这里时锁已在手上。
    let inputs = if inputs.is_empty() {
        picker::select_files()?
    } else {
        inputs
    };
    if inputs.is_empty() {
        // 用户在选择器里取消：什么都没要求，不是失败。
        return Ok(RunOutcome::Succeeded);
    }

    let workflow = ezz::ExtractionWorkflow::with_password_support(
        engine,
        paths.password_database,
        WindowsPasswordPrompt,
    );

    let mut failed = false;
    for input in &inputs {
        let result = workflow.extract(input);
        if result.is_err() {
            failed = true;
        }
        report_outcome(input, &result);
    }

    Ok(if failed {
        RunOutcome::Failed
    } else {
        RunOutcome::Succeeded
    })
}

pub fn show_fatal_error(message: &str) {
    // 这个函数在启动失败之后被调用，所以只用不需要任何初始化就能工作的 API。
    let text = wide(message);
    let caption = wide("ezz could not start");
    // SAFETY: 两个缓冲区都以 NUL 结尾，并且在调用期间存活。
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// 原先由 `nwg::init()` 提供的初始化（设计 §12）。必须在做任何 COM/shell/WinRT 调用之前执行。
fn initialize_process() -> Result<(), Box<dyn Error>> {
    // COM 必须最先：`trash` 的 `IFileOperation` 与所有 shell API 都要求调用线程已初始化
    // COM。遗漏它的症状是第一次移入回收站时直接失败。
    // SAFETY: 在主线程上调用；`None` 表示不载入类型库，使用默认安全属性。
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;

    // 视觉样式来自清单里的 Common-Controls 6.0 依赖；这里只注册控件类。
    let common_controls = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_STANDARD_CLASSES | ICC_WIN95_CLASSES,
    };
    // SAFETY: `dwSize` 已按 API 要求填写为结构体大小。
    if !unsafe { InitCommonControlsEx(&common_controls) }.as_bool() {
        warn!("could not register common controls");
    }

    // WinRT 的 apartment 必须与 COM 一致（都是 STA）。失败只影响通知，不影响解压，
    // 所以只记录：便携版在未注册 AUMID 的机器上仍要能提取。
    // SAFETY: 与上面的 COM 初始化同为单线程 apartment。
    if let Err(error) = unsafe { RoInitialize(RO_INIT_SINGLETHREADED) } {
        warn!("could not initialize WinRT (notifications will be unavailable): {error}");
    }

    Ok(())
}

/// 本进程的模块句柄（exe 里的资源表就挂在它下面）。
fn instance() -> HINSTANCE {
    // SAFETY: `None` 表示查询当前进程自身，不会失败到影响调用方。
    unsafe { GetModuleHandleW(None) }
        .map(|module| HINSTANCE(module.0))
        .unwrap_or_default()
}

/// 以 NUL 结尾的 UTF-16 缓冲区（Win32 的 `PCWSTR` 要求）。
pub(super) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 资源表里的字符串 ID 转成 `PCWSTR`（`MAKEINTRESOURCEW` 的等价写法）。
pub(super) fn resource_id(identifier: u32) -> PCWSTR {
    PCWSTR(identifier as usize as *const u16)
}

/// 控件与对话框的 ID：必须与 `assets/ezz.rc` 里的一致。
pub(super) const DIALOG_ID: u32 = 101;
/// 提示控件：只有提示词，没有文件名（文件名在通知与日志里）。
pub(super) const ID_PROMPT: i32 = 1001;
pub(super) const ID_PASSWORD: i32 = 1002;
pub(super) const ID_REMEMBER: i32 = 1003;
pub(super) const ID_KEEP_ORIGINAL: i32 = 1004;
/// 显示/隐藏密码（设计 §7）。
pub(super) const ID_SHOW_PASSWORD: i32 = 1005;
pub(super) const ICON_ID: u32 = 1;
