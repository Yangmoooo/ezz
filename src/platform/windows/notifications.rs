//! 完成与跳过的通知：WinRT toast（设计 §3.2、§12）。
//!
//! 直接使用 WinRT 而不经过包装库，是为了自己控制应用标识（`app_id`），而不是让库使用
//! 它的默认身份。

use std::error::Error;

use log::warn;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager, ToastTemplateType};
use windows::core::HSTRING;

/// toast 的应用标识。
///
/// 未注册的 AUMID 会被 Windows **原样当作发送者名字显示**（这就是之前显示成
/// `io.github.yangmoooo.ezz` 的原因），所以这个字符串直接决定用户看到什么。
/// `CreateToastNotifierWithId` 要求 ≤ 128 字符且不含空格。
/// 要显示成完全不同的名字只能注册 AUMID（开始菜单快捷方式），便携版不做。
const APP_ID: &str = "Ezz";

pub(crate) fn show_notification(summary: &str, body: &str) {
    if let Err(error) = send(summary, body) {
        warn!("could not show desktop notification: {error}");
    }
}

fn send(summary: &str, body: &str) -> Result<(), Box<dyn Error>> {
    // WinRT 调用在 windows crate 里是安全的包装（§12）；只有原始 Win32 才需要 unsafe。
    // 用系统模板而不是拼 XML：文案里的 `&`、`<` 之类字符不需要自己转义。
    let document = ToastNotificationManager::GetTemplateContent(ToastTemplateType::ToastText02)?;
    let texts = document.GetElementsByTagName(&HSTRING::from("text"))?;
    for (index, value) in [summary, body].into_iter().enumerate() {
        let node = texts.Item(index as u32)?;
        let text = document.CreateTextNode(&HSTRING::from(value))?;
        let _ = node.AppendChild(&text)?;
    }

    let notification = ToastNotification::CreateToastNotification(&document)?;
    let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
    {
        Ok(notifier) => notifier,
        Err(error) => {
            // 未注册的 AUMID 通常需要一个带 `System.AppUserModel.ID` 的快捷方式，便携版
            // 没有。退回到"无标识"路径：由系统按可执行文件推导身份（待真机确认显示出来
            // 的名字，见 9.3）。
            warn!(
                "could not use toast app id {APP_ID} ({error}); \
                     falling back to the executable identity"
            );
            ToastNotificationManager::CreateToastNotifier()?
        }
    };
    notifier.Show(&notification)?;

    Ok(())
}
