//! Codex from npm, installed into hoshi's home: each release in its own folder under
//! `tools/codex`, the one in use named by `active.json`. Also the adapter that runs the Codex
//! the settings, that install or the `PATH` provide.

use super::login;
use crate::packages;
use crate::process::{hide_console, kill_tree};
use crate::updater::{UpdateFailure, Updater, find_program};
use crate::{
    CommitMessage, CommitMessageRequest, GeneratedTitle, Installation, OutputSink, ProviderAdapter,
    PullRequestText, PullRequestTextRequest, RunningTurn, TitleRequest, TurnRequest,
};
use base64::Engine as _;
use hoshi_core::{
    BankedResetSpend, ProviderAccountStatus, ProviderModels, ProviderUpdateFailure, SlashCommand,
    TurnFailure, UpdateStep, UsageLimits,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha512};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const PACKAGE: &str = "@openai/codex";
const REGISTRY: &str = "https://registry.npmjs.org";
const STANDALONE_LATEST: &str = "https://releases.openai.com/codex/channels/latest";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(45 * 60);
/// A fresh binary can take long to start while the system scans it.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(90);
/// Releases are about 200 MB packed.
const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const STAGING_STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const ACTIVE_FILE: &str = "active.json";
const COMPLETE_FILE: &str = ".install-complete.json";
const MANIFEST_FILE: &str = "codex-package.json";
const STAGING_PREFIX: &str = ".install-";

/// A platform OpenAI publishes Codex for: npm's name for it and the target its binary is built
/// for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Platform {
    npm: &'static str,
    target: &'static str,
}

const PLATFORMS: [(&str, &str, Platform); 6] = [
    platform("linux", "x86_64", "linux-x64", "x86_64-unknown-linux-musl"),
    platform(
        "linux",
        "aarch64",
        "linux-arm64",
        "aarch64-unknown-linux-musl",
    ),
    platform("macos", "x86_64", "darwin-x64", "x86_64-apple-darwin"),
    platform("macos", "aarch64", "darwin-arm64", "aarch64-apple-darwin"),
    platform("windows", "x86_64", "win32-x64", "x86_64-pc-windows-msvc"),
    platform(
        "windows",
        "aarch64",
        "win32-arm64",
        "aarch64-pc-windows-msvc",
    ),
];

const fn platform(
    os: &'static str,
    arch: &'static str,
    npm: &'static str,
    target: &'static str,
) -> (&'static str, &'static str, Platform) {
    (os, arch, Platform { npm, target })
}

fn platform_for(os: &str, arch: &str) -> Option<Platform> {
    PLATFORMS
        .iter()
        .find(|(known_os, known_arch, _)| *known_os == os && *known_arch == arch)
        .map(|(_, _, platform)| *platform)
}

fn current_platform() -> Option<Platform> {
    platform_for(std::env::consts::OS, std::env::consts::ARCH)
}

impl Platform {
    fn executable(&self) -> &'static str {
        if self.target.contains("windows") {
            "codex.exe"
        } else {
            "codex"
        }
    }
}

/// One platform's package of a release, as npm serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Release {
    version: String,
    tarball: String,
    /// The tarball's SHA-512.
    sha512: Vec<u8>,
}

/// `1.2.3`, maybe with a prerelease such as `-alpha.1`, and nothing else: it names a folder.
fn is_version(text: &str) -> bool {
    let (release, prerelease) = match text.split_once('-') {
        Some((release, prerelease)) => (release, Some(prerelease)),
        None => (text, None),
    };
    let parts: Vec<_> = release.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && prerelease.is_none_or(|prerelease| {
            !prerelease.is_empty()
                && prerelease
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
                && !prerelease.contains("..")
        })
}

/// The version `@openai/codex` names latest, from its metadata.
fn version_of(latest: &Value) -> Option<String> {
    let version = latest["version"].as_str()?;
    is_version(version).then(|| version.to_owned())
}

/// The package that holds `platform`'s binary, as the main package's metadata names it: an
/// alias such as `npm:@openai/codex@0.169.0-win32-x64`.
fn platform_package(latest: &Value, platform: &Platform) -> Option<(String, String)> {
    let key = format!("{PACKAGE}-{}", platform.npm);
    let spec = latest["optionalDependencies"][&key].as_str()?;
    let (name, version) = match spec.strip_prefix("npm:") {
        Some(alias) => {
            let (name, version) = alias.rsplit_once('@')?;
            (name.to_owned(), version.to_owned())
        }
        None => (key, spec.to_owned()),
    };
    (name == PACKAGE && !version.is_empty()).then_some((name, version))
}

