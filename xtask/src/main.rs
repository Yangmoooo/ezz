use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[cfg(target_os = "macos")]
use plist::{Dictionary, Value};
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use xz2::read::XzDecoder;
#[cfg(target_os = "windows")]
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const SEVEN_ZIP_VERSION: &str = "26.02";

#[cfg(target_os = "macos")]
const ASSET: Asset = Asset {
    archive_name: "7zz-macos-universal.tar.xz",
    binary_name: "7zz",
    sha256: "39dce4d0048bad25df79c1500b3c72357cefa6bb7a5a9d872607a5b6eac6c93d",
    kind: ArchiveKind::TarXz,
};

#[cfg(target_os = "windows")]
const ASSET: Asset = Asset {
    archive_name: "7zz-windows-x64.zip",
    binary_name: "7zz.exe",
    sha256: "d02c5823652d15714c1552a9a18bc830d743401d9ad504b1f6f83938b19f4d3c",
    kind: ArchiveKind::Zip,
};

#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
)))]
compile_error!("ezz xtask only supports Windows and macOS");

#[derive(Clone, Copy)]
struct Asset {
    archive_name: &'static str,
    binary_name: &'static str,
    sha256: &'static str,
    kind: ArchiveKind,
}

#[derive(Clone, Copy)]
enum ArchiveKind {
    #[cfg(target_os = "macos")]
    TarXz,
    #[cfg(target_os = "windows")]
    Zip,
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
            println!(
                "Prepared 7-Zip {} at {}",
                SEVEN_ZIP_VERSION,
                binary.display()
            );
            Ok(())
        }
        Some(command) if command == OsStr::new("package") => {
            let artifact = package()?;
            println!("Packaged ezz at {}", artifact.display());
            Ok(())
        }
        _ => Err("usage: cargo xtask <prepare|package>".into()),
    }
}

fn package() -> Result<PathBuf, Box<dyn Error>> {
    let seven_zip = prepare()?;
    build_release()?;
    let version = package_version()?;

    #[cfg(target_os = "macos")]
    return package_macos(&version, &seven_zip);

    #[cfg(target_os = "windows")]
    return package_windows(&version, &seven_zip);
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

#[cfg(target_os = "macos")]
fn package_macos(version: &str, seven_zip: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let root = workspace_root();
    let dist = root.join("target").join("dist");
    fs::create_dir_all(&dist)?;
    let stage = dist.join(format!("ezz-{version}-macos-arm64"));
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }

    let app = stage.join("ezz.app");
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
    fs::copy(root.join("LICENSE"), licenses.join("ezz-LICENSE.txt"))?;
    for name in ["License.txt", "copying.txt", "man.txt", "unRarLicense.txt"] {
        fs::copy(root.join("assets/7zip").join(name), licenses.join(name))?;
    }
    write_macos_plist(&contents.join("Info.plist"), version)?;

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
        "sign ezz.app",
    )?;
    run_command(
        Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&app),
        "verify ezz.app signature",
    )?;

    let archive = dist.join(format!("ezz-{version}-macos-arm64.zip"));
    if archive.exists() {
        fs::remove_file(&archive)?;
    }
    run_command(
        Command::new("ditto")
            .args([
                "-c",
                "-k",
                "--keepParent",
                "--norsrc",
                "--noextattr",
                "--noqtn",
                "--noacl",
            ])
            .arg(&app)
            .arg(&archive),
        "create macOS release ZIP",
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
    plist.insert("CFBundleDisplayName".into(), Value::String("ezz".into()));
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
    plist.insert("CFBundleName".into(), Value::String("ezz".into()));
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
fn package_windows(version: &str, seven_zip: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let root = workspace_root();
    let dist = root.join("target").join("dist");
    fs::create_dir_all(&dist)?;
    let folder_name = format!("ezz-{version}-windows-x64");
    let stage = dist.join(&folder_name);
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
    fs::copy(root.join("LICENSE"), licenses.join("ezz-LICENSE.txt"))?;
    for name in ["License.txt", "copying.txt", "man.txt", "unRarLicense.txt"] {
        fs::copy(root.join("assets/7zip").join(name), licenses.join(name))?;
    }

    let archive = dist.join(format!("{folder_name}.zip"));
    if archive.exists() {
        fs::remove_file(&archive)?;
    }
    let mut writer = ZipWriter::new(File::create(&archive)?);
    append_directory_to_zip(&mut writer, &stage, &folder_name)?;
    writer.finish()?;
    Ok(archive)
}

#[cfg(target_os = "windows")]
fn append_directory_to_zip(
    writer: &mut ZipWriter<File>,
    directory: &Path,
    archive_directory: &str,
) -> Result<(), Box<dyn Error>> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    writer.add_directory(format!("{archive_directory}/"), options)?;

    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let name = format!(
            "{archive_directory}/{}",
            entry.file_name().to_string_lossy()
        );
        if path.is_dir() {
            append_directory_to_zip(writer, &path, &name)?;
        } else {
            writer.start_file(name, options)?;
            io::copy(&mut File::open(path)?, writer)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_command(command: &mut Command, operation: &str) -> Result<(), Box<dyn Error>> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("could not {operation}: {status}").into())
    }
}

