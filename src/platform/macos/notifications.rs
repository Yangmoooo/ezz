//! 通知：`UNUserNotificationCenter`（设计 §12）。
//!
//! 取代 `notify-rust`：它的 macOS 后端是 2018 年就已废弃的 `NSUserNotification`。
//! 代价是首次使用要请求一次系统授权，换来的是权限状态可知（不再是"发了但看不到"）。

use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use log::{info, warn};
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
    UNNotificationSound, UNUserNotificationCenter,
};

/// 启动时请求通知授权（设计 §12）。
///
/// 每次启动都调用：系统只在第一次真正弹授权框，之后是幂等的。被拒绝只记录，不影响解压。
pub(super) fn request_authorization() {
    let Some(center) = center() else {
        return;
    };

    let handler = RcBlock::new(|granted: Bool, error: *mut NSError| {
        // SAFETY: 回调只在 `error` 非空时读取它，与 Objective-C 的约定一致。
        if let Some(error) = unsafe { error.as_ref() } {
            warn!(
                "could not request notification authorization: {}",
                error.localizedDescription()
            );
        } else if granted.as_bool() {
            info!("notification authorization granted");
        } else {
            warn!("notification authorization denied; completion notifications will not appear");
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &handler,
    );
}

pub(crate) fn show_notification(summary: &str, body: &str) {
    if let Err(error) = send(summary, body) {
        warn!("could not show desktop notification: {error}");
    }
}

fn send(summary: &str, body: &str) -> Result<(), Box<dyn Error>> {
    let Some(center) = center() else {
        return Err("notifications are only available inside an application bundle".into());
    };

    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(summary));
    content.setBody(&NSString::from_str(body));
    // 与 Windows 侧一致：用系统默认提示音（Windows 的 toast 自带提示音）。
    content.setSound(Some(&UNNotificationSound::defaultSound()));

    // 标识只用于去重：带上毫秒时间戳，保证每条通知都单独出现。
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    let identifier = NSString::from_str(&format!("ezz-{stamp}"));
    // 没有 trigger 表示立即投递。
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
    center.addNotificationRequest_withCompletionHandler(&request, None);

    Ok(())
}

/// `UNUserNotificationCenter` 只在有 bundle 标识符的进程里可用。
///
/// `cargo run`（没有 `.app`）时直接跳过并记录：通知是完成后的附加信息，不值得让它
/// 把开发期的每一次提取都变成一条警告。
fn center() -> Option<Retained<UNUserNotificationCenter>> {
    // 没有 bundle 标识符就直接放弃：`?` 会把 `None` 传出去。
    let _ = NSBundle::mainBundle().bundleIdentifier()?;
    Some(UNUserNotificationCenter::currentNotificationCenter())
}