fn package_url(name: &str, version: &str) -> String {
    format!("{REGISTRY}/{name}/{version}")
}

/// The tarball and checksum of a platform package, from its metadata.
fn release_of(version: &str, package: &Value) -> Option<Release> {
    let tarball = package["dist"]["tarball"].as_str()?;
    if !tarball.starts_with(&format!("{REGISTRY}/")) {
        return None;
    }
    let integrity = package["dist"]["integrity"].as_str()?;
    let sha512 = integrity
        .split_whitespace()
        .find_map(|hash| hash.strip_prefix("sha512-"))
        .and_then(|hash| base64::engine::general_purpose::STANDARD.decode(hash).ok())
        .filter(|hash| hash.len() == 64)?;
    Some(Release {
        version: version.to_owned(),
        tarball: tarball.to_owned(),
        sha512,
    })
}

fn get_json(url: &str) -> Option<Value> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(LOOKUP_TIMEOUT))
        .build()
        .into();
    let text = agent
        .get(url)
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    serde_json::from_str(&text).ok()
}

fn latest_metadata() -> Option<Value> {
    get_json(&format!("{REGISTRY}/{PACKAGE}/latest"))
}

/// The latest Codex release on npm; reaches the network.
pub fn latest_version() -> Option<String> {
    packages::npm_latest(PACKAGE).filter(|version| is_version(version))
}

fn latest_release(platform: &Platform) -> Option<Release> {
    let latest = latest_metadata()?;
    let version = version_of(&latest)?;
    let (name, package_version) = platform_package(&latest, platform)?;
    let package = get_json(&package_url(&name, &package_version))?;
    release_of(&version, &package)
}

fn failure(reason: ProviderUpdateFailure) -> UpdateFailure {
    UpdateFailure {
        reason,
        output: None,
    }
}

fn write_failed(_: io::Error) -> UpdateFailure {
    failure(ProviderUpdateFailure::WriteFailed)
}

/// hoshi's own installs of Codex, under `<hoshi home>/tools/codex`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexInstall {
    dir: PathBuf,
}

impl CodexInstall {
    pub fn new(hoshi_home: &Path) -> Self {
        Self {
            dir: hoshi_home.join("tools").join("codex"),
        }
    }

