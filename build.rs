use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Cursor;
use std::io::{Read, Write, copy};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use xz2::read::XzDecoder;
use zip::ZipArchive;

const SEVENZZ_REPO_OWNER: &str = "Yangmoooo";
const SEVENZZ_REPO_NAME: &str = "7zz-bin";

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    ensure_7zz_is_downloaded()?;
    set_windows_resources()?;
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=./assets/icon/ezz.ico");
    println!("cargo:rerun-if-changed=./assets/hdpi.manifest.xml");
    Ok(())
}

fn ensure_7zz_is_downloaded() -> Result<(), Box<dyn Error>> {
    let out_dir = env::var("OUT_DIR")?;
    let out_dir_path = Path::new(&out_dir);

    let (asset_name, binary_name) = if cfg!(target_os = "windows") {
        ("7zz-windows-x64.zip", "7zz.exe")
    } else if cfg!(target_os = "linux") {
        ("7zz-linux-x64.tar.xz", "7zz")
    } else {
        panic!("Unsupported target OS for 7zz download.");
    };

    let binary_path = out_dir_path.join(binary_name);
    let version_path = out_dir_path.join("7zz.version");

    let mut client_builder = reqwest::blocking::Client::builder().user_agent("rust-build-script");

    let token_var = env::var("EZZ_GITHUB_TOKEN").or_else(|_| env::var("GITHUB_TOKEN"));
    if let Ok(token) = token_var {
        println!("cargo:warning=Using GitHub token for API authentication.");
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Authorization",
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))?,
        );
        client_builder = client_builder.default_headers(headers);
    } else {
        println!(
            "cargo:warning=EZZ_GITHUB_TOKEN or GITHUB_TOKEN not set. Making unauthenticated requests (rate limits may apply)."
        );
    }

    let client = client_builder.build()?;
    let api_url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        SEVENZZ_REPO_OWNER, SEVENZZ_REPO_NAME
    );
    let latest_release: GitHubRelease = client.get(&api_url).send()?.json()?;
    let latest_version = latest_release.tag_name;

    if binary_path.exists() && version_path.exists() {
        let mut local_version = String::new();
        File::open(&version_path)?.read_to_string(&mut local_version)?;
        if local_version.trim() == latest_version.trim() {
            println!(
                "cargo:warning=7zz is up to date (version {local_version}). Skipping download."
            );
            return Ok(());
        }
    }

    println!("cargo:warning=New 7zz version {latest_version} available. Downloading...");
    let asset = latest_release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            format!("Could not find asset matching '{asset_name}' in release '{latest_version}'")
        })?;

    let archive_path = out_dir_path.join(asset_name);
    download_file(&asset.browser_download_url, &archive_path)?;
    println!(
        "cargo:warning=Downloaded archive to {}",
        archive_path.display()
    );

    extract_archive(&archive_path, &binary_path)?;
    println!(
        "cargo:warning=Extracted binary to {}",
        binary_path.display()
    );

    fs::remove_file(&archive_path)?;

    let mut version_file = File::create(&version_path)?;
    version_file.write_all(latest_version.as_bytes())?;

    println!("cargo:warning=Setup of {} complete.", binary_path.display());
    Ok(())
}

fn download_file(url: &str, dest_path: &PathBuf) -> Result<(), Box<dyn Error>> {
    let response = reqwest::blocking::get(url)?;
    if !response.status().is_success() {
        return Err(format!("Failed to download file: HTTP {}", response.status()).into());
    }
    let mut dest_file = File::create(dest_path)?;
    let content = response.bytes()?;
    copy(&mut Cursor::new(content), &mut dest_file)?;
    Ok(())
}

fn extract_archive(archive_path: &Path, dest_path: &Path) -> Result<(), Box<dyn Error>> {
    if cfg!(target_os = "windows") {
        let file = File::open(archive_path)?;
        let mut archive = ZipArchive::new(file)?;
        let mut bin_file = archive.by_name("7zz.exe")?;
        let mut outfile = File::create(dest_path)?;
        copy(&mut bin_file, &mut outfile)?;
    } else if cfg!(target_os = "linux") {
        let file = File::open(archive_path)?;
        let xz = XzDecoder::new(file);
        let mut tar_archive = tar::Archive::new(xz);
        let mut found = false;
        for entry in tar_archive.entries()? {
            let mut entry = entry?;
            if entry.path()?.file_name() == Some(OsStr::new("7zzs")) {
                entry.unpack(dest_path)?;
                found = true;
                break;
            }
        }
        if !found {
            return Err(format!("Could not find 7zzs in '{}'", archive_path.display()).into());
        }
    } else {
        panic!("Unsupported target OS for archive extraction.");
    };
    Ok(())
}

#[cfg(target_os = "windows")]
fn set_windows_resources() -> Result<(), Box<dyn Error>> {
    println!("cargo:warning=Setting Windows specific resources...");
    let mut res = winres::WindowsResource::new();
    res.set_icon("./assets/icon/ezz.ico")
        .set_manifest_file("./assets/hdpi.manifest.xml")
        .set("FileDescription", "A very light wrapper around 7-Zip")
        .set("ProductName", "Easy unZip");
    res.compile()?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn set_windows_resources() -> Result<(), Box<dyn Error>> {
    Ok(())
}
