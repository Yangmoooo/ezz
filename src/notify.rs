pub enum Msg {
    Info,
    Ok,
    Err,
}

#[macro_export]
macro_rules! notify {
    ($ty:expr, $($arg:tt)*) => {
        {
            use notify_rust::{Notification, Timeout};
            let version = env!("CARGO_PKG_VERSION");
            let summary = match $ty {
                Msg::Info => format!("🧐 ezz v{version}"),
                Msg::Ok => format!("🥳 ezz v{version}"),
                Msg::Err => format!("🤬 ezz v{version}"),
            };
            let msg = format!($($arg)*);
            let _ = Notification::new()
                .summary(&summary)
                .body(&msg)
                .timeout(Timeout::Milliseconds(3000))
                .show();
        }
    };
}
