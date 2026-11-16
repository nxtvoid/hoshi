//! hoshi's own update: downloads a release's installer, checks its signature, and runs it.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hoshi_core::AppInstaller;
use minisign_verify::{PublicKey, Signature};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The key the release workflow signs installers with (`TAURI_SIGNING_PRIVATE_KEY`).
const PUBLIC_KEY: &str = "RWS78lHD4FE/lEO9ZEslmiuV4LEKzj2dnGLEn8hky7EEeLeaDws24blF";
const TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 16 * 1024;
const DEB_BINARY: &str = "/usr/bin/hoshi";

/// How this machine installs a hoshi update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallerFormat {
    /// The Windows setup, run passive; it starts hoshi again when done.
    Nsis,
    /// The Debian package, installed through `pkexec`.
    Deb,
}

impl InstallerFormat {
    /// `None` where hoshi can't install itself: other platforms, or a Linux build the `.deb`
    /// didn't install.
    pub fn current() -> Option<Self> {
        if cfg!(all(windows, target_arch = "x86_64")) {
            return Some(Self::Nsis);
        }
        if cfg!(all(target_os = "linux", target_arch = "x86_64"))
            && std::env::current_exe()
                .and_then(fs::canonicalize)
                .is_ok_and(|exe| exe == Path::new(DEB_BINARY))
        {
            return Some(Self::Deb);
        }
        None
    }

    /// The release asset for `version`, as `cargo packager` names it.
    pub fn asset_name(self, version: &str) -> String {
        match self {
            Self::Nsis => format!("hoshi_{version}_x64-setup.exe"),
            Self::Deb => format!("hoshi_{version}_amd64.deb"),
        }
    }
}

/// Downloads `installer` into `dir`, replacing what an earlier download left, and keeps it
/// only when its signature checks out. `progress` gets the bytes received and the total.
pub fn download(
    installer: &AppInstaller,
    dir: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .build()
        .into();
    let get = |url: &str| {
        agent
            .get(url)
            .header("User-Agent", "hoshi")
            .call()
            .map_err(|error| error.to_string())
    };

    let signature = get(&installer.signature_url)?
        .body_mut()
        .with_config()
        .limit(MAX_SIGNATURE_BYTES)
        .read_to_string()
        .map_err(|error| error.to_string())?;

    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let path = dir.join(&installer.name);
    let partial = dir.join(format!("{}.part", installer.name));

    let mut response = get(&installer.url)?;
    let total = response.body().content_length().unwrap_or(installer.size);
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_INSTALLER_BYTES)
        .reader();
    let mut file = fs::File::create(&partial).map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    let mut buffer = vec![0; 64 * 1024];
    progress(0, total);
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
        bytes.extend_from_slice(&buffer[..read]);
        progress(bytes.len() as u64, total);
    }
    file.flush().map_err(|error| error.to_string())?;
    drop(file);

    if let Err(error) = verify(&bytes, &signature) {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    fs::rename(&partial, &path).map_err(|error| error.to_string())?;
    Ok(path)
}

/// `signature` is a `.sig` asset: a minisign signature, base64-encoded.
fn verify(bytes: &[u8], signature: &str) -> Result<(), String> {
    verify_with(PUBLIC_KEY, bytes, signature)
}

fn verify_with(public_key: &str, bytes: &[u8], signature: &str) -> Result<(), String> {
    let key = PublicKey::from_base64(public_key).map_err(|error| error.to_string())?;
    let decoded = STANDARD
        .decode(signature.trim())
        .map_err(|_| "the installer's signature is not base64".to_owned())?;
    let text = String::from_utf8(decoded)
        .map_err(|_| "the installer's signature is not text".to_owned())?;
    let signature = Signature::decode(&text).map_err(|error| error.to_string())?;
    key.verify(bytes, &signature, false)
        .map_err(|_| "the installer's signature doesn't match".to_owned())
}