    /// The npm package it installs.
    pub fn package(&self) -> &'static str {
        PACKAGE
    }

    /// The executable of the release in use; `None` until one was installed.
    pub fn binary(&self) -> Option<PathBuf> {
        let platform = current_platform()?;
        let version = self.active()?;
        let dir = self.dir.join(&version);
        dir.join(COMPLETE_FILE).is_file().then_some(())?;
        let binary = dir.join("bin").join(platform.executable());
        binary.is_file().then_some(binary)
    }

    fn active(&self) -> Option<String> {
        let text = fs::read_to_string(self.dir.join(ACTIVE_FILE)).ok()?;
        let active: Value = serde_json::from_str(&text).ok()?;
        let version = active["version"].as_str()?;
        is_version(version).then(|| version.to_owned())
    }

    /// Installs npm's latest release unless it is the one in use, and switches to it once it
    /// is whole and starts. Returns the version now in use. Reaches the network.
    pub fn install(&self, mut step: impl FnMut(UpdateStep)) -> Result<String, UpdateFailure> {
        let platform =
            current_platform().ok_or_else(|| failure(ProviderUpdateFailure::Unsupported))?;
        step(UpdateStep::Downloading { percent: None });
        let release = latest_release(&platform)
            .ok_or_else(|| failure(ProviderUpdateFailure::DownloadFailed))?;
        self.install_release(&platform, &release, &mut step)?;
        Ok(release.version)
    }

    fn install_release(
        &self,
        platform: &Platform,
        release: &Release,
        step: &mut impl FnMut(UpdateStep),
    ) -> Result<(), UpdateFailure> {
        fs::create_dir_all(&self.dir).map_err(write_failed)?;
        self.sweep_staging();
        let destination = self.dir.join(&release.version);
        if destination.exists() {
            step(UpdateStep::Verifying);
            if completed(&destination, platform, release)
                && verify(&destination, platform, &release.version).is_ok()
            {
                return self.activate(&release.version);
            }
            remove_release(&destination).map_err(write_failed)?;
        }
        let staging = self.dir.join(format!("{STAGING_PREFIX}{}", unique()));
        fs::create_dir(&staging).map_err(write_failed)?;
        let result = self.stage(platform, release, &staging, &destination, step);
        let _ = fs::remove_dir_all(&staging);
        result?;
        self.activate(&release.version)
    }

    fn stage(
        &self,
        platform: &Platform,
        release: &Release,
        staging: &Path,
        destination: &Path,
        step: &mut impl FnMut(UpdateStep),
    ) -> Result<(), UpdateFailure> {
        let archive = staging.join("download.tgz");
        download(release, &archive, step)?;
        step(UpdateStep::Extracting);
        let runtime = staging.join("runtime");
        fs::create_dir(&runtime).map_err(write_failed)?;
        extract(&archive, platform, &runtime)?;
        let _ = fs::remove_file(&archive);
        step(UpdateStep::Verifying);
        verify(&runtime, platform, &release.version)?;
        let record = json!({
            "version": release.version,
            "target": platform.target,
            "sha512": hex(&release.sha512),
        });
        fs::write(runtime.join(COMPLETE_FILE), record.to_string()).map_err(write_failed)?;
        rename_with_retries(&runtime, destination).map_err(write_failed)
    }

    /// Points `active.json` at `version` in one rename, then drops the releases no longer in
    /// use, as far as nothing holds them.
    fn activate(&self, version: &str) -> Result<(), UpdateFailure> {
        let pointer = self.dir.join(format!(".active-{}.json", unique()));
        fs::write(&pointer, json!({ "version": version }).to_string()).map_err(write_failed)?;
        if let Err(error) = rename_with_retries(&pointer, &self.dir.join(ACTIVE_FILE)) {
            let _ = fs::remove_file(&pointer);
            return Err(write_failed(error));
        }
        self.prune(version);
        Ok(())
    }

    fn prune(&self, keep: &str) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == keep || !is_version(&name) {
                continue;
            }
            let _ = remove_release(&entry.path());
        }
    }

    fn sweep_staging(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(STAGING_PREFIX) && !name.starts_with(".active-") {
                continue;
            }
            let stale = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_none_or(|age| age >= STAGING_STALE_AFTER);
            if stale {
                let path = entry.path();
                let _ = fs::remove_dir_all(&path).or_else(|_| fs::remove_file(&path));
            }
        }
    }
}

/// Removes the completion record first, so a release only partly removed is never taken
/// for whole.
fn remove_release(dir: &Path) -> io::Result<()> {
    match fs::remove_file(dir.join(COMPLETE_FILE)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::remove_dir_all(dir)
}

fn completed(dir: &Path, platform: &Platform, release: &Release) -> bool {
    let Ok(text) = fs::read_to_string(dir.join(COMPLETE_FILE)) else {
        return false;
    };
    let Ok(record) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    record["version"] == release.version.as_str()
        && record["target"] == platform.target
        && record["sha512"] == hex(&release.sha512).as_str()
}

fn unique() -> String {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The system can hold a binary that just ran for a moment after it exits.
fn rename_with_retries(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if attempt >= 20 => return Err(error),
            Err(_) => {
                attempt += 1;
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn download(
    release: &Release,
    archive: &Path,
    step: &mut impl FnMut(UpdateStep),
) -> Result<(), UpdateFailure> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .build()
        .into();
    let download_failed = |_| failure(ProviderUpdateFailure::DownloadFailed);
    let mut response = agent
        .get(&release.tarball)
        .call()
        .map_err(download_failed)?;
    let total = response.body().content_length();
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_BYTES)
        .reader();
    let mut file = fs::File::create(archive).map_err(write_failed)?;
    let mut hash = Sha512::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let (mut downloaded, mut shown) = (0u64, None);
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(failure(ProviderUpdateFailure::DownloadFailed)),
        };
        hash.update(&buffer[..read]);
        file.write_all(&buffer[..read]).map_err(write_failed)?;
        downloaded += read as u64;
        let percent = percent(downloaded, total);
        if percent.is_some() && percent != shown {
            shown = percent;
            step(UpdateStep::Downloading { percent });
        }
    }
    file.flush().map_err(write_failed)?;
    if hash.finalize().as_slice() != release.sha512.as_slice() {
        return Err(failure(ProviderUpdateFailure::VerifyFailed));
    }
    Ok(())
}

