#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    let binary_name = if cfg!(target_os = "windows") {
        "7zz.exe"
    } else {
        "7zz"
    };
    let seven_zip = std::env::current_exe()
        .expect("resolve ezz executable path")
        .with_file_name(binary_name);
    let workflow = ezz::ExtractionWorkflow::new(seven_zip);
    let _application = ezz::DesktopApplication::new(workflow);
}
