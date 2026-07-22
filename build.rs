fn main() {
    println!("cargo:rerun-if-changed=assets/icon/ezz.ico");
    println!("cargo:rerun-if-changed=assets/hdpi.manifest.xml");

    #[cfg(target_os = "windows")]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/icon/ezz.ico");
        resource.set_manifest_file("assets/hdpi.manifest.xml");
        resource.compile().expect("compile Windows resources");
    }
}