fn percent(done: u64, total: Option<u64>) -> Option<u8> {
    let total = total.filter(|total| *total > 0)?;
    Some((done.min(total) * 100 / total) as u8)
}

/// Where an entry of the package goes under the release folder: only the platform's vendor
/// folder is kept. `Err` for a path that would leave it.
fn entry_destination(entry: &Path, platform: &Platform) -> Result<Option<PathBuf>, ()> {
    let mut components = entry.components();
    let prefix = ["package", "vendor", platform.target];
    for expected in prefix {
        match components.next() {
            Some(Component::Normal(part)) if part == expected => {}
            Some(Component::Normal(_)) | None => return Ok(None),
            Some(_) => return Err(()),
        }
    }
    let mut relative = PathBuf::new();
    for component in components {
        match component {
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text.contains(':') || text.contains('\\') {
                    return Err(());
                }
                relative.push(part);
            }
            Component::CurDir => {}
            _ => return Err(()),
        }
    }
    Ok((!relative.as_os_str().is_empty()).then_some(relative))
}

fn extract(archive: &Path, platform: &Platform, runtime: &Path) -> Result<(), UpdateFailure> {
    let corrupt = || failure(ProviderUpdateFailure::VerifyFailed);
    let file = fs::File::open(archive).map_err(write_failed)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(io::BufReader::new(file)));
    let entries = tar.entries().map_err(|_| corrupt())?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_ENTRIES {
            return Err(corrupt());
        }
        let mut entry = entry.map_err(|_| corrupt())?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        let path = entry.path().map_err(|_| corrupt())?.into_owned();
        let Some(relative) = entry_destination(&path, platform).map_err(|()| corrupt())? else {
            continue;
        };
        let target = runtime.join(relative);
        if kind.is_dir() {
            fs::create_dir_all(&target).map_err(write_failed)?;
        } else if kind.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(write_failed)?;
            }
            entry.unpack(&target).map_err(write_failed)?;
        } else {
            return Err(corrupt());
        }
    }
    Ok(())
}

/// The release in `dir` is the one expected for `platform`, whole, and its binary starts and
/// says so.
fn verify(dir: &Path, platform: &Platform, version: &str) -> Result<(), UpdateFailure> {
    let corrupt = || failure(ProviderUpdateFailure::VerifyFailed);
    let text = fs::read_to_string(dir.join(MANIFEST_FILE)).map_err(|_| corrupt())?;
    let manifest: Value = serde_json::from_str(&text).map_err(|_| corrupt())?;
    if !manifest_matches(&manifest, platform, version) {
        return Err(corrupt());
    }
    let binary = dir.join("bin").join(platform.executable());
    let whole =
        fs::metadata(&binary).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
    if !whole {
        return Err(corrupt());
    }
    match run_version(&binary, VERIFY_TIMEOUT) {
        Some(reported) if reported == version => Ok(()),
        _ => Err(corrupt()),
    }
}

fn manifest_matches(manifest: &Value, platform: &Platform, version: &str) -> bool {
    manifest["layoutVersion"] == 1
        && manifest["version"] == version
        && manifest["target"] == platform.target
        && manifest["entrypoint"] == format!("bin/{}", platform.executable()).as_str()
}

/// The version `codex --version` reports, as `codex-cli 0.169.0`.
fn parse_cli_version(output: &str) -> Option<String> {
    let version = output
        .trim()
        .lines()
        .next()?
        .trim()
        .strip_prefix("codex-cli ")?;
    is_version(version).then(|| version.to_owned())
}

fn run_version(binary: &Path, timeout: Duration) -> Option<String> {
    let mut command = Command::new(binary);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let pid = child.id();
    let child = Mutex::new(child);
    let reader = thread::spawn(move || {
        let mut text = String::new();
        stdout.read_to_string(&mut text).ok().map(|_| text)
    });
    let deadline = Instant::now() + timeout;
    loop {
        let status = child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_wait();
        match status {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                kill_tree(pid, &child);
                return None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
        }
    }
    parse_cli_version(&reader.join().ok()??)
}

