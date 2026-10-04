use ezz::{ExtractionWorkflow, PasswordPrompt, PasswordResponse};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationPolicy,
    NSApplicationDelegate, NSApplicationDelegateReply, NSButton, NSControlStateValueOn,
    NSModalResponseOK, NSOpenPanel, NSSecureTextField, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectNSDelayedPerforming,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, ns_string,
};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;
use std::path::{Path, PathBuf};

use super::RunOutcome;
use super::common::{PlatformPaths, initialize_logging, password_prompt_message, report_outcome};

mod notifications;

pub(crate) use notifications::show_notification;

/// 处理完成后退出前的让出时间（秒）。
///
/// 0 表示只让出一个 run loop 回合，先派发同一次激活里已经排队的打开事件。
const QUIT_AFTER: f64 = 0.0;

struct AppDelegateIvars {
    workflow: ExtractionWorkflow,
    pending: RefCell<VecDeque<PathBuf>>,
    launched: Cell<bool>,
    processing: Cell<bool>,
    /// 文件选择器正在运行嵌套 run loop；期间不得开始提取，也不得退出。
    picker_open: Cell<bool>,
    /// 本次运行至少有一个输入失败（决定退出码）。
    failed: Cell<bool>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = AppDelegateIvars]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            self.ivars().launched.set(true);
            let app = NSApplication::sharedApplication(self.mtm());
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);

            // 启动时没有待处理输入才显示选择器：被打开文件启动时，AppKit 会把
            // `openFiles:` 送到本方法之前。
            if self.ivars().pending.borrow().is_empty() {
                self.ivars().picker_open.set(true);
                let picked = select_files(self.mtm());
                self.ivars().picker_open.set(false);
                self.ivars().pending.borrow_mut().extend(picked);
            }
            self.process_pending();
        }

        #[unsafe(method(application:openFiles:))]
        fn application_open_files(
            &self,
            sender: &NSApplication,
            filenames: &NSArray<NSString>,
        ) {
            self.ivars().pending.borrow_mut().extend(
                filenames
                    .iter()
                    .map(|filename| PathBuf::from(filename.to_string())),
            );
            sender.replyToOpenOrPrint(NSApplicationDelegateReply::Success);
            if self.ivars().launched.get() {
                self.process_pending();
            }
        }
    }

    impl AppDelegate {
        /// 延迟退出检查。
        ///
        /// 只在确实无事可做时退出：提取进行中、选择器打开中、还有待处理输入都不退。
        #[unsafe(method(quitIfIdle:))]
        fn quit_if_idle(&self, _sender: Option<&AnyObject>) {
            if self.ivars().processing.get() || self.ivars().picker_open.get() {
                return;
            }
            if self.ivars().pending.borrow().is_empty() {
                NSApplication::sharedApplication(self.mtm()).terminate(None);
            }
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker, workflow: ExtractionWorkflow) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars {
            workflow,
            pending: RefCell::new(VecDeque::new()),
            launched: Cell::new(false),
            processing: Cell::new(false),
            picker_open: Cell::new(false),
            failed: Cell::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn process_pending(&self) {
        // 主线程同步执行就是串行化本身；重入守卫挡住模态对话框嵌套 run loop 期间到达的
        // 打开事件，它们留在 `pending` 里，由本次批次结束后的同一层循环取走。
        if self.ivars().picker_open.get() || self.ivars().processing.replace(true) {
            return;
        }

        loop {
            let inputs: Vec<_> = self.ivars().pending.borrow_mut().drain(..).collect();
            if inputs.is_empty() {
                break;
            }

            for input in &inputs {
                let result = self.ivars().workflow.extract(input);
                if result.is_err() {
                    self.ivars().failed.set(true);
                }
                report_outcome(input, &result);
            }
        }

        self.ivars().processing.set(false);
        unsafe { self.performSelector_withObject_afterDelay(sel!(quitIfIdle:), None, QUIT_AFTER) };
    }

    /// 本次运行是否有输入失败。在 `run()` 里于 `NSApplication::run` 返回后读取。
    fn failed(&self) -> bool {
        self.ivars().failed.get()
    }
}

struct MacPasswordPrompt;

impl PasswordPrompt for MacPasswordPrompt {
    fn request_password(
        &self,
        _input: &Path,
        previous_attempt_failed: bool,
    ) -> Option<PasswordResponse> {
        let mtm = MainThreadMarker::new().expect("password prompt must run on the main thread");
        let alert = NSAlert::new(mtm);
        alert.setMessageText(ns_string!("Password required"));
        // 弹窗里不显示文件名：它在通知与日志里。
        alert.setInformativeText(&NSString::from_str(password_prompt_message(
            previous_attempt_failed,
        )));
        alert.addButtonWithTitle(ns_string!("Extract"));
        alert.addButtonWithTitle(ns_string!("Cancel"));

        let accessory = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 86.0)),
        );
        let password = NSSecureTextField::initWithFrame(
            NSSecureTextField::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 58.0), NSSize::new(360.0, 24.0)),
        );
        password.setPlaceholderString(Some(ns_string!("Password")));
        let remember = unsafe {
            NSButton::checkboxWithTitle_target_action(
                ns_string!("Remember this password"),
                None,
                None,
                mtm,
            )
        };
        remember.setFrame(NSRect::new(
            NSPoint::new(0.0, 28.0),
            NSSize::new(360.0, 22.0),
        ));
        remember.setState(NSControlStateValueOn);
        let keep_original = unsafe {
            NSButton::checkboxWithTitle_target_action(
                ns_string!("Keep the original archive"),
                None,
                None,
                mtm,
            )
        };
        keep_original.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(360.0, 22.0),
        ));
        accessory.addSubview(&password);
        accessory.addSubview(&remember);
        accessory.addSubview(&keep_original);
        alert.setAccessoryView(Some(&accessory));

        // accessory 应用不先激活的话，模态框可能出现在其他窗口后面。
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);

        if alert.runModal() != NSAlertFirstButtonReturn {
            return None;
        }

        Some(PasswordResponse {
            password: password.stringValue().to_string(),
            remember: remember.state() == NSControlStateValueOn,
            keep_original: keep_original.state() == NSControlStateValueOn,
        })
    }
}

pub fn run() -> Result<RunOutcome, Box<dyn Error>> {
    let paths = PlatformPaths::discover()?;
    initialize_logging(&paths.log_file)?;

    // 通知需要用户授权：放在提取之前，让首次运行时的授权框先出现。
    notifications::request_authorization();

    // 引擎在启动时解析并校验一次：缺失时 main() 弹一次明确提示，不让每个输入各报一次。
    let workflow = ExtractionWorkflow::with_password_support(
        ezz::locate_engine()?,
        paths.password_database,
        MacPasswordPrompt,
    );

    let mtm = MainThreadMarker::new().ok_or("Ezz must start on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let delegate = AppDelegate::new(mtm, workflow);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
    Ok(if delegate.failed() {
        RunOutcome::Failed
    } else {
        RunOutcome::Succeeded
    })
}

pub fn show_fatal_error(message: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(ns_string!("Ezz could not start"));
    alert.setInformativeText(&NSString::from_str(message));
    alert.addButtonWithTitle(ns_string!("OK"));
    alert.runModal();
}

fn select_files(mtm: MainThreadMarker) -> Vec<PathBuf> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(true);
    panel.setResolvesAliases(true);
    if panel.runModal() != NSModalResponseOK {
        return Vec::new();
    }

    panel
        .URLs()
        .iter()
        .filter_map(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()))
        .collect()
}
