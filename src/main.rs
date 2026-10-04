// debug 构建保留控制台（方便看子进程输出与调试）；release 构建是 GUI 子系统，不弹控制台。
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod platform;

fn main() {
    match platform::run() {
        Ok(platform::RunOutcome::Succeeded) => {}
        Ok(platform::RunOutcome::Failed) => std::process::exit(1),
        Err(error) => {
            log::error!("ezz could not start: {error}");
            platform::show_fatal_error(&error.to_string());
            // 启动失败意味着什么都没做成，脚本必须看得出来。
            std::process::exit(1);
        }
    }
}
