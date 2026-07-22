use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;
use std::path::{Path, PathBuf};

use ezz::{DesktopApplication, ExtractionWorkflow, PasswordPrompt, PasswordResponse};
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

use super::common::{PlatformPaths, finish_batch, initialize_logging, notify_started};

struct AppDelegateIvars {
    application: DesktopApplication,
    pending: RefCell<VecDeque<PathBuf>>,
    launched: Cell<bool>,
    processing: Cell<bool>,
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

            if self.ivars().pending.borrow().is_empty() {
                self.ivars().pending.borrow_mut().extend(select_files(self.mtm()));
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
        #[unsafe(method(terminateIfIdle:))]
        fn terminate_if_idle(&self, _sender: Option<&AnyObject>) {
            if !self.ivars().processing.get() && self.ivars().pending.borrow().is_empty() {
                NSApplication::sharedApplication(self.mtm()).terminate(None);
            }
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker, application: DesktopApplication) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars {
            application,
            pending: RefCell::new(VecDeque::new()),
            launched: Cell::new(false),
            processing: Cell::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn process_pending(&self) {
        if self.ivars().processing.replace(true) {
            return;
        }

        loop {
            let inputs: Vec<_> = self.ivars().pending.borrow_mut().drain(..).collect();
            if inputs.is_empty() {
                break;
            }
            notify_started(inputs.len());
            let report = self.ivars().application.process_files(inputs);
            finish_batch(&report);
        }

        self.ivars().processing.set(false);
        unsafe { self.performSelector_withObject_afterDelay(sel!(terminateIfIdle:), None, 0.75) };
    }
}

struct MacPasswordPrompt;

impl PasswordPrompt for MacPasswordPrompt {
    fn request_password(
        &self,
        input: &Path,
        previous_attempt_failed: bool,
    ) -> Option<PasswordResponse> {
        let mtm = MainThreadMarker::new().expect("password prompt must run on the main thread");
        let alert = NSAlert::new(mtm);
        alert.setMessageText(ns_string!("Password required"));
        let filename = input
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| input.as_os_str().to_string_lossy());
        let information = if previous_attempt_failed {
            format!("The password for {filename} was incorrect. Try again.")
        } else {
            format!("Enter the password for {filename}.")
        };
        alert.setInformativeText(&NSString::from_str(&information));
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

pub fn run() -> Result<(), Box<dyn Error>> {
    let paths = PlatformPaths::discover()?;
    initialize_logging(&paths.log_file)?;
    let executable = std::env::current_exe()?;
    let seven_zip = executable.with_file_name("7zz");
    let workflow = ExtractionWorkflow::with_password_support(
        seven_zip,
        paths.password_database,
        MacPasswordPrompt,
    );
    let desktop_application = DesktopApplication::new(workflow);

    let mtm = MainThreadMarker::new().ok_or("ezz must start on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let delegate = AppDelegate::new(mtm, desktop_application);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
    Ok(())
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
    alert.setMessageText(ns_string!("ezz could not start"));
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
