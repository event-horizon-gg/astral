use super::Release;
use futures_lite::future::block_on;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

const APP: &str = "Astral Native.app";
const ID: &str = "com.joaoseixas.astral.native";
const EXEC: &str = "Contents/MacOS/astral-native";

fn bundle_at(executable: &Path) -> Option<&Path> {
    let bundle = executable.ancestors().nth(3)?;
    (bundle.file_name()?.to_str()? == APP
        && executable.strip_prefix(bundle).ok()? == Path::new(EXEC))
    .then_some(bundle)
}

pub(super) fn installed() -> bool {
    std::env::current_exe().is_ok_and(|exe| bundle_at(&exe).is_some())
}

pub(super) fn launch(
    file: &tempfile::NamedTempFile,
    release: &Release,
    data: &Path,
) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let target =
        bundle_at(&executable).ok_or("Install Astral Native in Applications before updating.")?;
    let stage = prepare_update(target, super::CURRENT, file, release)?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let ack = super::ack_path(data, &token).ok_or("Couldn't prepare update acknowledgement.")?;
    let helper = stage.path().join("install.sh");
    fs::write(&helper, include_str!("install-app-mac.sh")).map_err(|error| error.to_string())?;
    crate::installer::game_closed()?;
    Command::new("/bin/sh")
        .arg(&helper)
        .arg(std::process::id().to_string())
        .arg(target)
        .arg(stage.path())
        .arg(ack)
        .arg(token)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Couldn't start the installer. {error}"))?;
    let _ = stage.keep();
    Ok(())
}

fn prepare_update(
    target: &Path,
    current: &str,
    file: &tempfile::NamedTempFile,
    release: &Release,
) -> Result<tempfile::TempDir, String> {
    verify_bundle(target, current)?;
    let parent = target
        .parent()
        .ok_or("The Astral application folder could not be found.")?;
    let stage = tempfile::Builder::new()
        .prefix(".astral-native-update-")
        .tempdir_in(parent)
        .map_err(|error| format!("Couldn't stage the update beside Astral. {error}"))?;
    extract(super::verified_bytes(file, release)?, stage.path())?;
    verify_bundle(&stage.path().join(APP), &release.version.to_string())?;
    Ok(stage)
}

fn verify_bundle(bundle: &Path, version: &str) -> Result<(), String> {
    if !bundle.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
        return Err("Invalid Astral application bundle.".into());
    }
    let executable = bundle.join(EXEC);
    if !executable
        .symlink_metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o100 != 0)
    {
        return Err("The Astral executable is missing.".into());
    }
    let output = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(bundle.join("Contents/Info.plist"))
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("Couldn't read the Astral application identity.".into());
    }
    let info: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    if info["CFBundleIdentifier"] != ID
        || info["CFBundleShortVersionString"] != version
        || info["CFBundleExecutable"] != "astral-native"
    {
        return Err("The application identity or version does not match the release.".into());
    }
    let output = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("The macOS application signature could not be verified.".into());
    }
    Ok(())
}

