//! 通知：`UNUserNotificationCenter`。首次使用需要用户授权。

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

/// 启动时请求通知授权；系统只在第一次弹授权框，之后幂等。被拒绝只记录。
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
    // 与 Windows 侧一致：用系统默认提示音。
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

/// `UNUserNotificationCenter` 只在有 bundle 标识符的进程里可用；没有时跳过并记录。
fn center() -> Option<Retained<UNUserNotificationCenter>> {
    // 没有 bundle 标识符就直接放弃：`?` 会把 `None` 传出去。
    let _ = NSBundle::mainBundle().bundleIdentifier()?;
    Some(UNUserNotificationCenter::currentNotificationCenter())
}
