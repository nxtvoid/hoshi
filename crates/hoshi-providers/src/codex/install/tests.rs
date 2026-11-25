use super::*;
use hoshi_core::CodexAccount;

fn latest() -> Value {
    json!({
        "name": "@openai/codex",
        "version": "0.169.0",
        "optionalDependencies": {
            "@openai/codex-linux-x64": "npm:@openai/codex@0.169.0-linux-x64",
            "@openai/codex-linux-arm64": "npm:@openai/codex@0.169.0-linux-arm64",
            "@openai/codex-darwin-x64": "npm:@openai/codex@0.169.0-darwin-x64",
            "@openai/codex-darwin-arm64": "npm:@openai/codex@0.169.0-darwin-arm64",
            "@openai/codex-win32-x64": "npm:@openai/codex@0.169.0-win32-x64",
            "@openai/codex-win32-arm64": "npm:@openai/codex@0.169.0-win32-arm64",
        },
    })
}

#[test]
fn each_platform_downloads_its_own_package() {
    let latest = latest();
    let expected = [
        ("linux", "x86_64", "linux-x64"),
        ("linux", "aarch64", "linux-arm64"),
        ("macos", "x86_64", "darwin-x64"),
        ("macos", "aarch64", "darwin-arm64"),
        ("windows", "x86_64", "win32-x64"),
        ("windows", "aarch64", "win32-arm64"),
    ];

    for (os, arch, npm) in expected {
        let platform = platform_for(os, arch).unwrap();
        let (name, version) = platform_package(&latest, &platform).unwrap();
        assert_eq!(
            package_url(&name, &version),
            format!("https://registry.npmjs.org/@openai/codex/0.169.0-{npm}")
        );
    }
    assert_eq!(version_of(&latest).as_deref(), Some("0.169.0"));
    assert_eq!(platform_for("freebsd", "x86_64"), None);
    assert_eq!(platform_for("windows", "x86"), None);
}

#[test]
fn a_platform_package_gives_its_tarball_and_checksum() {
    let sha512 = vec![7u8; 64];
    let integrity = format!(
        "sha512-{}",
        base64::engine::general_purpose::STANDARD.encode(&sha512)
    );
    let tarball = "https://registry.npmjs.org/@openai/codex/-/codex-0.169.0-win32-x64.tgz";
    let package = json!({
        "name": "@openai/codex",
        "version": "0.169.0-win32-x64",
        "dist": { "tarball": tarball, "integrity": integrity },
    });

    assert_eq!(
        release_of("0.169.0", &package),
        Some(Release {
            version: "0.169.0".into(),
            tarball: tarball.into(),
            sha512,
        })
    );
    let elsewhere =
        json!({ "dist": { "tarball": "https://example.com/codex.tgz", "integrity": integrity } });
    assert_eq!(release_of("0.169.0", &elsewhere), None);
    let sha1_only = json!({ "dist": { "tarball": tarball, "integrity": "sha1-abc" } });
    assert_eq!(release_of("0.169.0", &sha1_only), None);
}

#[test]
fn a_platform_package_must_alias_codex_itself() {
    let platform = platform_for("linux", "x86_64").unwrap();
    let plain = json!({ "optionalDependencies": { "@openai/codex-linux-x64": "0.170.0" } });
    let foreign = json!({
        "optionalDependencies": { "@openai/codex-linux-x64": "npm:evil-codex@1.0.0" },
    });

    assert_eq!(platform_package(&plain, &platform), None);
    assert_eq!(platform_package(&foreign, &platform), None);
    assert_eq!(platform_package(&json!({}), &platform), None);
}

#[test]
fn only_the_platforms_vendor_folder_is_unpacked() {
    let platform = platform_for("windows", "x86_64").unwrap();
    let destination = |path: &str| entry_destination(Path::new(path), &platform);

    assert_eq!(
        destination("package/vendor/x86_64-pc-windows-msvc/bin/codex.exe"),
        Ok(Some(PathBuf::from("bin").join("codex.exe")))
    );
    assert_eq!(destination("package/package.json"), Ok(None));
    assert_eq!(
        destination("package/vendor/aarch64-pc-windows-msvc/bin/codex.exe"),
        Ok(None)
    );
    assert_eq!(
        destination("package/vendor/x86_64-pc-windows-msvc"),
        Ok(None)
    );
    assert_eq!(
        destination("package/vendor/x86_64-pc-windows-msvc/../../escape"),
        Err(())
    );
    assert_eq!(destination("/etc/passwd"), Err(()));
}

#[test]
fn the_manifest_must_name_the_release_and_its_binary() {
    let platform = platform_for("windows", "x86_64").unwrap();
    let manifest = json!({
        "layoutVersion": 1,
        "version": "0.169.0",
        "target": "x86_64-pc-windows-msvc",
        "entrypoint": "bin/codex.exe",
    });

    assert!(manifest_matches(&manifest, &platform, "0.169.0"));
    assert!(!manifest_matches(&manifest, &platform, "0.170.0"));
    let linux = platform_for("linux", "x86_64").unwrap();
    assert!(!manifest_matches(&manifest, &linux, "0.169.0"));
}

#[test]
fn only_plain_versions_name_a_release_folder() {
    assert!(is_version("0.169.0"));
    assert!(is_version("0.170.0-alpha.2"));
    assert!(!is_version("0.169"));
    assert!(!is_version("../0.169.0"));
    assert!(!is_version("0.169.0-.."));
    assert!(!is_version("0.169.0/x"));
}

