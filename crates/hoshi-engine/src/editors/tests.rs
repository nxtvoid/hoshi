use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hoshi-editors-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "").unwrap();
}

fn launcher_name(launcher: &str) -> String {
    if cfg!(windows) {
        format!("{launcher}64.exe")
    } else {
        launcher.to_owned()
    }
}

#[test]
fn finds_a_command_with_a_platform_extension() {
    let dir = scratch("path");
    let file = dir.join(format!("fake-editor{}", EXTENSIONS[0]));
    touch(&file);

    assert_eq!(find_in(&dir, "fake-editor"), Some(file));
    assert_eq!(find_in(&dir, "missing-editor"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finds_the_newest_jetbrains_install_off_the_path() {
    let root = scratch("installs");
    let name = launcher_name("idea");
    touch(&root.join("IntelliJ IDEA 2024.3/bin").join(&name));
    let newest = root.join("IntelliJ IDEA 2025.1/bin").join(&name);
    touch(&newest);
    touch(
        &root
            .join("WebStorm 2025.1/bin")
            .join(launcher_name("webstorm")),
    );
    let places = Places {
        install_roots: vec![root.clone()],
        ..Places::default()
    };

    assert_eq!(
        locate(Editor::Idea, &places),
        Some(Launcher::Program(newest))
    );
    assert_eq!(locate(Editor::Goland, &places), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn finds_an_ide_in_an_older_toolbox_layout() {
    let root = scratch("toolbox");
    let launcher = root
        .join("IDEA-U/ch-0/251.23774.435/bin")
        .join(launcher_name("idea"));
    touch(&launcher);
    let places = Places {
        install_roots: vec![root.clone()],
        ..Places::default()
    };

    assert_eq!(
        locate(Editor::Idea, &places),
        Some(Launcher::Program(launcher))
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// `path` as a desktop entry writes it inside double quotes.
fn quoted(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\\\\\")
}

fn words(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn finds_an_ide_by_its_desktop_entry() {
    let dir = scratch("desktop");
    let bin = dir.join("bin");
    let flatpak = bin.join(format!("flatpak{}", EXTENSIONS[0]));
    let env = bin.join(format!("env{}", EXTENSIONS[0]));
    touch(&flatpak);
    touch(&env);
    touch(&bin.join(format!("sh{}", EXTENSIONS[0])));
    let goland = dir.join("Go Land/bin/goland.sh");
    let pycharm = dir.join("snap/pycharm-community");
    let rider = dir.join("rider/bin/rider.sh");
    let clion = dir.join("clion/bin/clion.sh");
    for launcher in [&goland, &pycharm, &rider, &clion] {
        touch(launcher);
    }
    let user = dir.join("user");
    let system = dir.join("system");
    let write = |path: PathBuf, text: String| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        system.join("com.jetbrains.IntelliJ-IDEA-Ultimate.desktop"),
        "[Desktop Action new]\nExec=other\n\n[Desktop Entry]\nType=Application\n\
             Name=IntelliJ IDEA Ultimate\nExec=flatpak run --branch=stable --arch=x86_64 \
             --command=idea-wrapper --file-forwarding com.jetbrains.IntelliJ-IDEA-Ultimate \
             @@ %F @@\nStartupWMClass=com.jetbrains.IntelliJ-IDEA-Ultimate\n\
             X-Flatpak=com.jetbrains.IntelliJ-IDEA-Ultimate\n"
            .into(),
    );
    write(
        system.join("goland.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nExec=\"{}\" %f\n",
            quoted(&goland)
        ),
    );
    write(
        system.join("pycharm-community_pycharm-community.desktop"),
        format!(
            "[Desktop Entry]\nExec=env BAMF_DESKTOP_FILE_HINT=/var/lib/x.desktop \"{}\" %f\n\
                 StartupWMClass=jetbrains-pycharm-ce\n",
            quoted(&pycharm)
        ),
    );
    write(
        system.join("a-rider.desktop"),
        format!(
            "[Desktop Entry]\nExec=\"{}\" %f\nStartupWMClass=jetbrains-rider\n",
            quoted(&dir.join("gone/bin/rider.sh"))
        ),
    );
    write(
        system.join("b-rider.desktop"),
        format!("[Desktop Entry]\nExec=\"{}\" %f\n", quoted(&rider)),
    );
    write(
        system.join("clion.desktop"),
        format!(
            "[Desktop Entry]\nExec=sh -c \"'{}' %f\"\nStartupWMClass=jetbrains-clion\n",
            quoted(&clion)
        ),
    );
    write(
        system.join("jetbrains-toolbox.desktop"),
        "[Desktop Entry]\nExec=flatpak run toolbox %u\nStartupWMClass=jetbrains-toolbox\n".into(),
    );
    write(
        system.join("webstorm.desktop"),
        "[Desktop Entry]\nExec=flatpak run webstorm %f\nStartupWMClass=jetbrains-webstorm\n".into(),
    );
    write(
        user.join("webstorm.desktop"),
        "[Desktop Entry]\nHidden=true\n".into(),
    );
    write(
        system.join("com.jetbrains.IntelliJ-IDEA-Community.desktop"),
        "[Desktop Entry]\nExec=flatpak run community @@ %F @@\n\
             X-Flatpak=com.jetbrains.IntelliJ-IDEA-Community\n"
            .into(),
    );
    write(
        system.join("jetbrains-idea-ce.desktop"),
        "[Desktop Entry]\nExec=flatpak run ce %f\nStartupWMClass=jetbrains-idea-ce\n".into(),
    );
    write(
        user.join("goland.desktop"),
        format!(
            "[Desktop Entry]\nExec=\"{}\" %f\n",
            quoted(&dir.join("gone/bin/goland.sh"))
        ),
    );
    write(
        system.join("phpstorm-x.desktop"),
        "[Desktop Entry]\nExec=flatpak run x %f\nStartupWMClass=jetbrains-phpstormx\n".into(),
    );
    write(
        system.join("com.jetbrains.RubyMine.desktop"),
        "[Desktop Entry]\nExec=flatpak run --command=rubymine --file-forwarding \
             com.jetbrains.RubyMine @@u %U @@\nStartupWMClass=rubymine\n"
            .into(),
    );
    let places = Places {
        path: vec![bin],
        applications: vec![user, system],
        ..Places::default()
    };

    assert_eq!(
        locate(Editor::Idea, &places),
        Some(Launcher::Exec {
            program: flatpak.clone(),
            before: words(&[
                "run",
                "--branch=stable",
                "--arch=x86_64",
                "--command=idea-wrapper",
                "--file-forwarding",
                "com.jetbrains.IntelliJ-IDEA-Ultimate",
                "@@",
            ]),
            after: words(&["@@"]),
        }),
        "a Flathub IDE, the folder where it takes its files, Ultimate over Community"
    );
    assert_eq!(
        locate(Editor::Goland, &places),
        Some(Launcher::Exec {
            program: goland,
            before: Vec::new(),
            after: Vec::new(),
        }),
        "a user's entry whose IDE is gone doesn't hide the system's"
    );
    assert_eq!(
        locate(Editor::Rubymine, &places),
        Some(Launcher::Exec {
            program: flatpak,
            before: words(&[
                "run",
                "--command=rubymine",
                "--file-forwarding",
                "com.jetbrains.RubyMine",
                "@@u",
            ]),
            after: words(&["@@"]),
        })
    );
    assert_eq!(
        locate(Editor::Pycharm, &places),
        Some(Launcher::Exec {
            program: env,
            before: vec![
                "BAMF_DESKTOP_FILE_HINT=/var/lib/x.desktop".into(),
                pycharm.display().to_string(),
            ],
            after: Vec::new(),
        }),
        "a snap's entry, run through env"
    );
    assert_eq!(
        locate(Editor::Rider, &places),
        Some(Launcher::Exec {
            program: rider,
            before: Vec::new(),
            after: Vec::new(),
        }),
        "an entry whose IDE is gone gives way"
    );
    assert_eq!(locate(Editor::Clion, &places), None, "a shell line");
    assert_eq!(
        locate(Editor::Webstorm, &places),
        None,
        "an earlier hidden entry hides it"
    );
    assert_eq!(
        locate(Editor::Phpstorm, &places),
        None,
        "another class that starts the same"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_user_s_desktop_entry_overrides_the_system_s_of_the_same_name() {
    let dir = scratch("desktop-override");
    let bin = dir.join("bin");
    let flatpak = bin.join(format!("flatpak{}", EXTENSIONS[0]));
    touch(&flatpak);
    let write = |path: PathBuf, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        dir.join("user/idea.desktop"),
        "[Desktop Entry]\nExec=flatpak run mine %f\nStartupWMClass=jetbrains-idea-ce\n",
    );
    write(
        dir.join("system/idea.desktop"),
        "[Desktop Entry]\nExec=flatpak run theirs %f\nStartupWMClass=jetbrains-idea\n",
    );
    let places = Places {
        path: vec![bin],
        applications: vec![dir.join("user"), dir.join("system")],
        ..Places::default()
    };

    assert_eq!(
        locate(Editor::Idea, &places),
        Some(Launcher::Exec {
            program: flatpak,
            before: words(&["run", "mine"]),
            after: Vec::new(),
        })
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reads_an_exec_line_as_desktops_do() {
    assert_eq!(
        exec_words(&unescape(
            r#""/opt/my ide/bin/idea.sh" 'single  quoted' back\\ slash "a\\"b\\\\c\\x" %%U"#
        )),
        words(&[
            "/opt/my ide/bin/idea.sh",
            "single  quoted",
            "back slash",
            "a\"b\\c\\x",
            "%%U",
        ])
    );
    assert_eq!(unescape(r"a\sb\\c\q"), r"a b\c\q");
    assert_eq!(without_field_codes("%%U").as_deref(), Some("%U"));
    assert_eq!(without_field_codes("--open=%f").as_deref(), Some("--open="));
    assert_eq!(without_field_codes("%k"), None);
    assert!(is_assignment("BAMF_HINT=x") && !is_assignment("--flag=x"));
}

#[test]
fn finds_a_toolbox_script_named_with_a_number() {
    let dir = scratch("numbered");
    let script = dir.join(format!("idea2{}", EXTENSIONS[0]));
    touch(&script);
    touch(&dir.join(format!("idea12{}", EXTENSIONS[0])));
    touch(&dir.join(format!("ideax{}", EXTENSIONS[0])));
    let places = Places {
        toolbox_scripts: vec![dir.clone()],
        ..Places::default()
    };

    assert_eq!(
        locate(Editor::Idea, &places),
        Some(Launcher::Program(script))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finds_a_command_in_a_usual_folder_off_the_path() {
    let dir = scratch("bins");
    let idea = dir.join(format!("idea{}", EXTENSIONS[0]));
    touch(&idea);
    let places = Places {
        bins: vec![dir.clone()],
        ..Places::default()
    };

    assert_eq!(locate(Editor::Idea, &places), Some(Launcher::Program(idea)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_path_wins_over_an_install_folder() {
    let dir = scratch("both");
    let on_path = dir.join("path").join(format!("idea{}", EXTENSIONS[0]));
    touch(&on_path);
    touch(
        &dir.join("installs/IntelliJ IDEA 2025.1/bin")
            .join(launcher_name("idea")),
    );
    let places = Places {
        path: vec![dir.join("path")],
        install_roots: vec![dir.join("installs")],
        ..Places::default()
    };

    assert_eq!(
        locate(Editor::Idea, &places),
        Some(Launcher::Program(on_path))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_path_reaches_the_launcher_with_the_system_s_separators() {
    let expected = if cfg!(windows) {
        r"C:\Users\dev\shop\report.pdf"
    } else {
        "C:/Users/dev/shop/report.pdf"
    };
    assert_eq!(native_path("C:/Users/dev/shop/report.pdf"), expected);
    assert_eq!(
        native_path("/home/dev/shop/report.pdf").contains('/'),
        !cfg!(windows)
    );
}
