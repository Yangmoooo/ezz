use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "macos")]
use plist::{Dictionary, Value};
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use xz2::read::XzDecoder;
#[cfg(target_os = "windows")]
use zip::ZipArchive;

#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
)))]
compile_error!("ezz xtask only supports Windows and macOS");

/// `assets/7zz-bin.toml` 里属于当前平台的表名。
#[cfg(target_os = "windows")]
const PLATFORM_KEY: &str = "windows-x64";
#[cfg(target_os = "macos")]
const PLATFORM_KEY: &str = "macos-arm64";

/// 发布引擎：版本、当前平台的资产名与校验和。唯一来源是 `assets/7zz-bin.toml`。
struct EngineAsset {
    version: String,
    archive_name: String,
    binary_name: String,
    sha256: String,
    kind: ArchiveKind,
}

#[derive(Clone, Copy)]
enum ArchiveKind {
    #[cfg(target_os = "macos")]
    TarXz,
    #[cfg(target_os = "windows")]
    Zip,
}

impl ArchiveKind {
    fn from_name(archive: &str) -> Result<Self, Box<dyn Error>> {
        #[cfg(target_os = "macos")]
        if archive.ends_with(".tar.xz") {
            return Ok(Self::TarXz);
        }
        #[cfg(target_os = "windows")]
        if archive.ends_with(".zip") {
            return Ok(Self::Zip);
        }
        Err(format!("unsupported engine archive name: {archive}").into())
    }
}

fn load_engine_asset() -> Result<EngineAsset, Box<dyn Error>> {
    let path = engine_manifest_path();
    let document: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
    let table = document
        .get(PLATFORM_KEY)
        .ok_or_else(|| format!("{} has no [{PLATFORM_KEY}] table", path.display()))?;
    let archive_name = string_field(table, "archive")?;
    Ok(EngineAsset {
        kind: ArchiveKind::from_name(&archive_name)?,
        archive_name,
        binary_name: string_field(table, "binary")?,
        sha256: string_field(table, "sha256")?,
        version: string_field(&document, "version")?,
    })
}

fn string_field(value: &toml::Value, key: &str) -> Result<String, Box<dyn Error>> {
    value
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string field {key} in assets/7zz-bin.toml").into())
}

fn engine_manifest_path() -> PathBuf {
    workspace_root().join("assets").join("7zz-bin.toml")
}

/// 当前平台的稳定引擎路径（不带版本号）：测试与打包都从这里取。
fn engine_path() -> PathBuf {
    let name = if cfg!(target_os = "windows") {
        "7zz.exe"
    } else {
        "7zz"
    };
    workspace_root().join("target").join("ezz-tools").join(name)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("xtask failed: {error}");
        let mut source = error.source();
        while let Some(cause) = source {
            eprintln!("  caused by: {cause}");
            source = cause.source();
        }
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match env::args_os().nth(1).as_deref() {
        Some(command) if command == OsStr::new("prepare") => {
            let binary = prepare()?;
            println!("Prepared 7-Zip at {}", binary.display());
            Ok(())
        }
        Some(command) if command == OsStr::new("package") => {
            let artifact = package()?;
            println!("Packaged Ezz at {}", artifact.display());
            Ok(())
        }
        Some(command) if command == OsStr::new("update-7zz") => {
            let version = env::args_os()
                .nth(2)
                .ok_or("usage: cargo xtask update-7zz <version>")?;
            update_seven_zip(&version.to_string_lossy())
        }
        _ => Err("usage: cargo xtask <prepare|package|update-7zz>".into()),
    }
}

fn package() -> Result<PathBuf, Box<dyn Error>> {
    let seven_zip = prepare()?;
    build_release()?;

    #[cfg(target_os = "macos")]
    return package_macos(&seven_zip);

    #[cfg(target_os = "windows")]
    return package_windows(&seven_zip);
}

fn build_release() -> Result<(), Box<dyn Error>> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(workspace_root())
        .args(["build", "--release", "--package", "ezz"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("release build failed with {status}").into())
    }
}

/// 版本号来自根 `Cargo.toml` 的 `package.version`。
#[cfg(target_os = "macos")]
fn package_version() -> Result<String, Box<dyn Error>> {
    let manifest = fs::read_to_string(workspace_root().join("Cargo.toml"))?;
    let manifest: toml::Value = toml::from_str(&manifest)?;
    manifest
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "root Cargo.toml is missing package.version".into())
}