fn prepare() -> Result<PathBuf, Box<dyn Error>> {
    let cache_dir = workspace_root()
        .join("target")
        .join("ezz-tools")
        .join(SEVEN_ZIP_VERSION);
    fs::create_dir_all(&cache_dir)?;

    let archive_path = cache_dir.join(ASSET.archive_name);
    if !archive_path.is_file() || sha256(&archive_path)? != ASSET.sha256 {
        download(&asset_url(), &archive_path)?;
    }

    let actual_sha256 = sha256(&archive_path)?;
    if actual_sha256 != ASSET.sha256 {
        return Err(format!(
            "checksum mismatch for {}: expected {}, got {}",
            archive_path.display(),
            ASSET.sha256,
            actual_sha256
        )
        .into());
    }

    let binary_path = cache_dir.join(ASSET.binary_name);
    match ASSET.kind {
        #[cfg(target_os = "macos")]
        ArchiveKind::TarXz => extract_tar_xz(&archive_path, &binary_path)?,
        #[cfg(target_os = "windows")]
        ArchiveKind::Zip => extract_zip(&archive_path, &binary_path)?,
    }
    set_executable(&binary_path)?;

    Ok(binary_path)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be inside the workspace")
        .to_path_buf()
}

fn asset_url() -> String {
    format!(
        "https://github.com/Yangmoooo/7zz-bin/releases/download/{SEVEN_ZIP_VERSION}/{}",
        ASSET.archive_name
    )
}

fn download(url: &str, destination: &Path) -> Result<(), Box<dyn Error>> {
    let partial = destination.with_extension("download");
    let client = reqwest::blocking::Client::builder()
        .user_agent("ezz-xtask")
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(300))
        .build()?;
    let mut response = client.get(url).send()?.error_for_status()?;
    let mut file = File::create(&partial)?;
    io::copy(&mut response, &mut file)?;
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

    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(target_os = "macos")]
fn extract_tar_xz(archive_path: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    let decoder = XzDecoder::new(File::open(archive_path)?);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.file_name() == Some(OsStr::new(ASSET.binary_name)) {
            let mut output = File::create(destination)?;
            io::copy(&mut entry, &mut output)?;
            return Ok(());
        }
    }

    Err(format!(
        "{} is missing from {}",
        ASSET.binary_name,
        archive_path.display()
    )
    .into())
}

#[cfg(target_os = "windows")]
fn extract_zip(archive_path: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    let mut archive = ZipArchive::new(File::open(archive_path)?)?;
    let mut entry = archive.by_name(ASSET.binary_name)?;
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
