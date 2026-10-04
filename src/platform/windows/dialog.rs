//! 密码弹窗（设计 §7）：资源表里的 `DIALOGEX` + `DialogBoxParamW`。
//!
//! 用资源对话框而不是在代码里摆控件，是因为 Tab 导航、Enter 提交、Esc 取消、DPI 缩放
//! 全部由对话框管理器提供（D6）—— 因此不再需要自己处理键盘消息。

use std::error::Error;
use std::path::Path;

use ezz::{PasswordPrompt, PasswordResponse};
use log::warn;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{
    BST_CHECKED, EM_SETLIMITTEXT, EM_SETPASSWORDCHAR, IsDlgButtonChecked,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_SETCHECK, BN_CLICKED, DialogBoxParamW, EndDialog, GWLP_USERDATA, GetDlgItemTextW,
    GetWindowLongPtrW, ICON_BIG, ICON_SMALL, IDCANCEL, IDOK, LoadIconW, SendDlgItemMessageW,
    SendMessageW, SetDlgItemTextW, SetForegroundWindow, SetWindowLongPtrW, WM_CLOSE, WM_COMMAND,
    WM_INITDIALOG, WM_SETICON,
};
use windows::core::PCWSTR;

/// 密码长度上限（与原实现一致）。
const PASSWORD_LIMIT: usize = 1024;

/// 对话框按钮的命令 ID。取值沿用 winuser.h 的 `IDOK` / `IDCANCEL`（它们是
/// `MESSAGEBOX_RESULT` 类型，不能直接用在控件 ID 的位置上）。
const ID_EXTRACT: u32 = IDOK.0 as u32;
const ID_CANCEL: u32 = IDCANCEL.0 as u32;

/// 对话框上下文：由 `DialogBoxParamW` 的 `lParam` 传入，随后挂在窗口的 `DWLP_USER` 上。
struct DialogContext {
    information: String,
    response: Option<PasswordResponse>,
}

pub(super) struct WindowsPasswordPrompt;

impl PasswordPrompt for WindowsPasswordPrompt {
    fn request_password(
        &self,
        input: &Path,
        previous_attempt_failed: bool,
    ) -> Option<PasswordResponse> {
        match show(input, previous_attempt_failed) {
            Ok(response) => response,
            Err(error) => {
                warn!("could not show the password dialog: {error}");
                None
            }
        }
    }
}

fn show(
    input: &Path,
    previous_attempt_failed: bool,
) -> Result<Option<PasswordResponse>, Box<dyn Error>> {
    let mut context = DialogContext {
        information: information_text(input, previous_attempt_failed),
        response: None,
    };

    // SAFETY: 模板 ID 由资源脚本定义；`context` 在整个（同步的）调用期间存活，
    // 返回之后才读取它。
    let outcome = unsafe {
        DialogBoxParamW(
            Some(super::instance()),
            super::resource_id(super::DIALOG_ID),
            None,
            Some(dialog_proc),
            LPARAM((&raw mut context) as isize),
        )
    };
    // 对话框管理器用 -1 表示创建失败；其余值是 `EndDialog` 的返回值。
    if outcome == -1 {
        return Err("could not create the password dialog".into());
    }

    Ok(context.response)
}

fn information_text(input: &Path, previous_attempt_failed: bool) -> String {
    let filename = input
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| input.as_os_str().to_string_lossy());
    if previous_attempt_failed {
        format!("The password for {filename} was incorrect. Try again.")
    } else {
        format!("Enter the password for {filename}.")
    }
}

