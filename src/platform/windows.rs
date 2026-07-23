use std::collections::VecDeque;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use ezz::{DesktopApplication, ExtractionWorkflow, PasswordPrompt, PasswordResponse};
use interprocess::local_socket::{
    GenericNamespaced, Listener, ListenerOptions, Stream, prelude::*,
};
use log::{info, warn};
use native_windows_derive::NwgUi;
use native_windows_gui as nwg;
use nwg::NativeUi;
use serde::{Deserialize, Serialize};

use super::common::{PlatformPaths, finish_batch, initialize_logging, notify_started};

const INSTANCE_NAME: &str = "io.github.yangmoooo.ezz.v3";
const IDLE_TIMEOUT: Duration = Duration::from_millis(750);
const ICON_DATA: &[u8] = include_bytes!("../../assets/icon/ezz.ico");

#[derive(Debug, Serialize, Deserialize)]
enum InstanceMessage {
    OpenFiles(Vec<String>),
    PickFiles,
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

pub fn run() -> Result<(), Box<dyn Error>> {
    let paths = PlatformPaths::discover()?;
    initialize_logging(&paths.log_file)?;
    nwg::init()?;
    nwg::Font::set_global_family("Segoe UI")?;

    let initial_paths: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let instance_name = INSTANCE_NAME.to_ns_name::<GenericNamespaced>()?;
    let listener = match ListenerOptions::new()
        .name(instance_name.clone())
        .create_sync()
    {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            forward_to_primary(instance_name, message_for_paths(initial_paths))?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    let receiver = start_instance_listener(listener);
    let executable = std::env::current_exe()?;
    let workflow = ExtractionWorkflow::with_password_support(
        executable.with_file_name("7zz.exe"),
        paths.password_database,
        WindowsPasswordPrompt,
    );
    let application = DesktopApplication::new(workflow);
    let mut pending: VecDeque<_> = initial_paths.into();
    if pending.is_empty() {
        pending.extend(select_files()?);
    }

    loop {
        if !pending.is_empty() {
            let inputs: Vec<_> = pending.drain(..).collect();
            notify_started(inputs.len());
            finish_batch(&application.process_files(inputs));
        }

        match receiver.recv_timeout(IDLE_TIMEOUT) {
            Ok(InstanceMessage::OpenFiles(paths)) => {
                pending.extend(paths.into_iter().map(PathBuf::from));
            }
            Ok(InstanceMessage::PickFiles) => pending.extend(select_files()?),
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("single-instance listener stopped unexpectedly".into());
            }
        }
    }

    Ok(())
}

pub fn show_fatal_error(message: &str) {
    let _ = nwg::init();
    nwg::error_message("ezz could not start", message);
}

fn message_for_paths(paths: Vec<PathBuf>) -> InstanceMessage {
    if paths.is_empty() {
        InstanceMessage::PickFiles
    } else {
        InstanceMessage::OpenFiles(
            paths
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
        )
    }
}

fn forward_to_primary(
    name: interprocess::local_socket::Name<'_>,
    message: InstanceMessage,
) -> Result<(), Box<dyn Error>> {
    let mut last_error = None;
    for _ in 0..10 {
        match Stream::connect(name.clone()) {
            Ok(mut stream) => {
                serde_json::to_writer(&mut stream, &message)?;
                stream.write_all(b"\n")?;
                stream.flush()?;
                info!("forwarded input to the running ezz instance");
                return Ok(());
            }
            Err(error) => {
                last_error = Some(error);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Err(last_error
        .map(|error| format!("could not contact the running ezz instance: {error}"))
        .unwrap_or_else(|| "could not contact the running ezz instance".to_owned())
        .into())
}

fn start_instance_listener(listener: Listener) -> Receiver<InstanceMessage> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let mut connection = match connection {
                Ok(connection) => BufReader::new(connection),
                Err(error) => {
                    warn!("could not accept forwarded input: {error}");
                    continue;
                }
            };
            let mut line = String::new();
            if let Err(error) = connection.read_line(&mut line) {
                warn!("could not read forwarded input: {error}");
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(message) => {
                    if sender.send(message).is_err() {
                        break;
                    }
                }
                Err(error) => warn!("ignored invalid forwarded input: {error}"),
            }
        }
    });
    receiver
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