/// Runs a downloaded installer. On success hoshi should quit at once: the Windows setup waits
/// for it and starts it again, and on Linux the new version is already starting.
pub fn install(format: InstallerFormat, installer: &Path) -> Result<(), String> {
    match format {
        InstallerFormat::Nsis => Command::new(installer)
            .args(["/P", "/R"])
            .spawn()
            .map(drop)
            .map_err(|error| error.to_string()),
        InstallerFormat::Deb => {
            let status = Command::new("pkexec")
                .arg("dpkg")
                .arg("-i")
                .arg(installer)
                .status()
                .map_err(|error| error.to_string())?;
            if !status.success() {
                return Err(format!("dpkg exited with {status}"));
            }
            Command::new(DEB_BINARY)
                .spawn()
                .map(drop)
                .map_err(|error| error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway key's signature of `test`, as a `.sig` asset holds it.
    const TEST_KEY: &str = "RWRF5TTYQV05jrCeV4CE6y7TLZAedb5QNhBMbqznVOdECRfFWOemzDeJ";
    const TEST_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIGNhcmdvLXBhY2thZ2VyIHNlY3JldCBrZXkKUlVSRjVUVFlRVjA1am1mczRZTUFLaDFSWDU5T2pvWkUwOHVIUzBDMFJZUE5mWUk1d2NZR3dDNDRmeThnaXhiTGZDcmR1eGl5VnVnUWFwRmRWQVVWdG0rNHoxeEVNYU5Tc3dRPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk0ODM1MDM0CWZpbGU6dGVzdC5iaW4KV3QzQXo0YzRaZ3NyR2xwdUhHUWJTRDVjZW5FcXhaZnNQSWlKbUV6bnkxcGxzL2NsZTFmMWNKZTZOS2cwdGx4VWNGd2lWYzl2N1JEYUY5YTdBQ1RSQ3c9PQo=";
    /// The release key's signature of `hoshi`, made the way the release workflow signs.
    const RELEASE_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTNzhsSEQ0RkUvbEV6TXk5eEl5SDBBTC81MDVRSFpiRWR0MzlUQ1VpUll2VEJjejFTSHZEeUIrdHpQM3VZcFVnU2RCNnBPUTBmQUVscExzSlcraEpTSWNPWHl0di9LUnd3PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk0ODM1MDg1CWZpbGU6cmVhbC5iaW4Kc2ZUUFQ5aURPdUVMOHBnS2UwQnhNZUp0L3lWQ0t0bkVHQ3FkZFhLUlRMdTVLUmdicG5NZGJEb0ZpVjJDMG5sc1pUdk45azNCSXVQMzhHZ0FwMjlIQmc9PQo=";

    #[test]
    fn accepts_only_what_the_key_signed() {
        assert_eq!(verify_with(TEST_KEY, b"test", TEST_SIGNATURE), Ok(()));
        assert!(verify_with(TEST_KEY, b"tampered", TEST_SIGNATURE).is_err());
        assert!(verify(b"test", TEST_SIGNATURE).is_err());
        assert!(verify_with(TEST_KEY, b"test", "not base64!").is_err());
    }

    #[test]
    fn the_release_key_verifies_release_signatures() {
        assert_eq!(verify(b"hoshi", RELEASE_SIGNATURE), Ok(()));
        assert!(verify(b"hoshi!", RELEASE_SIGNATURE).is_err());
    }

    /// Serves `installer` and the release signature of `hoshi` until the test ends.
    fn serve(installer: &'static [u8]) -> AppInstaller {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let body = if request.url().ends_with(".sig") {
                    RELEASE_SIGNATURE.as_bytes()
                } else {
                    installer
                };
                let _ = request.respond(tiny_http::Response::from_data(body));
            }
        });
        AppInstaller {
            name: "hoshi_9.0.0_x64-setup.exe".into(),
            url: format!("http://127.0.0.1:{port}/hoshi_9.0.0_x64-setup.exe"),
            signature_url: format!("http://127.0.0.1:{port}/hoshi_9.0.0_x64-setup.exe.sig"),
            size: installer.len() as u64,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hoshi-app-update-{name}-{}", std::process::id()))
    }

    #[test]
    fn keeps_a_signed_download_and_reports_its_progress() {
        let dir = scratch("signed");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("hoshi_8.0.0_x64-setup.exe"), "an older download").unwrap();
        let mut seen = Vec::new();

        let path = download(&serve(b"hoshi"), &dir, |received, total| {
            seen.push((received, total))
        })
        .unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"hoshi");
        assert_eq!(seen.first(), Some(&(0, 5)));
        assert_eq!(seen.last(), Some(&(5, 5)));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn drops_a_download_whose_signature_does_not_match() {
        let dir = scratch("tampered");
        let error = download(&serve(b"hoshi, tampered"), &dir, |_, _| {}).unwrap_err();
        assert!(error.contains("signature"), "{error}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn names_the_assets_as_the_packager_does() {
        assert_eq!(
            InstallerFormat::Nsis.asset_name("1.0.0"),
            "hoshi_1.0.0_x64-setup.exe"
        );
        assert_eq!(
            InstallerFormat::Deb.asset_name("1.0.0"),
            "hoshi_1.0.0_amd64.deb"
        );
    }
}