/// 仓库里的文档与许可证原样拷进发布物：文档进 `stage`，许可证进 `licenses`。
fn copy_docs_and_licenses(
    root: &Path,
    stage: &Path,
    licenses: &Path,
) -> Result<(), Box<dyn Error>> {
    fs::copy(root.join("README.md"), stage.join("README.md"))?;
    fs::copy(root.join("CHANGELOG.md"), stage.join("CHANGELOG.md"))?;
    fs::copy(root.join("LICENSE"), licenses.join("ezz-LICENSE.txt"))?;
    for name in ["License.txt", "copying.txt", "man.txt", "unRarLicense.txt"] {
        fs::copy(root.join("assets/7zip").join(name), licenses.join(name))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn package_macos(seven_zip: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let root = workspace_root();
    let dist = root.join("target").join("dist");
    fs::create_dir_all(&dist)?;

    // 卷标是用户在 Finder 里看到的名字；发布物文件名保持小写。
    let volume_name = "Ezz";
    let stage = dist.join("stage-macos");
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }

    let app = stage.join("Ezz.app");
    let contents = app.join("Contents");
    let binaries = contents.join("MacOS");
    let resources = contents.join("Resources");
    let licenses = resources.join("licenses");
    fs::create_dir_all(&binaries)?;
    fs::create_dir_all(&licenses)?;

    fs::copy(
        root.join("target").join("release").join("ezz"),
        binaries.join("ezz"),
    )?;
    run_command(
        Command::new("lipo")
            .arg(seven_zip)
            .args(["-thin", "arm64", "-output"])
            .arg(binaries.join("7zz")),
        "prepare arm64 7zz",
    )?;
    set_executable(&binaries.join("ezz"))?;
    set_executable(&binaries.join("7zz"))?;
    fs::copy(
        root.join("assets/icon/ezz.icns"),
        resources.join("ezz.icns"),
    )?;
    copy_docs_and_licenses(&root, &stage, &licenses)?;
    write_macos_plist(&contents.join("Info.plist"), &package_version()?)?;

    // 先签嵌套的 7zz，再签应用包。
    run_command(
        Command::new("codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(binaries.join("7zz")),
        "sign bundled 7zz",
    )?;
    run_command(
        Command::new("codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(&app),
        "sign Ezz.app",
    )?;
    run_command(
        Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&app),
        "verify Ezz.app signature",
    )?;

    // DMG 根目录再放一个指向 /Applications 的符号链接。
    std::os::unix::fs::symlink("/Applications", stage.join("Applications"))?;

    let archive = dist.join("ezz-macos-arm64.dmg");
    if archive.exists() {
        fs::remove_file(&archive)?;
    }
    run_command(
        Command::new("hdiutil")
            .args(["create", "-volname", volume_name, "-srcfolder"])
            .arg(&stage)
            .args(["-ov", "-format", "UDZO"])
            .arg(&archive),
        "create macOS release DMG",
    )?;
    Ok(archive)
}

#[cfg(target_os = "macos")]
fn write_macos_plist(path: &Path, version: &str) -> Result<(), Box<dyn Error>> {
    let extensions = [
        "7z", "zip", "rar", "tar", "gz", "tgz", "bz2", "tbz", "tbz2", "xz", "txz", "zst", "tzst",
        "lz", "lzma", "cab", "arj", "lzh", "cpio", "001", "z01", "mp4", "mkv",
    ];
    let mut document_type = Dictionary::new();
    document_type.insert(
        "CFBundleTypeExtensions".into(),
        Value::Array(
            extensions
                .into_iter()
                .map(|extension| Value::String(extension.to_owned()))
                .collect(),
        ),
    );
    document_type.insert(
        "CFBundleTypeName".into(),
        Value::String("Archives and Steganographier videos".into()),
    );
    document_type.insert("CFBundleTypeRole".into(), Value::String("Viewer".into()));
    document_type.insert("LSHandlerRank".into(), Value::String("Alternate".into()));

    let mut plist = Dictionary::new();
    plist.insert(
        "CFBundleDevelopmentRegion".into(),
        Value::String("en".into()),
    );
    plist.insert("CFBundleDisplayName".into(), Value::String("Ezz".into()));
    plist.insert(
        "CFBundleDocumentTypes".into(),
        Value::Array(vec![Value::Dictionary(document_type)]),
    );
    plist.insert("CFBundleExecutable".into(), Value::String("ezz".into()));
    plist.insert("CFBundleIconFile".into(), Value::String("ezz.icns".into()));
    plist.insert(
        "CFBundleIdentifier".into(),
        Value::String("io.github.yangmoooo.ezz".into()),
    );
    plist.insert(
        "CFBundleInfoDictionaryVersion".into(),
        Value::String("6.0".into()),
    );
    plist.insert("CFBundleName".into(), Value::String("Ezz".into()));
    plist.insert("CFBundlePackageType".into(), Value::String("APPL".into()));
    plist.insert(
        "CFBundleShortVersionString".into(),
        Value::String(version.into()),
    );
    plist.insert("CFBundleVersion".into(), Value::String(version.into()));
    plist.insert(
        "LSMinimumSystemVersion".into(),
        Value::String("11.0".into()),
    );
    plist.insert("LSUIElement".into(), Value::Boolean(true));
    plist.insert("NSHighResolutionCapable".into(), Value::Boolean(true));
    plist::to_file_xml(path, &Value::Dictionary(plist))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn package_windows(seven_zip: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let root = workspace_root();
    let dist = root.join("target").join("dist");
    fs::create_dir_all(&dist)?;
    // 包内目录名与发布物文件名都不含版本号。
    let folder_name = "ezz-windows-x64";
    let stage = dist.join(folder_name);
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }

    let licenses = stage.join("licenses");
    fs::create_dir_all(&licenses)?;
    fs::copy(
        root.join("target").join("release").join("ezz.exe"),
        stage.join("ezz.exe"),
    )?;
    fs::copy(seven_zip, stage.join("7zz.exe"))?;
    copy_docs_and_licenses(&root, &stage, &licenses)?;

    let archive = dist.join(format!("{folder_name}.zip"));
    if archive.exists() {
        fs::remove_file(&archive)?;
    }
    // 用随发布物一起交付的同一个引擎打包：相对路径、真实时间戳（`zip` crate 只能写 1980）。
    run_command(
        Command::new(seven_zip)
            .current_dir(&dist)
            .args(["a", "-tzip", "-mx=9"])
            .arg(&archive)
            .arg(folder_name),
        "create Windows release ZIP",
    )?;
    Ok(archive)
}

fn run_command(command: &mut Command, operation: &str) -> Result<(), Box<dyn Error>> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("could not {operation}: {status}").into())
    }
}

