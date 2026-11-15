use super::Release;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_lite::future::block_on;
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const EXE: &str = "astral-native.exe";

fn installed_path() -> Option<PathBuf> {
    let directory = PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
        .join("Programs")
        .join("Astral Native");
    Some(directory.join(EXE))
}

pub(super) fn installed() -> bool {
    installed_path()
        .and_then(|path| path.canonicalize().ok())
        .is_some_and(|expected| {
            std::env::current_exe()
                .and_then(|path| path.canonicalize())
                .is_ok_and(|path| path == expected)
        })
}

pub(super) fn launch(
    file: &tempfile::NamedTempFile,
    release: &Release,
    data: &Path,
) -> Result<(), String> {
    if !installed() {
        return Err(
            "Install Astral Native under your local Programs folder before updating.".into(),
        );
    }
    let target = installed_path().ok_or("The Astral installation could not be found.")?;
    let stage = prepare_update(&target, file, release)?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let ack = super::ack_path(data, &token).ok_or("Couldn't prepare update acknowledgement.")?;
    let powershell = powershell_path()?;
    crate::installer::game_closed()?;
    install_command(
        &powershell,
        std::process::id(),
        &target,
        stage.path(),
        &ack,
        &token,
    )
    .creation_flags(0x0800_0000)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|error| format!("Couldn't start the Windows updater. {error}"))?;
    let _ = stage.keep();
    Ok(())
}

fn powershell_path() -> Result<PathBuf, String> {
    Ok(PathBuf::from(
        std::env::var_os("SystemRoot").ok_or("Windows PowerShell could not be found.")?,
    )
    .join("System32/WindowsPowerShell/v1.0/powershell.exe"))
}

fn prepare_update(
    target: &Path,
    file: &tempfile::NamedTempFile,
    release: &Release,
) -> Result<tempfile::TempDir, String> {
    let parent = target
        .parent()
        .ok_or("The Astral installation could not be found.")?;
    let stage = tempfile::Builder::new()
        .prefix(".astral-native-update-")
        .tempdir_in(parent)
        .map_err(|error| format!("Couldn't stage the update beside Astral. {error}"))?;
    block_on(super::zip::extract(
        super::verified_bytes(file, release)?,
        stage.path(),
        |name, is_dir| !is_dir && name == EXE,
        EXE,
    ))?;
    Ok(stage)
}

