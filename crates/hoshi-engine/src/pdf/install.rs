//! The PDF library, fetched once into hoshi's home the first time a PDF opens: a build pinned
//! by version and checksum, so a changed download is refused.

use super::PdfFailure;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const VERSION: &str = "8125";
const RELEASES: &str = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium";
const MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;

/// A build for one platform: its archive, the archive's SHA-256, and where the library is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Build {
    pub archive: &'static str,
    pub sha256: &'static str,
    pub entry: &'static str,
}

pub(super) fn build_for(os: &str, arch: &str) -> Option<Build> {
    let (archive, sha256) = match (os, arch) {
        ("windows", "x86_64") => (
            "pdfium-win-x64.tgz",
            "9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b",
        ),
        ("windows", "aarch64") => (
            "pdfium-win-arm64.tgz",
            "d394ba61991fb53f4d87f70988035f337d9e73f2bd2c3c1b966df8c43fcabb05",
        ),
        ("linux", "x86_64") => (
            "pdfium-linux-x64.tgz",
            "e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659",
        ),
        ("linux", "aarch64") => (
            "pdfium-linux-arm64.tgz",
            "8b8818f2ec57d1b213d952ca6d7f0ffeca9d0110487d0e20ad83ba918bc92efa",
        ),
        ("macos", "x86_64") => (
            "pdfium-mac-x64.tgz",
            "8a5193535b80c7215d8932f6647ae5097465b6dba2811602c21e120c2af112b0",
        ),
        ("macos", "aarch64") => (
            "pdfium-mac-arm64.tgz",
            "4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d",
        ),
        _ => return None,
    };
    let entry = match os {
        "windows" => "bin/pdfium.dll",
        "macos" => "lib/libpdfium.dylib",
        _ => "lib/libpdfium.so",
    };
    Some(Build {
        archive,
        sha256,
        entry,
    })
}

fn file_name(entry: &str) -> &str {
    entry.rsplit('/').next().unwrap_or(entry)
}

fn this_build() -> Option<Build> {
    build_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub(super) fn needs_download(dir: &Path) -> bool {
    this_build().is_some_and(|build| !dir.join(VERSION).join(file_name(build.entry)).is_file())
}

/// The library under `dir`, fetched first when it isn't there yet. Reaches the network.
pub(super) fn library(dir: &Path) -> Result<PathBuf, PdfFailure> {
    let build = this_build().ok_or(PdfFailure::Unsupported)?;
    let folder = dir.join(VERSION);
    let library = folder.join(file_name(build.entry));
    if library.is_file() {
        return Ok(library);
    }
    let archive = download(&build)?;
    install(&archive, &build, &folder).map_err(|_| PdfFailure::Download)?;
    Ok(library)
}

fn download(build: &Build) -> Result<Vec<u8>, PdfFailure> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into();
    let url = format!("{RELEASES}/{VERSION}/{}", build.archive);
    let mut response = agent.get(&url).call().map_err(|_| PdfFailure::Download)?;
    let mut archive = Vec::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_BYTES)
        .reader()
        .read_to_end(&mut archive)
        .map_err(|_| PdfFailure::Download)?;
    verify(&archive, build)?;
    Ok(archive)
}

pub(super) fn verify(archive: &[u8], build: &Build) -> Result<(), PdfFailure> {
    let digest: String = Sha256::digest(archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest == build.sha256 {
        Ok(())
    } else {
        Err(PdfFailure::Download)
    }
}

/// Writes the archive's library into `folder`, whole or not at all.
pub(super) fn install(archive: &[u8], build: &Build, folder: &Path) -> io::Result<()> {
    let mut entries = tar::Archive::new(GzDecoder::new(archive));
    for entry in entries.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().replace('\\', "/");
        if path.trim_start_matches("./") != build.entry {
            continue;
        }
        fs::create_dir_all(folder)?;
        let name = file_name(build.entry);
        let staging = folder.join(format!(".{name}-{}", std::process::id()));
        let mut file = fs::File::create(&staging)?;
        io::copy(&mut entry, &mut file)?;
        file.flush()?;
        drop(file);
        return fs::rename(&staging, folder.join(name)).inspect_err(|_| {
            let _ = fs::remove_file(&staging);
        });
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} isn't in the archive", build.entry),
    ))
}