/// Where hoshi finds Codex and where Codex keeps its configuration and sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSetup {
    /// The user's own binary, which then always runs.
    pub binary_path: Option<String>,
    /// `CODEX_HOME`; `None` is Codex's default.
    pub home: Option<PathBuf>,
    pub install: CodexInstall,
}

/// The Codex binary hoshi runs, by where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexBinary {
    /// The settings name it.
    Configured(PathBuf),
    /// hoshi installed it.
    Managed(PathBuf),
    /// Found on the `PATH`.
    OnPath(PathBuf),
}

impl CodexBinary {
    pub fn path(&self) -> &Path {
        match self {
            Self::Configured(path) | Self::Managed(path) | Self::OnPath(path) => path,
        }
    }

    fn standalone_home(&self) -> Option<PathBuf> {
        match self {
            Self::Managed(_) => None,
            Self::Configured(path) | Self::OnPath(path) => standalone_home(path, &real_path(path)),
        }
    }
}

impl CodexSetup {
    /// The settings' binary, else hoshi's install, else `codex` on the `PATH`.
    pub fn binary(&self) -> Option<CodexBinary> {
        resolve(
            self.binary_path.as_deref(),
            || self.install.binary(),
            || find_program("codex", true),
        )
    }
}

fn resolve(
    configured: Option<&str>,
    managed: impl FnOnce() -> Option<PathBuf>,
    on_path: impl FnOnce() -> Option<PathBuf>,
) -> Option<CodexBinary> {
    if let Some(configured) = configured.map(str::trim).filter(|path| !path.is_empty()) {
        let path = find_program(configured, true).unwrap_or_else(|| PathBuf::from(configured));
        return Some(CodexBinary::Configured(path));
    }
    managed()
        .map(CodexBinary::Managed)
        .or_else(|| on_path().map(CodexBinary::OnPath))
}

/// Builds the adapter that runs Codex from its binary and `CODEX_HOME`.
pub type CodexAdapterFactory =
    Box<dyn Fn(Option<String>, Option<String>) -> Arc<dyn ProviderAdapter> + Send + Sync>;

/// The Codex adapter on the binary [`CodexSetup::binary`] picks, picked again after an install.
/// It answers for the installation and account itself and leaves the rest to the adapter.
pub struct ManagedCodex {
    setup: CodexSetup,
    make: CodexAdapterFactory,
    current: Mutex<(Option<CodexBinary>, Arc<dyn ProviderAdapter>)>,
    /// `None` until asked; then `Some(None)` when the binary can't be run.
    version: Mutex<Option<Option<String>>>,
}

impl ManagedCodex {
    pub fn new(setup: CodexSetup, make: CodexAdapterFactory) -> Self {
        let binary = setup.binary();
        let adapter = make_adapter(&make, &setup, binary.as_ref());
        Self {
            setup,
            make,
            current: Mutex::new((binary, adapter)),
            version: Mutex::default(),
        }
    }

    pub fn setup(&self) -> &CodexSetup {
        &self.setup
    }

    fn adapter(&self) -> Arc<dyn ProviderAdapter> {
        lock(&self.current).1.clone()
    }

    fn installed(&self) -> Option<String> {
        let mut version = lock(&self.version);
        version
            .get_or_insert_with(|| {
                let binary = lock(&self.current).0.clone()?;
                run_version(binary.path(), LOOKUP_TIMEOUT)
            })
            .clone()
    }
}

fn make_adapter(
    make: &CodexAdapterFactory,
    setup: &CodexSetup,
    binary: Option<&CodexBinary>,
) -> Arc<dyn ProviderAdapter> {
    make(
        binary.map(|binary| binary.path().to_string_lossy().into_owned()),
        setup
            .home
            .as_ref()
            .map(|home| home.to_string_lossy().into_owned()),
    )
}

fn updater_for(binary: Option<&CodexBinary>, install: &CodexInstall) -> Option<Updater> {
    match binary {
        None | Some(CodexBinary::Managed(_)) => {
            current_platform().map(|_| Updater::Managed(install.clone()))
        }
        Some(CodexBinary::Configured(path) | CodexBinary::OnPath(path)) => {
            own_updater(path, &real_path(path))
        }
    }
}

