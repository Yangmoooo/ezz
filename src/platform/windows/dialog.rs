//! 密码弹窗：资源表里的 `DIALOGEX` + `DialogBoxParamW`。
//!
//! Tab 导航、Enter 提交、Esc 取消与 DPI 缩放都由对话框管理器提供，代码里不需要处理键盘消息。
//!
//! 内容是一行提示词 + 密码框 + 三个勾选项，不含文件名（它在通知与日志里）。

use std::error::Error;

use ezz::{PasswordPrompt, PasswordResponse};
use log::warn;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, EM_SETLIMITTEXT, EM_SETPASSWORDCHAR, IsDlgButtonChecked,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_SETCHECK, BN_CLICKED, DialogBoxParamW, EndDialog, GWLP_USERDATA, GetDlgItem,
    GetDlgItemTextW, GetWindowLongPtrW, ICON_BIG, ICON_SMALL, IDCANCEL, IDOK, LoadIconW,
    SendDlgItemMessageW, SendMessageW, SetDlgItemTextW, SetForegroundWindow, SetWindowLongPtrW,
    WM_CLOSE, WM_COMMAND, WM_INITDIALOG, WM_SETICON,
};
use windows::core::PCWSTR;

/// 密码长度上限。
const PASSWORD_LIMIT: usize = 1024;

/// 密码框的遮罩字符（`ES_PASSWORD` 的默认是实心圆点）。
const PASSWORD_MASK: u16 = b'*' as u16;

/// 对话框按钮的命令 ID：`IDOK` / `IDCANCEL` 是 `MESSAGEBOX_RESULT`，用它的 `.0`。
const ID_EXTRACT: u32 = IDOK.0 as u32;
const ID_CANCEL: u32 = IDCANCEL.0 as u32;

/// `Show password` 在 `WM_COMMAND` 里以 `u32` 比较（低 16 位是控件 ID）。
const SHOW_PASSWORD_ID: u32 = super::ID_SHOW_PASSWORD as u32;

/// 对话框上下文：由 `DialogBoxParamW` 的 `lParam` 传入，随后挂在窗口的 `DWLP_USER` 上。
struct DialogContext {
    /// 提示词（`Enter the password:` 或重试时的另一种）。不含文件名。
    message: &'static str,
    response: Option<PasswordResponse>,
}

pub(super) struct WindowsPasswordPrompt;

impl PasswordPrompt for WindowsPasswordPrompt {
    fn request_password(&self, previous_attempt_failed: bool) -> Option<PasswordResponse> {
        match show(previous_attempt_failed) {
            Ok(response) => response,
            Err(error) => {
                warn!("could not show the password dialog: {error}");
                None
            }
        }
    }
}

fn show(previous_attempt_failed: bool) -> Result<Option<PasswordResponse>, Box<dyn Error>> {
    let mut context = DialogContext {
        message: super::super::common::password_prompt_message(previous_attempt_failed),
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
                SHOW_PASSWORD_ID => {
                    // SAFETY: 控件 ID 来自资源脚本，句柄是当前对话框。
                    unsafe { toggle_password_visibility(window) };
                    1
                }
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

/// `WM_INITDIALOG` 里的一次性设置：提示词、图标、密码框上限与遮罩字符、勾选项默认值。
///
/// # Safety
/// 只能由持有有效窗口句柄的对话框过程调用。
unsafe fn initialize_controls(window: HWND, context: &DialogContext) {
    let message = super::wide(context.message);
    // SAFETY: 控件 ID 来自资源脚本；缓冲区以 NUL 结尾且在调用期间存活。
    unsafe {
        let _ = SetDlgItemTextW(window, super::ID_PROMPT, PCWSTR(message.as_ptr()));
    }

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
        // 密码框：长度上限 + 遮罩字符（默认隐藏）。
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
            WPARAM(PASSWORD_MASK as usize),
            LPARAM(0),
        );
        // 勾选项默认值（与 v2 一致）：只勾“记住密码”。
        for (identifier, checked) in [
            (super::ID_SHOW_PASSWORD, false),
            (super::ID_REMEMBER, true),
            (super::ID_KEEP_ORIGINAL, false),
        ] {
            let state = if checked { BST_CHECKED } else { BST_UNCHECKED };
            let _ = SendDlgItemMessageW(
                window,
                identifier,
                BM_SETCHECK,
                WPARAM(state.0 as usize),
                LPARAM(0),
            );
        }
        // 无主窗口的应用刚被调用起来：确保对话框出现在前台（与 macOS 侧的 activate 对应）。
        let _ = SetForegroundWindow(window);
    }
}