/// 对话框过程函数：返回 `1` 表示已处理，`0` 表示交给对话框管理器。
unsafe extern "system" fn dialog_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match message {
        WM_INITDIALOG => {
            // SAFETY: `lParam` 是 `show()` 里那个仍然存活的上下文。
            let raw = lparam.0 as *mut DialogContext;
            let context = unsafe { raw.as_mut() };
            let Some(context) = context else {
                return 0;
            };
            // SAFETY: 挂到窗口上供后续消息取回；窗口销毁时随窗口一起消失。
            unsafe {
                SetWindowLongPtrW(
                    window,
                    GWLP_USERDATA,
                    (context as *mut DialogContext) as isize,
                )
            };
            unsafe { initialize_controls(window, context) };
            // 返回 1：让对话框管理器按 Tab 顺序设置初始焦点（密码框是第一个可停留控件）。
            1
        }
        WM_COMMAND => {
            // 低 16 位是控件 ID，高 16 位是通知码（winuser.h）。
            let identifier = (wparam.0 & 0xffff) as u32;
            let notification = ((wparam.0 >> 16) & 0xffff) as u32;
            if notification != BN_CLICKED {
                return 0;
            }

            match identifier {
                ID_EXTRACT => {
                    if let Some(context) = unsafe { context(window) } {
                        context.response = Some(PasswordResponse {
                            password: password_text(window),
                            remember: is_checked(window, super::ID_REMEMBER),
                            keep_original: is_checked(window, super::ID_KEEP_ORIGINAL),
                        });
                    }
                    let _ = unsafe { EndDialog(window, ID_EXTRACT as isize) };
                    1
                }
                ID_CANCEL => {
                    let _ = unsafe { EndDialog(window, ID_CANCEL as isize) };
                    1
                }
                _ => 0,
            }
        }
        // 标题栏关闭按钮等同取消。
        WM_CLOSE => {
            let _ = unsafe { EndDialog(window, ID_CANCEL as isize) };
            1
        }
        _ => 0,
    }
}

/// `WM_INITDIALOG` 里的一次性设置：图标、文案、密码框上限与遮罩字符、勾选项默认值。
///
/// # Safety
/// 只能由持有有效窗口句柄的对话框过程调用。
unsafe fn initialize_controls(window: HWND, context: &DialogContext) {
    let text = super::wide(&context.information);
    // SAFETY: 控件 ID 来自资源脚本；`text` 以 NUL 结尾且在调用期间存活。
    let _ = unsafe { SetDlgItemTextW(window, super::ID_INFORMATION, PCWSTR(text.as_ptr())) };

    // SAFETY: 实例句柄是本进程，资源 ID 来自资源脚本。
    let icon = unsafe { LoadIconW(Some(super::instance()), super::resource_id(super::ICON_ID)) }
        .unwrap_or_default();
    unsafe {
        SendMessageW(
            window,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(icon.0 as isize)),
        );
        SendMessageW(
            window,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(icon.0 as isize)),
        );
        // 上限与原实现一致；遮罩字符用 `*`（默认是实心圆点）。
        let _ = SendDlgItemMessageW(
            window,
            super::ID_PASSWORD,
            EM_SETLIMITTEXT,
            WPARAM(PASSWORD_LIMIT),
            LPARAM(0),
        );
        let _ = SendDlgItemMessageW(
            window,
            super::ID_PASSWORD,
            EM_SETPASSWORDCHAR,
            WPARAM(b'*' as usize),
            LPARAM(0),
        );
        // 记住密码默认勾选，保留原归档默认不勾选（与原实现一致）。
        let _ = SendDlgItemMessageW(
            window,
            super::ID_REMEMBER,
            BM_SETCHECK,
            WPARAM(BST_CHECKED.0 as usize),
            LPARAM(0),
        );
        // 无主窗口的应用刚被调用起来：确保对话框出现在前台（与 macOS 侧的 activate 对应）。
        let _ = SetForegroundWindow(window);
    }
}

/// 取回挂在窗口上的上下文。
///
/// # Safety
/// 只能对正在运行的对话框调用，且返回的引用不得活过对话框。
unsafe fn context<'a>(window: HWND) -> Option<&'a mut DialogContext> {
    // SAFETY: `GWLP_USERDATA` 只在 `WM_INITDIALOG` 里写入一次，写的是仍然存活的上下文。
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut DialogContext;
    // SAFETY: 指针对应的上下文在对话框运行期间一直存活（且只在本线程使用）。
    unsafe { pointer.as_mut() }
}

fn is_checked(window: HWND, identifier: i32) -> bool {
    // SAFETY: 控件 ID 来自资源脚本，句柄是当前对话框。
    let state = unsafe { IsDlgButtonChecked(window, identifier) };
    state == BST_CHECKED.0
}