/// Updates a Codex the user put in place the way it was installed: `path` is where it was
/// found and `real` the file it leads to.
fn own_updater(path: &Path, real: &Path) -> Option<Updater> {
    // `codex update` finds the install through `CODEX_HOME`, which may not be the account's.
    if let Some(home) = standalone_home(path, real) {
        return Some(Updater::Command {
            program: path.to_owned(),
            args: vec!["update".into()],
            env: vec![("CODEX_HOME".into(), home.to_string_lossy().into_owned())],
        });
    }
    packages::updater(packages::manager_of(PACKAGE, path, real)?, PACKAGE)
}

/// The `CODEX_HOME` the official installer put `path` under, in `packages/standalone`.
fn standalone_home(path: &Path, real: &Path) -> Option<PathBuf> {
    real.ancestors()
        .chain(path.ancestors())
        .find(|dir| dir.ends_with("packages/standalone"))?
        .ancestors()
        .nth(2)
        .map(Path::to_owned)
}

fn real_path(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// The release `codex update` installs, which can lag npm's.
fn standalone_latest() -> Option<String> {
    packages::cached(STANDALONE_LATEST, || {
        standalone_version(&get_json(STANDALONE_LATEST)?)
    })
}

fn standalone_version(release: &Value) -> Option<String> {
    let version = release["tag_name"].as_str()?.strip_prefix("rust-v")?;
    is_version(version).then(|| version.to_owned())
}

fn account_status(
    installed: bool,
    auth: login::Auth,
    fallback: impl FnOnce() -> ProviderAccountStatus,
) -> ProviderAccountStatus {
    if !installed {
        return ProviderAccountStatus::NotInstalled;
    }
    match auth {
        login::Auth::SignedIn(_) => ProviderAccountStatus::Ready,
        login::Auth::SignedOut => ProviderAccountStatus::SignedOut,
        login::Auth::Unknown => fallback(),
    }
}

impl ProviderAdapter for ManagedCodex {
    fn provider(&self) -> &'static str {
        self.adapter().provider()
    }

    fn models(&self) -> ProviderModels {
        self.adapter().models()
    }

    fn usage_limits(&self) -> Option<UsageLimits> {
        self.adapter().usage_limits()
    }

    fn reports_usage_limits(&self) -> bool {
        self.adapter().reports_usage_limits()
    }

    fn spend_banked_reset(&self, grant_id: &str, request_id: &str) -> BankedResetSpend {
        self.adapter().spend_banked_reset(grant_id, request_id)
    }

    fn installation(&self) -> Option<Installation> {
        let binary = lock(&self.current).0.clone();
        let installed = self.installed();
        let updater = updater_for(binary.as_ref(), &self.setup.install)
            .filter(|updater| installed.is_some() || matches!(updater, Updater::Managed(_)));
        let latest = if binary
            .as_ref()
            .and_then(CodexBinary::standalone_home)
            .is_some()
        {
            standalone_latest()
        } else {
            latest_version()
        };
        Some(Installation {
            installed,
            latest,
            updater,
        })
    }

    fn forget_version(&self) {
        *lock(&self.version) = None;
        let binary = self.setup.binary();
        let mut current = lock(&self.current);
        if current.0 != binary {
            let adapter = make_adapter(&self.make, &self.setup, binary.as_ref());
            *current = (binary, adapter);
        } else {
            current.1.forget_version();
        }
    }

    fn end_idle_processes(&self) {
        self.adapter().end_idle_processes();
    }

    fn account_status(&self) -> ProviderAccountStatus {
        let installed = self.installed().is_some();
        let auth = login::auth(self.setup.home.as_deref());
        account_status(installed, auth, || self.adapter().account_status())
    }

    fn slash_commands(&self, cwd: &Path) -> Option<Vec<SlashCommand>> {
        self.adapter().slash_commands(cwd)
    }

    fn generate_title(&self, request: &TitleRequest) -> Option<GeneratedTitle> {
        self.adapter().generate_title(request)
    }

    fn write_commit_message(&self, request: &CommitMessageRequest) -> Option<CommitMessage> {
        self.adapter().write_commit_message(request)
    }

    fn write_pull_request(&self, request: &PullRequestTextRequest) -> Option<PullRequestText> {
        self.adapter().write_pull_request(request)
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        self.adapter().start_turn(request, sink)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
