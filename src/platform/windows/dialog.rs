//! 密码弹窗（设计 §7）：资源表里的 `DIALOGEX` + `DialogBoxParamW`。
//!
//! 用资源对话框而不是在代码里摆控件，是因为 Tab 导航、Enter 提交、Esc 取消、DPI 缩放
//! 全部由对话框管理器提供（D6）—— 因此不再需要自己处理键盘消息。
//!
//! 提示文字是"提示词 + 文件名"的一句话（`Enter the password: <name>`），最多两行：
//!
//! - 够短：只占一行，收起第二行并把下面的控件上移（对话框随之变矮）；
//! - 略长：占两行；
//! - 太长：在两行内对**文件名**做中间省略，提示词永远完整。
//!
//! 行数用 `DrawTextW(DT_CALCRECT)` 测量：static 控件绘制时走同一套排版逻辑，所以测量与
//! 显示一致。按字符宽度估算（`GetTextExtentPoint32W`）既不准，对字符数也不单调。

use std::error::Error;
use std::path::Path;

use ezz::{PasswordPrompt, PasswordResponse};
use log::warn;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DEFAULT_GUI_FONT, DT_CALCRECT, DT_EDITCONTROL, DT_NOPREFIX, DT_SINGLELINE, DT_WORDBREAK,
    DrawTextW, GetDC, GetStockObject, HDC, HGDIOBJ, MapWindowPoints, ReleaseDC, SelectObject,
};
use windows::Win32::UI::Controls::{
    BST_CHECKED, EM_SETLIMITTEXT, EM_SETPASSWORDCHAR, IsDlgButtonChecked,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_SETCHECK, BN_CLICKED, DialogBoxParamW, EndDialog, GWLP_USERDATA, GetClientRect, GetDlgItem,
    GetDlgItemTextW, GetWindowLongPtrW, GetWindowRect, ICON_BIG, ICON_SMALL, IDCANCEL, IDOK,
    LoadIconW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendDlgItemMessageW,
    SendMessageW, SetDlgItemTextW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, WM_CLOSE,
    WM_COMMAND, WM_GETFONT, WM_INITDIALOG, WM_SETICON,
};
use windows::core::PCWSTR;

/// 密码长度上限（与原实现一致）。
const PASSWORD_LIMIT: usize = 1024;

/// 测量排版时留出的余量（像素）：static 的绘制与测量的换行边界可能差一两个像素，
/// 宁可测得略宽松一点（多切掉几个字符不会难看，多出来的字被裁掉才会）。
const WIDTH_MARGIN: i32 = 4;

/// 提示文字最多占几行（设计 §7）。模板按两行预留高度，一行时由代码收起第二行。
const MAX_PROMPT_LINES: usize = 2;

/// 省略文件名时至少保留的字符数。
const MIN_NAME_CHARS: usize = 3;

/// 对话框按钮的命令 ID。取值沿用 winuser.h 的 `IDOK` / `IDCANCEL`（它们是
/// `MESSAGEBOX_RESULT` 类型，不能直接用在控件 ID 的位置上）。
const ID_EXTRACT: u32 = IDOK.0 as u32;
const ID_CANCEL: u32 = IDCANCEL.0 as u32;

/// 对话框上下文：由 `DialogBoxParamW` 的 `lParam` 传入，随后挂在窗口的 `DWLP_USER` 上。
struct DialogContext {
    /// 提示词（`Enter the password:` 或重试时的另一种），文件名接在它后面。
    message: &'static str,
    filename: String,
    response: Option<PasswordResponse>,
}

/// 提示文字的排版结果。
struct FittedPrompt {
    /// 送进控件的文字：已确保不超过 `MAX_PROMPT_LINES` 行。
    text: String,
    /// 实际需要几行。
    lines: usize,
    /// 一行的高度（像素），收起第二行时要用；测量失败时为 0。
    line_height: i32,
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
        message: super::super::common::password_prompt_message(previous_attempt_failed),
        filename: input
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| input.as_os_str().to_string_lossy().into_owned()),
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