/// 切换密码框的遮罩：`Show password` 勾上时显示明文。
///
/// # Safety
/// 只能由持有有效窗口句柄的对话框过程调用。
unsafe fn toggle_password_visibility(window: HWND) {
    // SAFETY: 控件 ID 来自资源脚本，句柄是当前对话框。
    unsafe {
        let visible = IsDlgButtonChecked(window, super::ID_SHOW_PASSWORD) == BST_CHECKED.0;
        let mask = if visible { 0 } else { PASSWORD_MASK as usize };
        let _ = SendDlgItemMessageW(
            window,
            super::ID_PASSWORD,
            EM_SETPASSWORDCHAR,
            WPARAM(mask),
            LPARAM(0),
        );
        // `EM_SETPASSWORDCHAR` 不会重画已经显示在框里的文本，必须让控件失效一次，
        // 否则要先敲一个键才看得到效果。
        if let Ok(password) = GetDlgItem(Some(window), super::ID_PASSWORD) {
            let _ = InvalidateRect(Some(password), None, true);
        }
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
    use windows::Win32::UI::Controls::EM_GETPASSWORDCHAR;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, FindWindowW, GetClassNameW, GetDlgCtrlID, GetDlgItem, GetWindowTextW,
        IsWindowVisible,
    };
    use windows::core::BOOL;

    /// `FindWindowW` 会找到进程里任意一个同名窗口，所以这些用例必须串行。
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        // 一个用例失败会让锁中毒，但后续用例依然应该能跑。
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 提示词在 `common` 里有用例；这里只做一次安全检查。
    #[test]
    fn prompt_message_is_never_empty() {
        for failed in [false, true] {
            let message = super::super::super::common::password_prompt_message(failed);
            assert!(!message.is_empty());
            assert!(message.len() < 60, "the prompt must fit on one line");
        }
    }

    /// 等对话框出现**且控件已建好**：对话框是资源模板，子控件按模板顺序逐个创建，而
    /// `FindWindowW` 在第一个子控件还没建完时就能找到窗口。
    ///
    /// 所以等“可见”而不是等某个具体控件：对话框管理器建完所有子控件、跑完 `WM_INITDIALOG`
    /// 之后才 ShowWindow，可见即完整，且不依赖模板里的控件顺序。
    fn wait_for_dialog() -> HWND {
        let title = super::super::wide("Password required");
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            // SAFETY: 标题是以 NUL 结尾的缓冲区，`window` 是本进程的窗口句柄。
            if let Ok(window) = unsafe { FindWindowW(None, PCWSTR(title.as_ptr())) }
                && !window.0.is_null()
                && unsafe { IsWindowVisible(window).as_bool() }
                && unsafe { GetDlgItem(Some(window), super::super::ID_PASSWORD) }.is_ok()
            {
                return window;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the password dialog did not appear");
    }

    /// 在同一个进程里驱动对话框：写文本、拨勾选框（`None` 表示不碰）、按按钮。
    fn drive_dialog(
        password: &str,
        button: u32,
        remember: Option<bool>,
        keep_original: Option<bool>,
    ) {
        let window = wait_for_dialog();
        let password = super::super::wide(password);
        // SAFETY: 同一个进程内的对话框；传进去的缓冲区在调用期间存活。
        unsafe {
            let _ = SetDlgItemTextW(window, super::super::ID_PASSWORD, PCWSTR(password.as_ptr()));
            for (identifier, value) in [
                (super::super::ID_REMEMBER, remember),
                (super::super::ID_KEEP_ORIGINAL, keep_original),
            ] {
                let Some(value) = value else { continue };
                let state = if value {
                    BST_CHECKED.0
                } else {
                    BST_UNCHECKED.0
                };
                let _ = SendDlgItemMessageW(
                    window,
                    identifier,
                    BM_SETCHECK,
                    WPARAM(state as usize),
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

    /// 读回提示词（先把对话框关掉再返回，见下面注释）。
    fn prompt_text_of(window: HWND) -> String {
        let mut buffer = [0_u16; 512];
        // SAFETY: 控件属于本进程的对话框；`buffer` 可写。
        let length = unsafe { GetDlgItemTextW(window, super::super::ID_PROMPT, &mut buffer) };
        String::from_utf16_lossy(&buffer[..length as usize])
    }

    /// 密码框当前的遮罩字符（0 = 明文）。
    fn mask_character(window: HWND) -> u16 {
        // SAFETY: 控件属于本进程的对话框。
        unsafe {
            SendDlgItemMessageW(
                window,
                super::super::ID_PASSWORD,
                EM_GETPASSWORDCHAR,
                WPARAM(0),
                LPARAM(0),
            )
            .0 as u16
        }
    }

    /// 点一次 `Show password`（勾选框：先拨状态，再发一条 `BN_CLICKED`）。
    fn click_show_password(window: HWND, checked: bool) {
        let state = if checked {
            BST_CHECKED.0
        } else {
            BST_UNCHECKED.0
        };
        // SAFETY: 控件属于本进程的对话框。
        unsafe {
            let _ = SendDlgItemMessageW(
                window,
                super::super::ID_SHOW_PASSWORD,
                BM_SETCHECK,
                WPARAM(state as usize),
                LPARAM(0),
            );
            let _ = SendMessageW(
                window,
                WM_COMMAND,
                Some(WPARAM(SHOW_PASSWORD_ID as usize)),
                Some(LPARAM(0)),
            );
        }
    }

    /// `EnumChildWindows` 的回调：记下控件的 ID。
    ///
    /// # Safety
    /// `lparam` 必须是调用方传入的 `&mut Vec<i32>`，且在枚举期间保持存活。
    unsafe extern "system" fn collect_identifier(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: 见函数的 `# Safety`。
        unsafe {
            let identifiers = &mut *(lparam.0 as *mut Vec<i32>);
            identifiers.push(GetDlgCtrlID(window));
        }
        true.into()
    }

    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn the_prompt_is_a_single_line_without_the_filename() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        let driver = std::thread::spawn(|| {
            let window = wait_for_dialog();
            let shown = prompt_text_of(window);
            // 先关掉对话框再断言：若先断言，失败会让主线程永远卡在模态循环里。
            drive_dialog("", ID_CANCEL, None, None);
            shown
        });
        let _ = show(false).expect("show the dialog");
        let shown = driver.join().expect("driver thread");

        assert_eq!(shown, "Enter the password:");
    }

    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn show_password_toggles_the_mask_character() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        let driver = std::thread::spawn(|| {
            let window = wait_for_dialog();
            let hidden = mask_character(window);
            click_show_password(window, true);
            let revealed = mask_character(window);
            click_show_password(window, false);
            let rehidden = mask_character(window);
            drive_dialog("", ID_CANCEL, None, None);
            (hidden, revealed, rehidden)
        });
        let _ = show(false).expect("show the dialog");
        let (hidden, revealed, rehidden) = driver.join().expect("driver thread");

        assert_eq!(hidden, PASSWORD_MASK, "the password starts hidden");
        assert_eq!(revealed, 0, "Show password must reveal the text");
        assert_eq!(rehidden, PASSWORD_MASK, "unchecking must hide it again");
    }

    /// `EnumChildWindows` 的回调：收集子窗口句柄。
    ///
    /// # Safety
    /// `lparam` 必须是调用方传入的 `&mut Vec<HWND>`，且在枚举期间保持存活。
    unsafe extern "system" fn collect_window(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: 见函数的 `# Safety`。
        unsafe {
            let windows = &mut *(lparam.0 as *mut Vec<HWND>);
            windows.push(window);
        }
        true.into()
    }

    /// 三个勾选项要有互不冲突的助记键（`Alt` + 字母），OK / Cancel 不要：`Alt` 组合才会
    /// 触发，单独敲字母是在往密码框里输入。
    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn checkbox_labels_have_unique_mnemonics() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        let driver = std::thread::spawn(|| {
            let window = wait_for_dialog();
            let mut children: Vec<HWND> = Vec::new();
            // SAFETY: `children` 在枚举期间存活；回调只往里 push。
            unsafe {
                let _ = EnumChildWindows(
                    Some(window),
                    Some(collect_window),
                    LPARAM((&raw mut children) as isize),
                );
            }

            let mut labels: Vec<(i32, String)> = Vec::new();
            for child in children {
                let mut class = [0_u16; 64];
                let mut text = [0_u16; 256];
                // SAFETY: 控件属于本进程的对话框；两个缓冲区都可写。
                let (class_length, text_length) = unsafe {
                    (
                        GetClassNameW(child, &mut class),
                        GetWindowTextW(child, &mut text),
                    )
                };
                let class_name = String::from_utf16_lossy(&class[..class_length as usize]);
                if class_name != "Button" {
                    continue;
                }
                labels.push((
                    unsafe { GetDlgCtrlID(child) },
                    String::from_utf16_lossy(&text[..text_length as usize]),
                ));
            }
            drive_dialog("", ID_CANCEL, None, None);
            labels
        });
        let _ = show(false).expect("show the dialog");
        let labels = driver.join().expect("driver thread");
        let label_of = |identifier: i32| {
            labels
                .iter()
                .find(|(candidate, _)| *candidate == identifier)
                .map(|(_, label)| label.clone())
                .unwrap_or_else(|| panic!("control {identifier} not found in {labels:?}"))
        };

        let mut mnemonics: Vec<char> = Vec::new();
        for identifier in [
            super::super::ID_SHOW_PASSWORD,
            super::super::ID_REMEMBER,
            super::super::ID_KEEP_ORIGINAL,
        ] {
            let label = label_of(identifier);
            let letter = label
                .find('&')
                .and_then(|index| label[index + 1..].chars().next());
            let Some(letter) = letter else {
                panic!("checkbox {identifier} has no mnemonic: {label:?}");
            };
            mnemonics.push(letter.to_ascii_uppercase());
        }

        mnemonics.sort_unstable();
        let mut unique = mnemonics.clone();
        unique.dedup();
        assert_eq!(mnemonics, unique, "duplicate mnemonics: {mnemonics:?}");

        for identifier in [IDOK.0, IDCANCEL.0] {
            let label = label_of(identifier);
            assert!(
                !label.contains('&'),
                "button {identifier} does not need a mnemonic: {label:?}"
            );
        }
    }

    /// 重复的控件 ID 会让 `GetDlgItem`/`SetDlgItemTextW` 只作用于其中一个，另一个（可能是
    /// 空的）盖在上面。
    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn the_template_has_no_duplicate_control_ids() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        let driver = std::thread::spawn(|| {
            let window = wait_for_dialog();
            let mut identifiers: Vec<i32> = Vec::new();
            // SAFETY: `identifiers` 在枚举期间存活；回调只往里 push。
            unsafe {
                let _ = EnumChildWindows(
                    Some(window),
                    Some(collect_identifier),
                    LPARAM((&raw mut identifiers) as isize),
                );
            }
            drive_dialog("", ID_CANCEL, None, None);
            identifiers
        });
        let _ = show(false).expect("show the dialog");
        let mut identifiers = driver.join().expect("driver thread");

        identifiers.sort_unstable();
        let mut unique = identifiers.clone();
        unique.dedup();
        assert_eq!(
            identifiers, unique,
            "the template has duplicate control ids: {identifiers:?}"
        );
        assert!(!identifiers.is_empty(), "no controls were enumerated");
    }

    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn extract_returns_the_typed_password_and_the_checkbox_states() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        let driver =
            std::thread::spawn(|| drive_dialog("secret", ID_EXTRACT, Some(false), Some(true)));
        let response = show(true)
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
        let driver = std::thread::spawn(|| {
            let window = wait_for_dialog();
            let show_password = super::is_checked(window, super::super::ID_SHOW_PASSWORD);
            drive_dialog("secret", ID_EXTRACT, None, None);
            show_password
        });
        let response = show(false)
            .expect("show the dialog")
            .expect("the dialog must return a response");
        let show_password = driver.join().expect("driver thread");

        assert_eq!(response.password, "secret");
        assert!(response.remember, "Remember this password defaults to on");
        assert!(!response.keep_original, "Keep the original defaults to off");
        assert!(!show_password, "Show password defaults to off");
    }

    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn cancel_returns_no_response() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        let driver = std::thread::spawn(|| drive_dialog("", ID_CANCEL, None, None));
        let response = show(false).expect("show the dialog");
        driver.join().expect("driver thread");

        assert!(response.is_none(), "cancelling must not produce a password");
    }
}