fn password_text(window: HWND) -> String {
    let mut buffer = [0_u16; PASSWORD_LIMIT + 1];
    // SAFETY: `buffer` 是可写切片，长度由切片本身给出；返回的是不含终止符的字符数。
    let length = unsafe { GetDlgItemTextW(window, super::ID_PASSWORD, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, PoisonError};
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

    /// 这三个用例都要驱动*同一个*标题的模态对话框，而 `FindWindowW` 会找到进程里的任意一个。
    /// 测试默认并行运行，所以必须自己串行化（否则会去操纵别的用例的窗口）。
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        // 一个用例失败会让锁中毒，但后续用例依然应该能跑。
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 文案随“上一次失败”变化（设计 §7）：不复现弹窗也能验证。
    #[test]
    fn information_text_reports_a_previous_failure() {
        let input = Path::new("C:/data/archive.7z");
        assert_eq!(
            information_text(input, false),
            "Enter the password for archive.7z."
        );
        assert_eq!(
            information_text(input, true),
            "The password for archive.7z was incorrect. Try again."
        );
    }

    /// 等对话框出现：`show` 在另一个线程里创建它，这里轮询窗口标题。
    fn wait_for_dialog() -> HWND {
        let title = super::super::wide("Password required");
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            // SAFETY: 标题是以 NUL 结尾的缓冲区。
            if let Ok(window) = unsafe { FindWindowW(None, PCWSTR(title.as_ptr())) }
                && !window.0.is_null()
            {
                return window;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the password dialog did not appear");
    }

    /// 在同一个进程里驱动对话框：写文本、拨勾选框（`None` 表示不碰）、按按钮。
    ///
    /// 这些用例验证的是“控件 ID 与资源模板接对了”：拿不到 GUI 时靠手动清单，
    /// 这是能自动化的那部分。运行方式：`cargo test --bin ezz -- --ignored`。
    fn drive_dialog(
        password: &str,
        button: u32,
        remember: Option<bool>,
        keep_original: Option<bool>,
    ) {
        let window = wait_for_dialog();
        let password = super::super::wide(password);
        let check = |value: bool| WPARAM(if value { BST_CHECKED.0 as usize } else { 0 });
        // SAFETY: 同一个进程内的对话框；传进去的缓冲区在调用期间存活。
        unsafe {
            let _ = SetDlgItemTextW(window, super::super::ID_PASSWORD, PCWSTR(password.as_ptr()));
            if let Some(value) = remember {
                let _ = SendDlgItemMessageW(
                    window,
                    super::super::ID_REMEMBER,
                    BM_SETCHECK,
                    check(value),
                    LPARAM(0),
                );
            }
            if let Some(value) = keep_original {
                let _ = SendDlgItemMessageW(
                    window,
                    super::super::ID_KEEP_ORIGINAL,
                    BM_SETCHECK,
                    check(value),
                    LPARAM(0),
                );
            }
            let _ = SendMessageW(
                window,
                WM_COMMAND,
                Some(WPARAM(button as usize)),
                Some(LPARAM(0)),
            );
        }
    }

    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn extract_returns_the_typed_password_and_the_checkbox_states() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        let driver =
            std::thread::spawn(|| drive_dialog("secret", ID_EXTRACT, Some(false), Some(true)));
        let response = show(Path::new("archive.7z"), true)
            .expect("show the dialog")
            .expect("the dialog must return a response");
        driver.join().expect("driver thread");

        assert_eq!(response.password, "secret");
        assert!(!response.remember);
        assert!(response.keep_original);
    }

    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn checkbox_defaults_are_remember_yes_and_keep_original_no() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        // 完全不碰勾选框，直接按“Extract”：验证 `WM_INITDIALOG` 里的默认值。
        let driver = std::thread::spawn(|| drive_dialog("secret", ID_EXTRACT, None, None));
        let response = show(Path::new("archive.7z"), false)
            .expect("show the dialog")
            .expect("the dialog must return a response");
        driver.join().expect("driver thread");

        assert_eq!(response.password, "secret");
        assert!(response.remember, "Remember this password defaults to on");
        assert!(!response.keep_original, "Keep the original defaults to off");
    }

    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn cancel_returns_no_response() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        let driver = std::thread::spawn(|| drive_dialog("", ID_CANCEL, None, None));
        let response = show(Path::new("archive.7z"), false).expect("show the dialog");
        driver.join().expect("driver thread");

        assert!(response.is_none(), "cancelling must not produce a password");
    }
}