/// 下载并校验引擎，解到缓存目录；返回的稳定路径不包含版本号。
fn prepare() -> Result<PathBuf, Box<dyn Error>> {
    let asset = load_engine_asset()?;
    let cache_dir = workspace_root()
        .join("target")
        .join("ezz-tools")
        .join(&asset.version);
    fs::create_dir_all(&cache_dir)?;

    let archive_path = cache_dir.join(&asset.archive_name);
    if !archive_path.is_file() || sha256(&archive_path)? != asset.sha256 {
        download(&asset_url(&asset), &archive_path)?;
    }

    let actual_sha256 = sha256(&archive_path)?;
    if actual_sha256 != asset.sha256 {
        return Err(format!(
            "checksum mismatch for {}: expected {}, got {}",
            archive_path.display(),
            asset.sha256,
            actual_sha256
        )
        .into());
    }

    let extracted = cache_dir.join(&asset.binary_name);
    match asset.kind {
        #[cfg(target_os = "macos")]
        ArchiveKind::TarXz => extract_tar_xz(&archive_path, &extracted, &asset.binary_name)?,
        #[cfg(target_os = "windows")]
        ArchiveKind::Zip => extract_zip(&archive_path, &extracted, &asset.binary_name)?,
    }
    set_executable(&extracted)?;

    let stable = engine_path();
    fs::copy(&extracted, &stable)?;
    set_executable(&stable)?;
    Ok(stable)
}

