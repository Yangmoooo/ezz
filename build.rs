#[cfg(target_os = "windows")]
extern crate winres;

#[cfg(target_os = "windows")]
fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("./assets/icon/ezz.ico")
        .set_manifest_file("./assets/hdpi.manifest.xml");
    res.compile().unwrap();
}

#[cfg(target_os = "linux")]
fn main() {}