fn install_command(
    powershell: &Path,
    parent: u32,
    target: &Path,
    stage: &Path,
    ack: &Path,
    token: &str,
) -> Command {
    let script: Vec<_> = include_str!("install-app-windows.ps1")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut command = Command::new(powershell);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
        ])
        .arg(STANDARD.encode(script))
        .env("ASTRAL_UPDATE_PARENT", parent.to_string())
        .env("ASTRAL_UPDATE_TARGET", target)
        .env("ASTRAL_UPDATE_STAGE", stage)
        .env("ASTRAL_UPDATE_ACK", ack)
        .env("ASTRAL_UPDATE_TOKEN", token);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_updates::{BASE, Envelope, REPO, TAG_PREFIX, Target, verify};
    use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};
    use ring::{
        rand::SystemRandom,
        signature::{Ed25519KeyPair, KeyPair},
    };
    use sha2::{Digest, Sha256};
    use std::fs;

    fn fixture_executable(output: &Path, source: &str) {
        let result = Command::new(powershell_path().unwrap())
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(
                "$ErrorActionPreference = 'Stop'; Add-Type -TypeDefinition $env:ASTRAL_TEST_SOURCE -OutputAssembly $env:ASTRAL_TEST_EXE -OutputType ConsoleApplication",
            )
            .env("ASTRAL_TEST_SOURCE", source)
            .env("ASTRAL_TEST_EXE", output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    fn sign_fixture(bytes: &[u8], version: semver::Version) -> Release {
        let target = Target::WindowsX64;
        let tag = format!("{TAG_PREFIX}{version}");
        let key = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
        let payload = serde_json::to_vec(&serde_json::json!({
            "schema": 1, "repository": REPO, "app": "astral-native", "version": version.to_string(),
            "assets": { target.key(): {
                "url": format!("{BASE}{tag}/{}", target.filename(&version)),
                "sha256": format!("{:x}", Sha256::digest(bytes)),
                "size": bytes.len()
            }}
        }))
        .unwrap();
        verify(
            Envelope {
                payload: STANDARD.encode(&payload),
                signature: STANDARD.encode(key.sign(&payload).as_ref()),
            },
            key.public_key().as_ref(),
            &tag,
            target,
        )
        .unwrap()
    }

    #[test]
    #[ignore = "requires a release ZIP built by native/package-windows.ps1"]
    fn windows_release_archive_extracts_verifies_and_runs() {
        use std::io::{Read, Seek, SeekFrom};

        let archive = std::env::var_os("ASTRAL_TEST_WINDOWS_ARCHIVE")
            .expect("set ASTRAL_TEST_WINDOWS_ARCHIVE to the release ZIP");
        let bytes = fs::read(archive).unwrap();
        let version = super::super::stable(super::super::CURRENT).unwrap();
        let release = sign_fixture(&bytes, version);
        let download = tempfile::NamedTempFile::new().unwrap();
        fs::write(download.path(), bytes).unwrap();
        let installation = tempfile::tempdir().unwrap();
        let stage = prepare_update(&installation.path().join(EXE), &download, &release).unwrap();
        let executable = stage.path().join(EXE);
        {
            let mut file = fs::File::open(&executable).unwrap();
            let mut dos_header = [0; 64];
            file.read_exact(&mut dos_header).unwrap();
            assert_eq!(&dos_header[..2], b"MZ");
            let pe_offset = u32::from_le_bytes(dos_header[60..64].try_into().unwrap());
            assert!(pe_offset >= 64);
            file.seek(SeekFrom::Start(u64::from(pe_offset))).unwrap();
            let mut header = [0; 26];
            file.read_exact(&mut header).unwrap();
            assert_eq!(&header[..4], b"PE\0\0");
            assert_eq!(
                u16::from_le_bytes([header[4], header[5]]),
                0x8664,
                "expected AMD64"
            );
            assert_eq!(
                u16::from_le_bytes([header[24], header[25]]),
                0x20b,
                "expected PE32+"
            );
        }
        let output = Command::new(&executable).arg("--version").output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            format!("Astral Native {}", release.version),
        );
        super::super::assert_packaged_fixture_scan(&executable);
    }

    #[test]
    fn signed_windows_update_commits_or_restores_the_previous_executable() {
        crate::installer::game_closed().expect("close WoW before running updater tests");
        let workspace = tempfile::tempdir().unwrap();
        let prior = workspace.path().join("prior.exe");
        let next = workspace.path().join("next.exe");
        fixture_executable(
            &prior,
            r#"using System; using System.IO;
               public static class Astral {
                   public static int Main(string[] args) {
                       if (args.Length == 2 && args[0] == "--astral-update-recovery")
                           File.WriteAllText(Environment.GetEnvironmentVariable("ASTRAL_UPDATE_ACK") + ".recovery", args[1]);
                       return 0;
                   }
               }"#,
        );
        fixture_executable(
            &next,
            r#"using System; using System.IO;
               public static class Astral {
                   public static int Main(string[] args) {
                       if (args.Length != 2 || args[0] != "--astral-update-ack" ||
                           args[1] != Environment.GetEnvironmentVariable("ASTRAL_UPDATE_TOKEN") ||
                           Environment.GetEnvironmentVariable("ASTRAL_TEST_ACKNOWLEDGE") != "1") return 1;
                       File.WriteAllText(Environment.GetEnvironmentVariable("ASTRAL_UPDATE_ACK"), "");
                       return 0;
                   }
               }"#,
        );
        let next_bytes = fs::read(&next).unwrap();
        let old_bytes = fs::read(&prior).unwrap();
        let zip = block_on(async {
            let mut bytes = Vec::new();
            let mut writer = ZipFileWriter::new(&mut bytes);
            writer
                .write_entry_whole(
                    ZipEntryBuilder::new(EXE.into(), Compression::Deflate),
                    &next_bytes,
                )
                .await
                .unwrap();
            writer.close().await.unwrap();
            bytes
        });
        let release = sign_fixture(&zip, semver::Version::new(1, 2, 4));
        let powershell = powershell_path().unwrap();
        for (acknowledges, missing_replacement) in [(true, false), (false, false), (false, true)] {
            let installation = tempfile::Builder::new()
                .prefix("Astral test ")
                .tempdir_in(workspace.path())
                .unwrap();
            let target = installation.path().join(EXE);
            fs::copy(&prior, &target).unwrap();
            let download = tempfile::NamedTempFile::new().unwrap();
            let mut changed = zip.clone();
            changed[0] ^= 1;
            fs::write(download.path(), changed).unwrap();
            assert!(prepare_update(&target, &download, &release).is_err());
            assert_eq!(fs::read_dir(installation.path()).unwrap().count(), 1);
            fs::write(download.path(), &zip).unwrap();
            let stage = prepare_update(&target, &download, &release).unwrap();
            if missing_replacement {
                fs::remove_file(stage.path().join(EXE)).unwrap();
            }
            let mut parent = Command::new(&prior).spawn().unwrap();
            let parent_id = parent.id();
            assert!(parent.wait().unwrap().success());
            let ack = installation.path().join("ack");
            let result = install_command(
                &powershell,
                parent_id,
                &target,
                stage.path(),
                &ack,
                "test-token",
            )
            .env(
                "ASTRAL_TEST_ACKNOWLEDGE",
                if acknowledges { "1" } else { "0" },
            )
            .output()
            .unwrap();
            assert_eq!(
                result.status.success(),
                !missing_replacement,
                "{}\n{}",
                String::from_utf8_lossy(&result.stderr),
                fs::read_to_string(stage.path().join("install.log")).unwrap_or_default(),
            );
            assert!(!ack.exists());
            let actual = fs::read(&target).unwrap();
            if acknowledges {
                assert_eq!(actual, next_bytes);
                assert!(!stage.path().exists());
            } else if missing_replacement {
                assert_eq!(actual, old_bytes);
                assert!(!stage.path().join("failed.exe").exists());
                assert!(!stage.path().join("previous.exe").exists());
                assert_reopened(&ack, "replacement-failed");
            } else {
                assert_eq!(actual, old_bytes);
                assert_eq!(
                    fs::read(stage.path().join("failed.exe")).unwrap(),
                    next_bytes
                );
                assert!(
                    fs::read_to_string(stage.path().join("install.log"))
                        .unwrap()
                        .contains("Restored the previous version.")
                );
                assert_reopened(&ack, "rolled-back");
            }
        }
    }

    fn assert_reopened(ack: &Path, expected: &str) {
        let marker = ack.with_extension("recovery");
        for _ in 0..100 {
            if fs::read_to_string(&marker).is_ok_and(|reason| reason == expected) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("The previous version did not reopen with {expected}");
    }
}