fn extract(bytes: Vec<u8>, directory: &Path) -> Result<(), String> {
    block_on(super::zip::extract(
        bytes,
        directory,
        |name, is_dir| name.starts_with("Astral Native.app/") || (is_dir && name == APP),
        "Astral Native.app/Contents/MacOS/astral-native",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a release ZIP built by native/package-mac.sh"]
    fn release_archive_extracts_verifies_and_runs() {
        use super::super::{
            BASE, CURRENT, Envelope, REPO, TAG_PREFIX, Target, verified_bytes, verify,
        };
        use base64::{Engine, engine::general_purpose::STANDARD};
        use ring::{
            rand::SystemRandom,
            signature::{Ed25519KeyPair, KeyPair},
        };
        use sha2::{Digest, Sha256};

        let archive = std::env::var_os("ASTRAL_TEST_MAC_ARCHIVE")
            .expect("set ASTRAL_TEST_MAC_ARCHIVE to the release ZIP");
        let data = fs::read(archive).unwrap();
        let target = Target::current().unwrap();
        let version = super::super::stable(CURRENT).unwrap();
        let tag = format!("{TAG_PREFIX}{version}");
        let manifest = serde_json::json!({
            "schema": 1, "repository": REPO, "app": "astral-native", "version": CURRENT,
            "assets": {
                target.key(): {
                    "url": format!("{BASE}{tag}/{}", target.filename(&version)),
                    "sha256": format!("{:x}", Sha256::digest(&data)),
                    "size": data.len()
                }
            }
        });
        let payload = serde_json::to_vec(&manifest).unwrap();
        let key = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
        let release = verify(
            Envelope {
                payload: STANDARD.encode(&payload),
                signature: STANDARD.encode(key.sign(&payload).as_ref()),
            },
            key.public_key().as_ref(),
            &tag,
            target,
        )
        .unwrap();
        let download = tempfile::NamedTempFile::new().unwrap();
        fs::write(download.path(), data).unwrap();
        let stage = tempfile::tempdir().unwrap();
        extract(verified_bytes(&download, &release).unwrap(), stage.path()).unwrap();
        let app = stage.path().join(APP);
        verify_bundle(&app, super::super::CURRENT).unwrap();
        let output = Command::new(app.join(EXEC))
            .arg("--version")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            format!("Astral Native {}", super::super::CURRENT)
        );
        super::super::assert_packaged_fixture_scan(&app.join(EXEC));
    }

    #[test]
    fn packaged_zip_keeps_bundle_signature_and_version_after_extraction() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join(APP);
        signed_fixture_app(&app, "1.2.3", "#!/bin/sh\nexit 0\n");
        let stage = tempfile::tempdir().unwrap();
        extract(zip_fixture(temp.path()), stage.path()).unwrap();
        verify_bundle(&stage.path().join(APP), "1.2.3").unwrap();
        assert!(verify_bundle(&stage.path().join(APP), "1.2.4").is_err());
    }

    fn signed_fixture_app(app: &Path, version: &str, script: &str) {
        fixture_app(app, script);
        fs::write(
            app.join("Contents/Info.plist"),
            format!(
                r#"<?xml version="1.0"?><plist version="1.0"><dict>
            <key>CFBundleIdentifier</key><string>{ID}</string>
            <key>CFBundleExecutable</key><string>astral-native</string>
            <key>CFBundlePackageType</key><string>APPL</string>
            <key>CFBundleShortVersionString</key><string>{version}</string>
            </dict></plist>"#
            ),
        )
        .unwrap();
        let signed = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(app)
            .output()
            .unwrap();
        assert!(
            signed.status.success(),
            "{}",
            String::from_utf8_lossy(&signed.stderr)
        );
    }

    fn zip_fixture(directory: &Path) -> Vec<u8> {
        let zipped = Command::new("/usr/bin/zip")
            .current_dir(directory)
            .args(["-q", "-r", "app.zip", APP])
            .status()
            .unwrap();
        assert!(zipped.success());
        fs::read(directory.join("app.zip")).unwrap()
    }

    fn fixture_app(path: &Path, script: &str) {
        let exe = path.join(EXEC);
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, script).unwrap();
        fs::set_permissions(exe, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn restart_fixture(target: &Path, stage: &Path, ack: &Path) -> std::process::ExitStatus {
        restart_fixture_with_game_probe(target, stage, ack, "/usr/bin/false")
    }

    fn restart_fixture_with_game_probe(
        target: &Path,
        stage: &Path,
        ack: &Path,
        probe: &str,
    ) -> std::process::ExitStatus {
        // The test sandbox cannot enumerate desktop processes.
        let helper = include_str!("install-app-mac.sh").replace(
            "/usr/bin/pgrep -x 'Wow|World of Warcraft|WowClassic|WowF|WowForever'",
            probe,
        );
        Command::new("/bin/sh")
            .args(["-c", &helper, "install", "2147483647"])
            .arg(target)
            .arg(stage)
            .arg(ack)
            .arg("token")
            .env("ASTRAL_TEST_UPDATE_ACK", ack)
            .status()
            .unwrap()
    }

    #[test]
    fn restart_helper_keeps_the_old_bundle_if_wow_starts_or_the_probe_fails() {
        for (probe, reason) in [
            ("/usr/bin/true", "wow-running"),
            ("(exit 2)", "game-check-failed"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let target = temp.path().join(APP);
            let stage = temp.path().join("stage");
            let marker = temp.path().join("reopened");
            signed_fixture_app(
                &target,
                "1.2.3",
                "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$2\" > \"$ASTRAL_TEST_UPDATE_ACK\"\n",
            );
            signed_fixture_app(&stage.join(APP), "1.2.4", "#!/bin/sh\nexit 1\n");
            assert!(!restart_fixture_with_game_probe(&target, &stage, &marker, probe).success());
            verify_bundle(&target, "1.2.3").unwrap();
            verify_bundle(&stage.join(APP), "1.2.4").unwrap();
            assert!(!stage.join("previous.app").exists());
            assert_reopened(&marker, reason);
        }
    }

    fn assert_reopened(marker: &Path, reason: &str) {
        for _ in 0..100 {
            if let Ok(arguments) = fs::read_to_string(marker)
                && arguments.ends_with('\n')
            {
                assert_eq!(
                    arguments.trim(),
                    format!("--astral-update-recovery {reason}")
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("The previous version was not reopened");
    }

    #[test]
    fn stale_acknowledgement_leaves_the_previous_bundle_installed() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join(APP);
        let stage = temp.path().join("stage");
        let ack = temp.path().join("ack");
        fs::write(&ack, "stale").unwrap();
        signed_fixture_app(
            &target,
            "1.2.3",
            "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$2\" > \"${ASTRAL_TEST_UPDATE_ACK}.recovery\"\n",
        );
        signed_fixture_app(&stage.join(APP), "1.2.4", "#!/bin/sh\nexit 1\n");
        assert!(!restart_fixture(&target, &stage, &ack).success());
        verify_bundle(&target, "1.2.3").unwrap();
        verify_bundle(&stage.join(APP), "1.2.4").unwrap();
        assert!(!stage.join("previous.app").exists());
        assert_eq!(fs::read_to_string(&ack).unwrap(), "stale");
        assert_reopened(&ack.with_extension("recovery"), "replacement-failed");
    }

    #[test]
    fn missing_replacement_restores_and_reopens_the_previous_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join(APP);
        let stage = temp.path().join("stage");
        let marker = temp.path().join("reopened");
        fs::create_dir(&stage).unwrap();
        signed_fixture_app(
            &target,
            "1.2.3",
            "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$2\" > \"$ASTRAL_TEST_UPDATE_ACK\"\n",
        );
        assert!(!restart_fixture(&target, &stage, &marker).success());
        verify_bundle(&target, "1.2.3").unwrap();
        assert_reopened(&marker, "replacement-failed");
    }

    #[test]
    fn signed_update_commits_or_rolls_back_after_startup() {
        use super::super::{BASE, Envelope, REPO, TAG_PREFIX, Target, verify};
        use base64::{Engine, engine::general_purpose::STANDARD};
        use ring::{
            rand::SystemRandom,
            signature::{Ed25519KeyPair, KeyPair},
        };
        use sha2::{Digest, Sha256};

        let target_platform = Target::current().unwrap();
        let version = semver::Version::new(1, 2, 4);
        let tag = format!("{TAG_PREFIX}{version}");
        let key = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
        for acknowledges in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let target = temp.path().join(APP);
            signed_fixture_app(
                &target,
                "1.2.3",
                "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$2\" > \"${ASTRAL_TEST_UPDATE_ACK}.recovery\"\n",
            );
            let source = tempfile::tempdir().unwrap();
            let script = if acknowledges {
                "#!/bin/sh\n[ \"$1\" = --astral-update-ack ] && : > \"$ASTRAL_TEST_UPDATE_ACK\"\n"
            } else {
                "#!/bin/sh\nexit 1\n"
            };
            signed_fixture_app(&source.path().join(APP), "1.2.4", script);
            let zip = zip_fixture(source.path());
            let payload = serde_json::to_vec(&serde_json::json!({
                "schema": 1, "repository": REPO, "app": "astral-native", "version": version.to_string(),
                "assets": { target_platform.key(): {
                    "url": format!("{BASE}{tag}/{}", target_platform.filename(&version)),
                    "sha256": format!("{:x}", Sha256::digest(&zip)),
                    "size": zip.len()
                }}
            })).unwrap();
            let release = verify(
                Envelope {
                    payload: STANDARD.encode(&payload),
                    signature: STANDARD.encode(key.sign(&payload).as_ref()),
                },
                key.public_key().as_ref(),
                &tag,
                target_platform,
            )
            .unwrap();
            let download = tempfile::NamedTempFile::new().unwrap();
            let mut changed = zip.clone();
            changed[0] ^= 1;
            fs::write(download.path(), changed).unwrap();
            assert!(prepare_update(&target, "1.2.3", &download, &release).is_err());
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
            fs::write(download.path(), zip).unwrap();
            let stage = prepare_update(&target, "1.2.3", &download, &release).unwrap();
            let ack = temp.path().join("ack");
            assert!(restart_fixture(&target, stage.path(), &ack).success());
            assert!(!ack.exists());
            if acknowledges {
                verify_bundle(&target, "1.2.4").unwrap();
                assert!(!stage.path().exists());
            } else {
                verify_bundle(&target, "1.2.3").unwrap();
                verify_bundle(&stage.path().join("failed.app"), "1.2.4").unwrap();
                assert_reopened(&ack.with_extension("recovery"), "rolled-back");
            }
        }
    }

    #[test]
    fn restart_helper_rolls_back_if_the_replacement_exits() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join(APP);
        let stage = temp.path().join("stage");
        let ack = temp.path().join("ack");
        let prior = "#!/bin/sh\nexit 0\n";
        let broken = "#!/bin/sh\nexit 1\n";
        fixture_app(&target, prior);
        fixture_app(&stage.join(APP), broken);
        // The sandbox cannot enumerate desktop processes. Supply a closed-game
        // result while testing the real helper's file swaps and child handling.
        let helper = include_str!("install-app-mac.sh").replace(
            "/usr/bin/pgrep -x 'Wow|World of Warcraft|WowClassic|WowF|WowForever'",
            "/usr/bin/false",
        );
        assert!(
            Command::new("/bin/sh")
                .arg("-c")
                .arg(helper)
                .arg("install")
                .arg("2147483647")
                .arg(&target)
                .arg(&stage)
                .arg(&ack)
                .arg("token")
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(fs::read_to_string(target.join(EXEC)).unwrap(), prior);
        assert_eq!(
            fs::read_to_string(stage.join("failed.app").join(EXEC)).unwrap(),
            broken
        );
    }

    #[test]
    fn restart_helper_keeps_a_replacement_that_acknowledges_startup() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join(APP);
        let stage = temp.path().join("stage");
        let ack = temp.path().join("ack");
        let prior = "#!/bin/sh\nexit 1\n";
        let next =
            "#!/bin/sh\n[ \"$1\" = --astral-update-ack ] && : > \"$ASTRAL_TEST_UPDATE_ACK\"\n";
        fixture_app(&target, prior);
        fixture_app(&stage.join(APP), next);
        let helper = include_str!("install-app-mac.sh").replace(
            "/usr/bin/pgrep -x 'Wow|World of Warcraft|WowClassic|WowF|WowForever'",
            "/usr/bin/false",
        );
        assert!(
            Command::new("/bin/sh")
                .arg("-c")
                .arg(helper)
                .arg("install")
                .arg("2147483647")
                .arg(&target)
                .arg(&stage)
                .arg(&ack)
                .arg("token")
                .env("ASTRAL_TEST_UPDATE_ACK", &ack)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(fs::read_to_string(target.join(EXEC)).unwrap(), next);
        assert!(!stage.exists());
        assert!(!ack.exists());
    }
}
