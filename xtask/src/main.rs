use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use xz2::read::XzDecoder;
#[cfg(target_os = "windows")]
use zip::ZipArchive;

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

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
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
        _ => Err("usage: cargo xtask prepare".into()),
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
