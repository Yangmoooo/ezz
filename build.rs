fn main() {
    println!("cargo:rerun-if-changed=assets/ezz.rc");
    println!("cargo:rerun-if-changed=assets/icon/ezz.ico");
    println!("cargo:rerun-if-changed=assets/hdpi.manifest.xml");

    #[cfg(target_os = "windows")]
    {
        // 版本只有一个来源（Cargo.toml），四段数字与字符串都由它派生（设计 §11.2）。
        let version = env!("CARGO_PKG_VERSION");
        let mut parts: Vec<u32> = version
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect();
        parts.resize(4, 0);
        let comma_separated = parts
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let macros = [
            format!("EZZ_VERSION=\"{version}\""),
            format!("EZZ_VERSION_COMMA={comma_separated}"),
        ];

        // 图标、清单、版本信息与密码对话框都在同一个 .rc 里。清单是必需的（DPI 与视觉样式），
        // 编译不出来就让构建失败，而不是发布一个没有清单的 exe。
        embed_resource::compile("assets/ezz.rc", macros)
            .manifest_required()
            .unwrap_or_else(|result| panic!("could not embed Windows resources: {result}"));
    }
}