/// `WM_INITDIALOG` 里的一次性设置：提示文字、布局收起、图标、密码框上限与遮罩字符、
/// 勾选项默认值。此时对话框还没显示，所以调整尺寸看不到跳变。
///
/// # Safety
/// 只能由持有有效窗口句柄的对话框过程调用。
unsafe fn initialize_controls(window: HWND, context: &DialogContext) {
    let fitted = fit_prompt(window, context);
    let text = super::wide(&fitted.text);
    // SAFETY: 控件 ID 来自资源脚本；缓冲区以 NUL 结尾且在调用期间存活。
    unsafe {
        let _ = SetDlgItemTextW(window, super::ID_PROMPT, PCWSTR(text.as_ptr()));
    }
    if fitted.lines < MAX_PROMPT_LINES {
        // SAFETY: 句柄属于当前对话框。
        unsafe { collapse_second_line(window, fitted.line_height) };
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

/// 用提示控件自己的字体和宽度测量，并把整句压进 `MAX_PROMPT_LINES` 行。
///
/// 拿不到控件或 DC 时退回到"不裁剪"：宁可让控件自己裁，也不要拿错误的测量结果去改文字。
fn fit_prompt(window: HWND, context: &DialogContext) -> FittedPrompt {
    let untouched = || FittedPrompt {
        text: super::super::common::password_prompt_with_message(
            context.message,
            &context.filename,
        ),
        lines: MAX_PROMPT_LINES,
        line_height: 0,
    };

    // SAFETY: 控件属于当前对话框；DC 在函数返回前释放，字体用完恢复。
    unsafe {
        let Ok(control) = GetDlgItem(Some(window), super::ID_PROMPT) else {
            return untouched();
        };
        let mut bounds = RECT::default();
        if GetClientRect(control, &raw mut bounds).is_err() {
            return untouched();
        }
        let width = bounds.right - WIDTH_MARGIN;
        let device = GetDC(Some(control));
        if device.is_invalid() || width <= 0 {
            return untouched();
        }
        let font = SendMessageW(control, WM_GETFONT, None, None).0;
        let previous = if font == 0 {
            SelectObject(device, GetStockObject(DEFAULT_GUI_FONT))
        } else {
            SelectObject(device, HGDIOBJ(font as *mut core::ffi::c_void))
        };

        // 一行的高度由同一套排版逻辑给出，比 `TEXTMETRIC` 更贴近实际绘制。
        let line_height = drawn_height(device, "Xg", width, true).max(1);
        let lines_of = |text: &str| {
            let height = drawn_height(device, text, width, false);
            (height + line_height - 1) / line_height
        };
        let mut fitted = fit_prompt_text(
            context.message,
            &context.filename,
            MAX_PROMPT_LINES,
            &|text| lines_of(text) as usize,
        );
        fitted.line_height = line_height;

        SelectObject(device, previous);
        ReleaseDC(Some(control), device);
        fitted
    }
}

/// 用 static 的排版逻辑量一段文字要占多高（`DT_CALCRECT` 只测量、不绘制）。
///
/// 多行时必须带 `DT_EDITCONTROL`：它让 DrawText 像多行编辑控件那样换行（**超长单词
/// 也会被切开**），并且不会像单独用 `DT_WORDBREAK` 那样把矩形加宽去容纳最长的单词
/// —— 实测一个 163 字符的连续文件名：`DT_WORDBREAK` 量出 25 px（一行）、宽度被撞到
/// 1622 px，而加上 `DT_EDITCONTROL` 量出 100 px（四行）、宽度保持 450 px。
/// 模板里那个控件带 `SS_EDITCONTROL`，所以两边规则一致。
fn drawn_height(device: HDC, text: &str, width: i32, single_line: bool) -> i32 {
    let mut text = super::wide(text);
    let mut bounds = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: 0,
    };
    let format = if single_line {
        DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX
    } else {
        DT_CALCRECT | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX
    };
    let length = text.len() - 1;
    // SAFETY: `device` 已选好字体；缓冲区以 NUL 结尾（终止符不计入长度）。
    unsafe { DrawTextW(device, &mut text[..length], &raw mut bounds, format) }
}

/// 把"提示 + 文件名"压进 `max_lines` 行以内（设计 §7）。**只裁文件名**。
///
/// `lines_of` 是测量函数：真实实现用 `DrawTextW`，用例里注入假的。二分之后再逐字符收紧，
/// 因为排版行数对字符数不是严格单调的（字符宽度不同、可以换行的位置也不同）。
fn fit_prompt_text(
    message: &str,
    filename: &str,
    max_lines: usize,
    lines_of: &dyn Fn(&str) -> usize,
) -> FittedPrompt {
    let compose = |name: &str| super::super::common::password_prompt_with_message(message, name);
    let result = |text: String, lines_of: &dyn Fn(&str) -> usize| FittedPrompt {
        lines: lines_of(&text),
        text,
        line_height: 0,
    };

    let full = compose(filename);
    if lines_of(&full) <= max_lines {
        return result(full, lines_of);
    }

    let characters = filename.chars().count();
    let (mut low, mut high) = (MIN_NAME_CHARS, characters);
    let mut best: Option<usize> = None;
    while low <= high {
        let middle = low + (high - low) / 2;
        let candidate = compose(&super::super::common::truncate_middle(filename, middle));
        if lines_of(&candidate) <= max_lines {
            best = Some(middle);
            low = middle + 1;
        } else if middle == 0 {
            break;
        } else {
            high = middle - 1;
        }
    }

    // 二分只保证“这个字符数可行”，边界附近可能还能再放一个字符，所以逐字收紧。
    // `MIN_NAME_CHARS` 以下不再收缩：再短就只剩下省略号，没有信息量了。
    let mut kept = best.unwrap_or(MIN_NAME_CHARS);
    loop {
        let candidate = compose(&super::super::common::truncate_middle(filename, kept));
        if lines_of(&candidate) <= max_lines || kept <= MIN_NAME_CHARS {
            return result(candidate, lines_of);
        }
        kept -= 1;
    }
}

/// 提示只占一行时：收起提示控件的第二行，缩短对话框，并把下面的控件上移一行。
///
/// # Safety
/// 只能由持有有效窗口句柄的对话框过程调用，且必须在对话框显示之前。
unsafe fn collapse_second_line(window: HWND, line_height: i32) {
    if line_height <= 0 {
        return;
    }

    // SAFETY: 所有句柄都属于当前对话框。
    unsafe {
        if let Ok(prompt) = GetDlgItem(Some(window), super::ID_PROMPT) {
            let mut bounds = RECT::default();
            if GetClientRect(prompt, &raw mut bounds).is_ok() {
                let _ = SetWindowPos(
                    prompt,
                    None,
                    0,
                    0,
                    bounds.right,
                    (bounds.bottom - line_height).max(line_height),
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }

        for identifier in [
            super::ID_PASSWORD,
            super::ID_REMEMBER,
            super::ID_KEEP_ORIGINAL,
            ID_EXTRACT as i32,
            ID_CANCEL as i32,
        ] {
            move_up(window, identifier, line_height);
        }

        let mut dialog = RECT::default();
        if GetWindowRect(window, &raw mut dialog).is_ok() {
            let height = (dialog.bottom - dialog.top - line_height).max(line_height * 4);
            // 对话框是 `DS_CENTER` 的：变矮之后只往下收会显得偏心，上移半个行高补偿。
            let _ = SetWindowPos(
                window,
                None,
                dialog.left,
                dialog.top + line_height / 2,
                dialog.right - dialog.left,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// 把一个控件上移 `dy` 像素（坐标从屏幕映射回父窗口客户区，避免自己算原点）。
///
/// # Safety
/// 句柄必须属于当前对话框。
unsafe fn move_up(window: HWND, identifier: i32, dy: i32) {
    // SAFETY: 句柄属于当前对话框；点集只含一个点。
    unsafe {
        let Ok(control) = GetDlgItem(Some(window), identifier) else {
            return;
        };
        let mut bounds = RECT::default();
        if GetWindowRect(control, &raw mut bounds).is_err() {
            return;
        }
        let mut point = POINT {
            x: bounds.left,
            y: bounds.top,
        };
        // SAFETY: 单个点的切片；两个句柄都有效。
        let _ = MapWindowPoints(None, Some(window), std::slice::from_mut(&mut point));
        let _ = SetWindowPos(
            control,
            None,
            point.x,
            point.y - dy,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
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
    use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, FindWindowW, GetDlgCtrlID};
    use windows::core::BOOL;

    /// 这些用例都要驱动*同一个*标题的模态对话框，而 `FindWindowW` 会找到进程里的任意一个。
    /// 测试默认并行运行，所以必须自己串行化（否则会去操纵别的用例的窗口）。
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        // 一个用例失败会让锁中毒，但后续用例依然应该能跑。
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 文案随“上一次失败”变化（设计 §7）：纯函数，在 `common` 里有用例；
    /// 这里只留一个安全检查，保证弹窗拿到的不是空串、也不长到独占不了第一行。
    #[test]
    fn prompt_message_is_never_empty() {
        for failed in [false, true] {
            let message = super::super::super::common::password_prompt_message(failed);
            assert!(!message.is_empty());
            assert!(
                message.len() < 60,
                "the message must fit on one line: {message}"
            );
        }
    }

    /// 假的测量函数：每行 40 个字符（真实控件在 150% 缩放下大约 50–60）。
    /// 真实测量是 `DrawTextW`，这里只是把拟合逻辑单独拿出来推理。
    fn forty_per_line(text: &str) -> usize {
        text.chars().count().div_ceil(40)
    }

    #[test]
    fn fit_prompt_keeps_short_prompts_unchanged() {
        let fitted = fit_prompt_text(
            "Enter the password:",
            "a.7z",
            MAX_PROMPT_LINES,
            &forty_per_line,
        );
        assert_eq!(fitted.text, "Enter the password: a.7z");
        assert_eq!(fitted.lines, 1);
    }

    #[test]
    fn fit_prompt_uses_two_lines_before_truncating() {
        let name = "0123456789012345678901234567"; // 一行放不下的长度
        let fitted = fit_prompt_text(
            "Enter the password:",
            name,
            MAX_PROMPT_LINES,
            &forty_per_line,
        );
        assert_eq!(fitted.text, format!("Enter the password: {name}"));
        assert_eq!(fitted.lines, 2);
        assert!(!fitted.text.contains('…'), "two lines are enough here");
    }

    #[test]
    fn fit_prompt_truncates_only_the_filename() {
        let name = "0123456789012345678901234567890123456789012345678901234567890123";
        let fitted = fit_prompt_text(
            "Enter the password:",
            name,
            MAX_PROMPT_LINES,
            &forty_per_line,
        );

        assert!(
            fitted.text.starts_with("Enter the password:"),
            "the prompt must stay intact: {}",
            fitted.text
        );
        assert!(fitted.text.contains('…'), "expected a middle ellipsis");
        assert!(fitted.text.ends_with('3'), "the tail must survive");
        assert!(fitted.lines <= MAX_PROMPT_LINES, "{}", fitted.lines);
    }

    #[test]
    fn fit_prompt_never_grows_the_original() {
        // 名字本来就能放下：一个字都不该改。
        let name = "archive.7z";
        for message in [
            "Enter the password:",
            "The password was incorrect. Try again:",
        ] {
            let fitted = fit_prompt_text(message, name, MAX_PROMPT_LINES, &forty_per_line);
            assert!(fitted.text.ends_with(name));
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

    /// 打开一次对话框并带回三个观察值：对话框高度、密码框相对客户区的偏移、
    /// 提示控件的高度。
    ///
    /// 偏移用客户区坐标（而不是屏幕坐标）：2a 会把对话框自己下移半个行高来保持居中，
    /// 用屏幕坐标会让断言依赖那个补偿值。
    ///
    /// 断言放在调用方（主线程）：driver 线程里断言失败会让主线程永远卡在模态循环里。
    fn measure_dialog(name: &str) -> (i32, i32, i32) {
        let driver = std::thread::spawn(|| {
            let window = wait_for_dialog();
            // SAFETY: 句柄属于本进程的有效对话框。
            let measured = unsafe {
                let mut dialog = RECT::default();
                let mut prompt = RECT::default();
                let _ = GetWindowRect(window, &raw mut dialog);
                let password_control =
                    GetDlgItem(Some(window), super::super::ID_PASSWORD).expect("password control");
                let mut password = RECT::default();
                let _ = GetWindowRect(password_control, &raw mut password);
                let mut top_left = POINT {
                    x: password.left,
                    y: password.top,
                };
                let _ = MapWindowPoints(None, Some(window), std::slice::from_mut(&mut top_left));
                let prompt_control =
                    GetDlgItem(Some(window), super::super::ID_PROMPT).expect("prompt control");
                let _ = GetWindowRect(prompt_control, &raw mut prompt);
                (
                    dialog.bottom - dialog.top,
                    top_left.y,
                    prompt.bottom - prompt.top,
                )
            };
            drive_dialog("", ID_CANCEL, None, None);
            measured
        });
        let _ = show(Path::new(name), false).expect("show the dialog");
        driver.join().expect("driver thread")
    }

    /// 2a：文字只占一行时，第二行被收起——对话框矮一行，密码框高一行。
    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn prompt_layout_collapses_when_the_text_fits_on_one_line() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        let (short_height, short_password, short_prompt) = measure_dialog("a.7z");
        let (long_height, long_password, long_prompt) =
            measure_dialog("a-very-long-archive-name-that-needs-two-lines-to-show-2026-10-04.7z");

        // 一行的高度由两个对话框的提示控件高度差给出：控件本身还带一点内边距，
        // 所以不能拿控制高度的一半当一行。
        let line_height = long_prompt - short_prompt;
        assert!(line_height > 0, "short={short_prompt} long={long_prompt}");
        assert_eq!(
            short_password,
            long_password - line_height,
            "the password box must move up by one line"
        );
        assert_eq!(
            short_height,
            long_height - line_height,
            "the dialog must shrink by one line"
        );
    }

    /// 2c：两行都放不下时，只有文件名被中间省略，提示词完整。
    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn a_long_filename_is_ellipsized_inside_two_lines() {
        let _serial = serial();
        super::super::initialize_process().expect("initialize process");

        // 约 140 字符：两行（约 110 字符）内放不下，必然触发省略。
        let long_name = concat!(
            "a-very-long-archive-name-that-cannot-possibly-fit-into-two-lines-2026-10-04",
            "-and-then-some-more-characters-to-be-sure-it-does-not-fit-at-all.7z"
        );
        let full_length = long_name.chars().count();
        let driver = std::thread::spawn(move || {
            let window = wait_for_dialog();
            let mut buffer = [0_u16; 4096];
            // SAFETY: 控件属于本进程的对话框；`buffer` 可写。
            let length = unsafe { GetDlgItemTextW(window, super::super::ID_PROMPT, &mut buffer) };
            let shown = String::from_utf16_lossy(&buffer[..length as usize]);
            drive_dialog("", ID_CANCEL, None, None);
            shown
        });
        let _ = show(Path::new(long_name), false).expect("show the dialog");
        let shown = driver.join().expect("driver thread");

        assert!(
            shown.starts_with("Enter the password:"),
            "the prompt must stay intact: {shown}"
        );
        assert!(
            shown.contains('…'),
            "the name should be ellipsized: {shown}"
        );
        assert!(shown.ends_with("7z"), "the tail should survive: {shown}");
        assert!(
            shown.chars().count() < full_length,
            "the name was not shortened: {shown}"
        );
    }

    /// 模板里不得有重复的控件 ID。
    ///
    /// 重复时 `GetDlgItem`/`SetDlgItemTextW` 只作用于其中一个，另一个（可能是空的）盖在上面
    /// —— 这个 bug 真的发生过：提示文字被一个同 ID 的空标签遮住。
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
        let _ = show(Path::new("archive.7z"), false).expect("show the dialog");
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