/// 把 `assets/7zz-bin.toml` 升到给定版本：下载两个平台的资产、重算校验和、回写文件。
fn update_seven_zip(version: &str) -> Result<(), Box<dyn Error>> {
    let path = engine_manifest_path();
    let document: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
    let cache_dir = workspace_root()
        .join("target")
        .join("ezz-tools")
        .join(version);
    fs::create_dir_all(&cache_dir)?;

    let mut updated = Vec::new();
    for key in ["windows-x64", "macos-arm64"] {
        let table = document
            .get(key)
            .ok_or_else(|| format!("{} has no [{key}] table", path.display()))?;
        let archive_name = string_field(table, "archive")?;
        let binary_name = string_field(table, "binary")?;
        let url = format!(
            "https://github.com/Yangmoooo/7zz-bin/releases/download/{version}/{archive_name}"
        );
        let archive = cache_dir.join(&archive_name);
        download(&url, &archive)?;
        let digest = sha256(&archive)?;
        println!("{key}: {archive_name}\n  sha256 = {digest}");
        updated.push((key, archive_name, binary_name, digest));
    }

    // 手写而不是 toml 序列化：文件头的说明要留着。
    let mut text = String::from(
        "# 发布所用的 7-Zip 引擎：唯一来源。\n\
         #\n\
         # 升级用 `cargo xtask update-7zz <版本>`（或 `just 7zz-update <版本>`）：\n\
         # 它下载下面两个平台的资产、重新计算 sha256 并回写本文件。\n",
    );
    text.push_str(&format!("\nversion = \"{version}\"\n"));
    for (key, archive_name, binary_name, digest) in updated {
        text.push_str(&format!(
            "\n[{key}]\narchive = \"{archive_name}\"\nbinary = \"{binary_name}\"\nsha256 = \"{digest}\"\n"
        ));
    }
    fs::write(&path, text)?;
    println!("updated {}", path.display());

    // 立刻按新条目准备一次：既填好缓存，也确认钉进去的哈希确实对得上。
    let engine = prepare()?;
    check_engine_version(&engine, version)?;
    println!("prepared {}", engine.display());
    Ok(())
}

/// 跑一次引擎，确认它自报的版本就是刚钉下的版本。
fn check_engine_version(engine: &Path, expected: &str) -> Result<(), Box<dyn Error>> {
    let output = Command::new(engine).output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !text.contains(expected) {
        return Err(format!(
            "{} does not report version {expected}; it printed: {}",
            engine.display(),
            text.lines().next().unwrap_or("").trim()
        )
        .into());
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be inside the workspace")
        .to_path_buf()
}

fn asset_url(asset: &EngineAsset) -> String {
    format!(
        "https://github.com/Yangmoooo/7zz-bin/releases/download/{}/{}",
        asset.version, asset.archive_name
    )
}

fn download(url: &str, destination: &Path) -> Result<(), Box<dyn Error>> {
    let partial = destination.with_extension("download");
    run_command(
        Command::new("curl")
            .args([
                "-fsSL",
                "--retry",
                "3",
                "--connect-timeout",
                "30",
                "--max-time",
                "300",
            ])
            .arg("-o")
            .arg(&partial)
            .arg(url),
        "download 7-Zip",
    )?;
    fs::rename(partial, destination)?;
    Ok(())
}

fn sha256(path: &Path) -> Result<String, Box<dyn Error>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();
    let mut checksum = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(checksum, "{byte:02x}");
    }
    Ok(checksum)
}

#[cfg(target_os = "macos")]
fn extract_tar_xz(
    archive_path: &Path,
    destination: &Path,
    member: &str,
) -> Result<(), Box<dyn Error>> {
    let decoder = XzDecoder::new(File::open(archive_path)?);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.file_name() == Some(OsStr::new(member)) {
            let mut output = File::create(destination)?;
            io::copy(&mut entry, &mut output)?;
            return Ok(());
        }
    }

    Err(format!("{member} is missing from {}", archive_path.display()).into())
}

#[cfg(target_os = "windows")]
fn extract_zip(
    archive_path: &Path,
    destination: &Path,
    member: &str,
) -> Result<(), Box<dyn Error>> {
    let mut archive = ZipArchive::new(File::open(archive_path)?)?;
    let mut entry = archive.by_name(member)?;
    let mut output = File::create(destination)?;
    io::copy(&mut entry, &mut output)?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
}

#[cfg(windows)]
fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}
