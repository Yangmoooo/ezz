use std::error::Error;
use std::path::{Path, PathBuf};

use ezz::{ExtractionWorkflow, PasswordPrompt, PasswordResponse};
use log::warn;
use native_windows_derive::NwgUi;
use native_windows_gui as nwg;
use nwg::NativeUi;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows::core::PCWSTR;

use super::RunOutcome;
use super::common::{PlatformPaths, initialize_logging, report_outcome, report_skipped};

/// 全局串行化互斥体（设计 §3.3）。
///
/// `Local\` 命名空间与密码库的每用户作用域对齐；名字里不带版本号，跳版本也互斥。
const EXTRACT_MUTEX: &str = "Local\\io.github.yangmoooo.ezz.extract";

const ICON_DATA: &[u8] = include_bytes!("../../assets/icon/ezz.ico");

/// 进程启动即持有、直到退出的命名互斥体：一个用户会话里只能有一个 ezz。
///
/// 加锁早于文件选择器与密码弹窗，因此**不需要推理"交互期间该不该持锁"** —— 拒绝永远
/// 发生在调用到达的那一刻，不会出现"用户已经选完文件才被告知跳过"。拿到之后**不再释放**，
/// 由进程退出交给系统回收（不需 `ReleaseMutex`），因此不实现 `Drop`。
struct ExtractionLock {
    /// 句柄故意不在进程内关闭：关闭即等于释放。
    _handle: HANDLE,
}

impl ExtractionLock {
    /// 非阻塞尝试获取。返回 `None` 表示另一个 ezz 正在提取，本次调用必须立即拒绝。
    fn try_acquire() -> Option<Self> {
        let name = wide(EXTRACT_MUTEX);
        // 安全：`name` 是以 NUL 结尾的 UTF-16 缓冲区，在调用期间保持存活；`None` 表示
        // 使用默认安全属性（§3.3 不需要跨用户共享）。
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.ok()?;

        // 安全：`handle` 刚刚由 `CreateMutexW` 返回且有效；等待 0 毫秒即非阻塞尝试。
        let status = unsafe { WaitForSingleObject(handle, 0) };
        // WAIT_OBJECT_0：互斥体现在归本次调用所有。
        // WAIT_ABANDONED：上一个持有者崩溃或被强制结束，互斥体已释放，同样归我们所有。
        // 其余（WAIT_TIMEOUT）：别人正持有 —— 立即放弃，不进入任何等待状态。
        if status == WAIT_OBJECT_0 || status == WAIT_ABANDONED {
            return Some(Self { _handle: handle });
        }

        // 安全：拒绝路径上我们从未获得所有权，这个句柄必须立刻关闭，否则会泄漏。
        unsafe {
            let _ = CloseHandle(handle);
        }
        None
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[derive(Default, NwgUi)]
pub struct PasswordDialog {
    #[nwg_resource(source_bin: Some(ICON_DATA))]
    icon: nwg::Icon,

    #[nwg_control(
        title: "Password required",
        center: true,
        size: (390, 225),
        flags: "WINDOW|VISIBLE",
        icon: Some(&data.icon)
    )]
    #[nwg_events(
        OnInit: [PasswordDialog::focus_password],
        OnKeyEnter: [PasswordDialog::accept],
        OnWindowClose: [PasswordDialog::cancel]
    )]
    window: nwg::Window,

    #[nwg_control(position: (20, 16), size: (350, 42))]
    information: nwg::Label,

    #[nwg_control(
        position: (20, 64),
        size: (350, 25),
        limit: 1024,
        password: Some('*'),
        focus: true
    )]
    password: nwg::TextInput,

    #[nwg_control(
        text: "Remember this password",
        position: (20, 101),
        size: (350, 24),
        check_state: nwg::CheckBoxState::Checked
    )]
    remember: nwg::CheckBox,

    #[nwg_control(
        text: "Keep the original archive",
        position: (20, 130),
        size: (350, 24)
    )]
    keep_original: nwg::CheckBox,

    #[nwg_control(text: "Extract", position: (194, 172), size: (82, 30))]
    #[nwg_events(OnButtonClick: [PasswordDialog::accept])]
    accept_button: nwg::Button,

    #[nwg_control(text: "Cancel", position: (288, 172), size: (82, 30))]
    #[nwg_events(OnButtonClick: [PasswordDialog::cancel])]
    cancel_button: nwg::Button,

    response: std::cell::RefCell<Option<PasswordResponse>>,
}

impl PasswordDialog {
    fn show(
        input: &Path,
        previous_attempt_failed: bool,
    ) -> Result<Option<PasswordResponse>, nwg::NwgError> {
        let dialog = PasswordDialog::build_ui(Default::default())?;
        let filename = input
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| input.as_os_str().to_string_lossy());
        let information = if previous_attempt_failed {
            format!("The password for {filename} was incorrect. Try again.")
        } else {
            format!("Enter the password for {filename}.")
        };
        dialog.information.set_text(&information);
        nwg::dispatch_thread_events();
        Ok(dialog.response.borrow_mut().take())
    }

    fn focus_password(&self) {
        self.password.set_focus();
    }

    fn accept(&self) {
        self.response.replace(Some(PasswordResponse {
            password: self.password.text(),
            remember: self.remember.check_state() == nwg::CheckBoxState::Checked,
            keep_original: self.keep_original.check_state() == nwg::CheckBoxState::Checked,
        }));
        nwg::stop_thread_dispatch();
    }

    fn cancel(&self) {
        nwg::stop_thread_dispatch();
    }
}

struct WindowsPasswordPrompt;

impl PasswordPrompt for WindowsPasswordPrompt {
    fn request_password(
        &self,
        input: &Path,
        previous_attempt_failed: bool,
    ) -> Option<PasswordResponse> {
        match PasswordDialog::show(input, previous_attempt_failed) {
            Ok(response) => response,
            Err(error) => {
                warn!("could not show password dialog: {error}");
                None
            }
        }
    }
}

pub fn run() -> Result<RunOutcome, Box<dyn Error>> {
    let paths = PlatformPaths::discover()?;
    initialize_logging(&paths.log_file)?;
    nwg::init()?;
    nwg::Font::set_global_family("Segoe UI")?;

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
        select_files()?
    } else {
        inputs
    };
    if inputs.is_empty() {
        // 用户在选择器里取消：什么都没要求，不是失败。
        return Ok(RunOutcome::Succeeded);
    }

    let workflow = ExtractionWorkflow::with_password_support(
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
    let _ = nwg::init();
    nwg::error_message("ezz could not start", message);
}

fn select_files() -> Result<Vec<PathBuf>, nwg::NwgError> {
    let mut dialog = nwg::FileDialog::default();
    nwg::FileDialog::builder()
        .title("Select files to extract")
        .action(nwg::FileDialogAction::Open)
        .multiselect(true)
        .build(&mut dialog)?;
    if !dialog.run::<&nwg::Window>(None) {
        return Ok(Vec::new());
    }
    dialog
        .get_selected_items()
        .map(|items| items.into_iter().map(PathBuf::from).collect())
}