#[test]
fn the_cli_reports_its_version() {
    assert_eq!(
        parse_cli_version("codex-cli 0.169.0\n").as_deref(),
        Some("0.169.0")
    );
    assert_eq!(parse_cli_version("0.169.0"), None);
    assert_eq!(parse_cli_version(""), None);
}

#[test]
fn the_settings_binary_wins_then_hoshis_install_then_the_path() {
    let managed = || Some(PathBuf::from("/hoshi/tools/codex/0.169.0/bin/codex"));
    let on_path = || Some(PathBuf::from("/usr/local/bin/codex"));
    let none = || None;

    assert_eq!(
        resolve(Some("/opt/codex/bin/codex"), managed, on_path),
        Some(CodexBinary::Configured(PathBuf::from(
            "/opt/codex/bin/codex"
        )))
    );
    assert_eq!(
        resolve(Some("  "), managed, on_path),
        Some(CodexBinary::Managed(PathBuf::from(
            "/hoshi/tools/codex/0.169.0/bin/codex"
        )))
    );
    assert_eq!(
        resolve(None, none, on_path),
        Some(CodexBinary::OnPath(PathBuf::from("/usr/local/bin/codex")))
    );
    assert_eq!(resolve(None, none, none), None);
}

#[test]
fn hoshis_own_install_is_updated_by_hoshi() {
    let install = CodexInstall::new(Path::new("/hoshi"));
    let managed = CodexBinary::Managed(PathBuf::from("/hoshi/tools/codex/0.1.0/bin/codex"));
    let expected = current_platform().map(|_| Updater::Managed(install.clone()));

    assert_eq!(updater_for(Some(&managed), &install), expected);
    assert_eq!(updater_for(None, &install), expected);
}

#[test]
fn a_standalone_codex_updates_itself_in_the_home_it_is_under() {
    let path = Path::new("/home/dev/.local/bin/codex");
    let real = Path::new("/home/dev/.codex/packages/standalone/releases/0.1.0/bin/codex");

    assert_eq!(
        own_updater(path, real),
        Some(Updater::Command {
            program: path.to_owned(),
            args: vec!["update".into()],
            env: vec![("CODEX_HOME".into(), "/home/dev/.codex".into())],
        })
    );
}

#[test]
fn a_codex_of_unknown_origin_is_left_to_the_user() {
    let path = Path::new("/opt/codex");

    assert_eq!(own_updater(path, path), None);
}

#[test]
fn the_standalone_channel_names_its_release_by_tag() {
    let version = |tag: &str| standalone_version(&json!({ "tag_name": tag }));

    assert_eq!(version("rust-v0.172.0"), Some("0.172.0".into()));
    assert_eq!(version("0.172.0"), None);
}

#[test]
fn the_account_follows_auth_json_and_asks_the_adapter_when_it_cant_tell() {
    let signed_in = login::Auth::SignedIn(CodexAccount::default());
    let ready = || ProviderAccountStatus::Ready;

    assert_eq!(
        account_status(false, signed_in.clone(), ready),
        ProviderAccountStatus::NotInstalled
    );
    assert_eq!(
        account_status(true, signed_in, || ProviderAccountStatus::SignedOut),
        ProviderAccountStatus::Ready
    );
    assert_eq!(
        account_status(true, login::Auth::SignedOut, ready),
        ProviderAccountStatus::SignedOut
    );
    assert_eq!(
        account_status(true, login::Auth::Unknown, ready),
        ProviderAccountStatus::Ready
    );
}

#[test]
fn the_active_release_needs_its_completion_record() {
    let Some(platform) = current_platform() else {
        return;
    };
    let home = std::env::temp_dir().join(format!("hoshi-codex-install-{}", unique()));
    let install = CodexInstall::new(&home);
    let bin = install.dir.join("0.169.0").join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join(platform.executable()), b"binary").unwrap();
    fs::write(install.dir.join(ACTIVE_FILE), r#"{"version":"0.169.0"}"#).unwrap();
    assert_eq!(install.binary(), None);

    fs::write(install.dir.join("0.169.0").join(COMPLETE_FILE), "{}").unwrap();
    assert_eq!(install.binary(), Some(bin.join(platform.executable())));

    fs::write(install.dir.join(ACTIVE_FILE), r#"{"version":"../x"}"#).unwrap();
    assert_eq!(install.binary(), None);
    let _ = fs::remove_dir_all(&home);
}

#[test]
#[ignore = "downloads Codex from npm, about 200 MB"]
fn installs_the_latest_release_into_a_new_home() {
    let home = std::env::temp_dir().join(format!("hoshi-codex-live-{}", unique()));
    let install = CodexInstall::new(&home);
    let mut steps = Vec::new();

    let version = install.install(|step| steps.push(step)).unwrap();

    let binary = install.binary().expect("the install is active");
    assert_eq!(
        run_version(&binary, VERIFY_TIMEOUT).as_deref(),
        Some(version.as_str())
    );
    assert!(steps.contains(&UpdateStep::Extracting));
    assert!(steps.contains(&UpdateStep::Verifying));
    assert!(steps.contains(&UpdateStep::Downloading { percent: Some(100) }));

    let mut again = Vec::new();
    assert_eq!(install.install(|step| again.push(step)).unwrap(), version);
    assert!(!again.contains(&UpdateStep::Extracting));
    let _ = fs::remove_dir_all(&home);
}
