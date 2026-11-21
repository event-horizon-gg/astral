//! Real process-death acceptance. The child runs this same test executable,
//! parks inside a transaction, and is killed by the parent without destructors.
use super::*;
use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};
use std::process::{Child, Command, Stdio};

const CASE_ENV: &str = "ASTRAL_PROCESS_RECOVERY_CASE_V1";
const TEST: &str = "settings_backup::process_recovery_tests::recovers_after_real_process_death";
const SETTINGS: [&str; 3] = [
    "bindings-cache.wtf",
    "layout-local.txt",
    "SavedVariables/EllesmereUI.lua",
];

#[derive(Serialize, Deserialize)]
struct Case {
    client: Client,
    phase: String,
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn paused(fixture: &Path) -> ! {
    let mut ready = File::create_new(fixture.join("ready")).unwrap();
    ready.write_all(b"transaction reached").unwrap();
    ready.sync_all().unwrap();
    loop {
        std::thread::park_timeout(Duration::from_secs(30));
    }
}

fn write_settings(account: &Path, mode: &str) -> io::Result<()> {
    for path in SETTINGS {
        let file = account.join(path);
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(file, format!("{mode}/{path}"))?;
    }
    Ok(())
}

fn catalog(client: Client) -> Vec<Definition> {
    let mut items = vec![
        ("ellesmere", "EllesmereUI", vec![]),
        ("profiles", "AstralProfiles", vec![]),
    ];
    if client == Client::Retail {
        items.push(("consoleport", "ConsolePort", vec!["ConsolePort_Bar".into()]));
    }
    items.push(("process-fixture", "FixtureCore", vec!["FixtureBar".into()]));
    items
        .into_iter()
        .map(|(id, folder, folders)| {
            let mut source = Definition::github_project(
                "fixture/ProcessRecovery",
                client,
                folders,
                folder.into(),
                ".*".into(),
            );
            source.id = id.into();
            source
        })
        .collect()
}

fn package(
    root: &Path,
    client: Client,
    source: &Definition,
    catalog: &[Definition],
) -> crate::packages::Inspection {
    let interface = crate::packages::active_interface(root, client).unwrap();
    let bytes = futures_lite::future::block_on(async {
        let mut bytes = Vec::new();
        let mut writer = ZipFileWriter::new(&mut bytes);
        for folder in source.module_names() {
            let dependency = if folder == "ConsolePort_Bar" {
                "## Dependencies: ConsolePort\n"
            } else {
                ""
            };
            for (name, content) in [
                (
                    format!("{folder}/{folder}.toc"),
                    format!("## Interface: {interface}\n## Version: 9.0.0\n{dependency}new.lua\n"),
                ),
                (format!("{folder}/new.lua"), "-- new addon files\n".into()),
            ] {
                writer
                    .write_entry_whole(
                        ZipEntryBuilder::new(name.into(), Compression::Deflate),
                        content.as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        }
        writer.close().await.unwrap();
        bytes
    });
    crate::packages::inspect(bytes, root, client, source, catalog).unwrap()
}

fn run_child(path: &Path) -> io::Result<()> {
    // A test-case file can only describe client/phase, never arbitrary target paths.
    let fixture = path.parent().unwrap().canonicalize()?;
    assert!(fixture.starts_with(std::env::temp_dir().canonicalize()?));
    assert!(
        fixture
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("astral-process-recovery-")
    );
    assert_eq!(path.file_name().unwrap(), "case.json");
    let case: Case = serde_json::from_slice(&fs::read(path)?)?;
    assert!(
        [
            "addon-first",
            "addon-last",
            "profile-before",
            "profile-mid",
            "profile-after"
        ]
        .contains(&case.phase.as_str())
    );
    let root = fixture.join(case.client.folder()).canonicalize()?;
    assert_eq!(root.parent(), Some(fixture.as_path()));
    let data = fixture.join("app").canonicalize()?;
    assert_eq!(data.parent(), Some(fixture.as_path()));
    let catalog = catalog(case.client);
    if case.phase.starts_with("addon-") {
        let source = catalog.last().unwrap();
        let inspected = package(&root, case.client, source, &catalog);
        let moved = std::cell::Cell::new(0);
        installer::install_with_archive(
            &inspected,
            "FixtureCore",
            ("9.0.0", ""),
            Some(&data.join("preferences.json")),
            || Ok(()),
            |from, to| {
                fs::rename(from, to)?;
                moved.set(moved.get() + 1);
                let checkpoint = if case.phase == "addon-first" {
                    1
                } else {
                    inspected.modules.len()
                };
                if moved.get() == checkpoint {
                    paused(&fixture);
                }
                Ok(())
            },
        )
        .map_err(io::Error::other)?;
    } else {
        let store = Store::open(&data, &root, case.client, "AccountOne")?;
        let review = store
            .prepare_profile(Profile::Handheld, true, &catalog, ProfileEdits::new())?
            .review;
        assert!(review.blocker.is_none(), "{:?}", review.blocker);
        assert!(!review.packages.is_empty());
        let check = || -> io::Result<()> {
            if case.phase == "profile-mid" && store.profile_state()?.interrupted {
                let changed = SETTINGS
                    .into_iter()
                    .filter(|path| {
                        fs::read_to_string(store.account.join(path)).ok().as_deref()
                            == Some(format!("handheld/{path}").as_str())
                    })
                    .count();
                if changed > 0 && changed < SETTINGS.len() {
                    paused(&fixture);
                }
            }
            Ok(())
        };
        store.apply_review_with(&review, &catalog, check, |sources, operation, finish| {
            let packages: Vec<_> = sources
                .iter()
                .map(|&source| package(&root, case.client, &catalog[source], &catalog))
                .collect();
            installer::install_profile_packages(
                &root,
                case.client,
                &packages,
                &data.join("preferences.json"),
                operation,
                || Ok(()),
                || {
                    if case.phase == "profile-before" {
                        paused(&fixture);
                    }
                    finish()?;
                    if case.phase == "profile-after" {
                        paused(&fixture);
                    }
                    Ok(())
                },
            )
            .map(|_| ())
            .map_err(io::Error::other)
        })?;
    }
    panic!("Child completed without reaching its interruption checkpoint");
}

#[test]
fn recovers_after_real_process_death() -> io::Result<()> {
    if let Some(path) = std::env::var_os(CASE_ENV) {
        return run_child(Path::new(&path));
    }
    for client in Client::ALL {
        let client_name = client.label();
        for phase in [
            "addon-first",
            "addon-last",
            "profile-before",
            "profile-mid",
            "profile-after",
        ] {
            let fixture = tempfile::Builder::new()
                .prefix("astral-process-recovery-")
                .tempdir()?;
            let root = fixture.path().join(client.folder());
            let data = fixture.path().join("app");
            let account = root.join("WTF/Account/AccountOne");
            fs::create_dir_all(root.join("Interface/AddOns"))?;
            fs::create_dir_all(&data)?;
            fs::create_dir_all(&account)?;
            fs::write(
                fixture.path().join(".build.info"),
                "Product!STRING:0|Active!DEC:1|Version!STRING:0\nwow|1|12.1.5.123\nwowf|1|2.5.5.123\n",
            )?;
            fs::write(
                root.join("WTF/Config.wtf"),
                "SET graphicsQuality \"9\"\nSET GamePadEnable \"0\"\n",
            )?;
            let catalog = catalog(client);
            write_settings(&account, "desktop")?;
            if phase.starts_with("addon-") {
                for folder in ["FixtureCore", "FixtureBar"] {
                    let module = root.join("Interface/AddOns").join(folder);
                    fs::create_dir(&module)?;
                    fs::write(
                        module.join(format!("{folder}.toc")),
                        "## Version: 1\nold.lua\n",
                    )?;
                    fs::write(module.join("old.lua"), "-- original addon files\n")?;
                }
            } else {
                let store = Store::open(&data, &root, client, "AccountOne")?;
                store.switch_with_policy(Profile::Handheld, false, &catalog, || Ok(()))?;
                write_settings(&account, "handheld")?;
                store.switch_checked(Profile::Regular, &catalog, || Ok(()))?;
            }
            let config = fixture.path().join("case.json");
            preferences::save_json(
                &config,
                &Case {
                    client,
                    phase: phase.into(),
                },
            )?;
            let mut child = Running(
                Command::new(std::env::current_exe()?)
                    .args(["--exact", TEST, "--nocapture"])
                    .env(CASE_ENV, &config)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
                    .spawn()?,
            );
            let deadline = Instant::now() + Duration::from_secs(20);
            while !fixture.path().join("ready").exists() {
                assert!(
                    child.0.try_wait()?.is_none(),
                    "{client_name}/{phase}: child exited before checkpoint"
                );
                assert!(
                    Instant::now() < deadline,
                    "{client_name}/{phase}: child never reached checkpoint"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            let records = installer::list(&root, client).map_err(io::Error::other)?;
            let pending: Vec<_> = records.iter().filter(|record| record.interrupted).collect();
            assert_eq!(pending.len(), 1, "{client_name}/{phase}");
            assert!(
                installer::ensure_no_pending_profile_packages(&root).is_err()
                    || phase.starts_with("addon-")
            );
            if phase == "profile-mid" {
                let store = Store::open(&data, &root, client, "AccountOne")?;
                assert!(store.profile_state()?.interrupted);
                let changed = SETTINGS
                    .into_iter()
                    .filter(|path| {
                        fs::read_to_string(account.join(path)).ok().as_deref()
                            == Some(format!("handheld/{path}").as_str())
                    })
                    .count();
                assert!(
                    changed > 0 && changed < SETTINGS.len(),
                    "the parent must observe partially applied settings"
                );
            }
            child.0.kill()?;
            assert!(!child.0.wait()?.success());
            let backup = pending[0];
            if let Some(operation) = &backup.transaction.profile_switch {
                let store = Store::open(&data, &root, client, "AccountOne")?;
                store
                    .recover_package_switch_checked(&backup.path, operation, &catalog, || Ok(()))?;
                let committed = phase == "profile-after";
                assert_eq!(
                    store.profile_state()?.active,
                    if committed {
                        Profile::Handheld
                    } else {
                        Profile::Regular
                    }
                );
                assert!(!store.profile_state()?.interrupted);
                for folder in ["EllesmereUI", "AstralProfiles"] {
                    assert_eq!(
                        root.join("Interface/AddOns").join(folder).is_dir(),
                        committed
                    );
                }
                assert_eq!(
                    root.join("Interface/AddOns/ConsolePort").is_dir(),
                    committed && client == Client::Retail
                );
                assert_eq!(
                    root.join("Interface/AddOns/ConsolePort_Bar").is_dir(),
                    committed && client == Client::Retail
                );
                assert_eq!(backup.path.join("manifest.json").is_file(), committed);
                let version_location = if committed {
                    root.join("Interface/AddOns")
                } else {
                    backup.path.join("interrupted-files")
                };
                for folder in ["EllesmereUI", "AstralProfiles"] {
                    assert_eq!(
                        fs::read_to_string(version_location.join(folder).join("new.lua"))?,
                        "-- new addon files\n"
                    );
                }
                if client == Client::Retail {
                    for folder in ["ConsolePort", "ConsolePort_Bar"] {
                        assert_eq!(
                            fs::read_to_string(version_location.join(folder).join("new.lua"))?,
                            "-- new addon files\n"
                        );
                    }
                }

                for path in SETTINGS {
                    assert_eq!(
                        fs::read_to_string(account.join(path))?,
                        format!("{}/{path}", if committed { "handheld" } else { "desktop" })
                    );
                }
                assert!(installer::ensure_no_pending_profile_packages(&root).is_ok());
            } else {
                installer::recover_with(&root, client, &backup.path, || Ok(()), |_| unreachable!())
                    .map_err(io::Error::other)?;
                for folder in ["FixtureCore", "FixtureBar"] {
                    assert_eq!(
                        fs::read_to_string(
                            root.join("Interface/AddOns").join(folder).join("old.lua")
                        )?,
                        "-- original addon files\n"
                    );
                    assert!(
                        !root
                            .join("Interface/AddOns")
                            .join(folder)
                            .join("new.lua")
                            .exists()
                    );
                }
                let interrupted = backup.path.join("interrupted-files");
                let mut saved_new = 0;
                for entry in fs::read_dir(&interrupted)? {
                    let file = entry?.path().join("new.lua");
                    if file.is_file() {
                        assert_eq!(fs::read_to_string(file)?, "-- new addon files\n");
                        saved_new += 1;
                    }
                }
                assert_eq!(saved_new, if phase == "addon-first" { 1 } else { 2 });
                assert!(backup.path.join("recovered.json").is_file());
            }
            assert!(!backup.path.join("pending.json").exists());
            let graphics = fs::read_to_string(root.join("WTF/Config.wtf"))?;
            assert!(graphics.contains("SET graphicsQuality \"9\""));
            assert!(graphics.contains(if phase == "profile-after" {
                "SET GamePadEnable \"1\""
            } else {
                "SET GamePadEnable \"0\""
            }));
            assert!(
                !installer::list_saved(&data.join("backups"), None)
                    .map_err(io::Error::other)?
                    .is_empty()
            );
            eprintln!("Verified process kill and recovery: {client_name}/{phase}");
        }
    }
    Ok(())
}
