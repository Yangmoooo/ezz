extern crate native_windows_derive as nwd;
extern crate native_windows_gui as nwg;

use nwd::NwgUi;
use nwg::NativeUi;
use std::cell::RefCell;

use crate::types::EzzResult;

const ICON_DATA: &[u8] = include_bytes!("../../../../assets/icon/ezz.ico");

#[derive(Default, NwgUi)]
pub struct PasswordDialog {
    #[nwg_resource(source_bin: Some(ICON_DATA))]
    icon: nwg::Icon,

    #[nwg_control(
        title: "ezz", center: true, size: (240, 120),
        flags: "WINDOW|VISIBLE", icon: Some(&data.icon)
    )]
    #[nwg_events(
        OnInit: [PasswordDialog::init_controls],
        OnKeyEnter: [PasswordDialog::on_ok],
        OnWindowClose: [PasswordDialog::on_close]
    )]
    window: nwg::Window,

    #[nwg_control(text: "输入密码：", position: (20, 15), size: (200, 20))]
    label: nwg::Label,

    // 如需隐藏输入内容，可设置属性 password: Some('*')
    #[nwg_control(position: (20, 38), size: (200, 20), limit: 256, focus: true)]
    pwd_input: nwg::TextInput,

    #[nwg_control(text: "确定", position: (25, 72), size: (85, 30))]
    #[nwg_events( OnButtonClick: [PasswordDialog::on_ok] )]
    ok_btn: nwg::Button,

    #[nwg_control(text: "取消", position: (130, 72), size: (85, 30))]
    #[nwg_events( OnButtonClick: [PasswordDialog::on_cancel] )]
    cancel_btn: nwg::Button,

    password: RefCell<Option<String>>,
}

impl PasswordDialog {
    pub fn ask_password() -> EzzResult<Option<String>> {
        nwg::init()?;
        let mut font = nwg::Font::default();
        nwg::Font::builder()
            .size(12)
            .family("宋体")
            .build(&mut font)?;
        nwg::Font::set_global_default(Some(font));

        let dialog = PasswordDialog::build_ui(Default::default())?;

        nwg::dispatch_thread_events(); // 进入消息循环

        let password = dialog.password.borrow_mut().take();
        Ok(password)
    }

    fn init_controls(&self) {
        self.pwd_input.set_focus();
    }

    fn on_ok(&self) {
        let password = self.pwd_input.text();
        self.password.replace(Some(password));
        nwg::stop_thread_dispatch();
    }

    fn on_cancel(&self) {
        nwg::stop_thread_dispatch();
    }

    fn on_close(&self) {
        nwg::stop_thread_dispatch();
    }
}
