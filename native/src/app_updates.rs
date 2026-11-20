use base64::{Engine, engine::general_purpose::STANDARD};
use ring::signature::{ED25519, UnparsedPublicKey};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

const REPO: &str = "theedward/astral";
const BASE: &str = "https://github.com/theedward/astral/releases/download/";
const PUBLIC_KEY: &str = include_str!("../../desktop/update-public-key.pem");
const MANIFEST_NAME: &str = "astral-native-update.json";
const TAG_PREFIX: &str = "native-v";
const MAX_SIZE: u64 = 600 * 1024 * 1024;
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[cfg(target_os = "macos")]
#[path = "app_update_mac.rs"]
mod mac;

#[cfg(target_os = "windows")]
#[path = "app_update_windows.rs"]
mod windows;

#[cfg(any(target_os = "macos", target_os = "windows", test))]
#[path = "app_update_zip.rs"]
mod zip;

fn ack_path(data: &Path, token: &str) -> Option<std::path::PathBuf> {
    (token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| data.join("app-updates").join(format!("ack-{token}")))
}

pub fn acknowledge(token: &str) {
    if let Some(path) = crate::preferences::path()
        .as_deref()
        .and_then(Path::parent)
        .and_then(|data| ack_path(data, token))
        && let Err(error) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    {
        eprintln!("Couldn't acknowledge the Astral update. {error}");
    }
}

fn install_supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::installed()
    }
    #[cfg(target_os = "windows")]
    {
        windows::installed()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

#[derive(Clone, Copy)]
enum Target {
    MacArm64,
    MacX64,
    WindowsX64,
}

impl Target {
    fn current() -> Result<Self, String> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Ok(Self::MacArm64),
            ("macos", "x86_64") => Ok(Self::MacX64),
            ("windows", "x86_64") => Ok(Self::WindowsX64),
            _ => Err("Astral updates are unavailable for this computer.".into()),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::MacArm64 => "darwin-arm64",
            Self::MacX64 => "darwin-x64",
            Self::WindowsX64 => "win32-x64",
        }
    }

    fn filename(self, version: &Version) -> String {
        let suffix = match self {
            Self::MacArm64 => "mac-arm64.zip",
            Self::MacX64 => "mac-x64.zip",
            Self::WindowsX64 => "win-x64.zip",
        };
        format!("Astral-Native-{version}-{suffix}")
    }
}

#[derive(Deserialize)]
struct Envelope {
    payload: String,
    signature: String,
}

#[derive(Deserialize)]
struct Manifest {
    schema: u32,
    repository: String,
    app: String,
    version: String,
    #[serde(default)]
    notes: String,
    assets: HashMap<String, Asset>,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
struct Asset {
    url: String,
    sha256: String,
    size: u64,
}

#[derive(Clone)]
pub struct Release {
    pub version: Version,
    pub notes: String,
    asset: Asset,
}

#[derive(Deserialize)]
struct PublishedRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<PublishedAsset>,
}

fn native_release_tag(releases: &[PublishedRelease]) -> Option<&str> {
    releases
        .iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter(|release| {
            release
                .assets
                .iter()
                .any(|asset| asset.name == MANIFEST_NAME && asset.state == "uploaded")
        })
        .filter_map(|release| {
            let version = stable(release.tag_name.strip_prefix(TAG_PREFIX)?).ok()?;
            Some((version, release.tag_name.as_str()))
        })
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, tag)| tag)
}

#[derive(Deserialize)]
struct PublishedAsset {
    name: String,
    state: String,
}

pub enum Check {
    Unpublished,
    Current(Release),
    Available(Release),
}

fn stable(value: &str) -> Result<Version, String> {
    let version = Version::parse(value).map_err(|_| "Invalid Astral version.")?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err("Invalid stable Astral version.".into());
    }
    Ok(version)
}

fn public_key() -> Result<Vec<u8>, String> {
    let encoded = PUBLIC_KEY
        .lines()
        .find(|line| !line.starts_with("-----"))
        .ok_or("Invalid Astral update key.")?;
    let der = STANDARD
        .decode(encoded)
        .map_err(|_| "Invalid Astral update key.")?;
    const ED25519_SPKI: &[u8] = &[
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    der.strip_prefix(ED25519_SPKI)
        .filter(|key| key.len() == 32)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| "Invalid Astral update key.".into())
}

fn verify(envelope: Envelope, key: &[u8], tag: &str, target: Target) -> Result<Release, String> {
    if envelope.payload.len() > 100_000 || envelope.signature.len() > 128 {
        return Err("Invalid update manifest.".into());
    }
    let bytes = STANDARD
        .decode(envelope.payload)
        .map_err(|_| "Invalid update manifest.")?;
    let signature = STANDARD
        .decode(envelope.signature)
        .map_err(|_| "Invalid update signature.")?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&bytes, &signature)
        .map_err(|_| "The update signature could not be verified.")?;
    let mut manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid signed update manifest.")?;
    if manifest.schema != 1 || manifest.repository != REPO || manifest.app != "astral-native" {
        return Err("Unexpected update publisher or application.".into());
    }
    let version = stable(&manifest.version)?;
    if tag != format!("{TAG_PREFIX}{version}") {
        return Err("Release metadata does not match its tag.".into());
    }
    let asset = manifest
        .assets
        .remove(target.key())
        .ok_or("There is no update for this computer yet.")?;
    if asset.url != format!("{BASE}{tag}/{}", target.filename(&version))
        || !(1..=MAX_SIZE).contains(&asset.size)
        || asset.sha256.len() != 64
        || !asset
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Invalid update download.".into());
    }
    Ok(Release {
        version,
        notes: manifest.notes.chars().take(20_000).collect(),
        asset,
    })
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(3)
        .timeout_global(Some(Duration::from_secs(timeout)))
        .build()
        .into()
}

pub fn check() -> Result<Check, String> {
    let target = Target::current()?;
    let agent = agent(20);
    let mut response = match agent
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases?per_page=100"
        ))
        .header("User-Agent", "Astral-Native")
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(Check::Unpublished),
        Err(error) => return Err(format!("Couldn't check Astral updates. {error}")),
    };
    let releases: Vec<PublishedRelease> = response
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_json()
        .map_err(|error| format!("Couldn't read Astral release. {error}"))?;
    let Some(tag) = native_release_tag(&releases) else {
        return Ok(Check::Unpublished);
    };
    let envelope = agent
        .get(format!("{BASE}{tag}/{MANIFEST_NAME}"))
        .header("User-Agent", "Astral-Native")
        .call()
        .map_err(|error| format!("Couldn't download the update manifest. {error}"))?
        .body_mut()
        .with_config()
        .limit(150_000)
        .read_json()
        .map_err(|error| format!("Couldn't read the update manifest. {error}"))?;
    let release = verify(envelope, &public_key()?, tag, target)?;
    if release.version > stable(CURRENT)? {
        Ok(Check::Available(release))
    } else {
        Ok(Check::Current(release))
    }
}

fn download(
    release: &Release,
    directory: &Path,
    progress: impl FnMut(u64),
) -> Result<tempfile::NamedTempFile, String> {
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let mut file = tempfile::Builder::new()
        .prefix("astral-native-")
        .suffix(".zip")
        .tempfile_in(directory)
        .map_err(|error| error.to_string())?;
    let mut response = agent(600)
        .get(&release.asset.url)
        .header("User-Agent", "Astral-Native")
        .header("Accept-Encoding", "identity")
        .call()
        .map_err(|error| format!("Couldn't download Astral. {error}"))?;
    stream_verified_bytes(
        response
            .body_mut()
            .with_config()
            .limit(release.asset.size + 1)
            .reader(),
        &mut file,
        &release.asset,
        progress,
    )?;
    file.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    Ok(file)
}

fn verify_bytes(input: impl Read, output: impl Write, asset: &Asset) -> Result<(), String> {
    stream_verified_bytes(input, output, asset, |_| {})
}

fn stream_verified_bytes(
    mut input: impl Read,
    mut output: impl Write,
    asset: &Asset,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let mut hash = Sha256::new();
    let mut count = 0;
    let mut buffer = [0; 32 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > asset.size {
            return Err("The update is larger than expected.".into());
        }
        hash.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
        progress(count);
    }
    if count != asset.size || format!("{:x}", hash.finalize()) != asset.sha256 {
        return Err("The update download failed integrity verification.".into());
    }
    Ok(())
}

pub fn validate_staged(file: &tempfile::NamedTempFile, release: &Release) -> Result<(), String> {
    let input = file.reopen().map_err(|error| error.to_string())?;
    verify_bytes(input, io::sink(), &release.asset)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn verified_bytes(file: &tempfile::NamedTempFile, release: &Release) -> Result<Vec<u8>, String> {
    let input = file.reopen().map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    verify_bytes(input, &mut bytes, &release.asset)?;
    Ok(bytes)
}

#[cfg(test)]
fn assert_packaged_fixture_scan(executable: &Path) {
    let fixture = tempfile::tempdir().unwrap();
    for (client, toc, version) in [
        (crate::Client::Retail, "KeyUI_Mainline.toc", "1.2.3"),
        (crate::Client::Forever, "KeyUI_Camelot.toc", "2.3.4"),
    ] {
        let addon = fixture
            .path()
            .join(client.folder())
            .join("Interface/AddOns/KeyUI");
        std::fs::create_dir_all(&addon).unwrap();
        std::fs::write(addon.join(toc), format!("## Version: {version}\n")).unwrap();
        let manual = addon.with_file_name("ManualFixture");
        std::fs::create_dir_all(&manual).unwrap();
        std::fs::write(manual.join("Z.toc"), "## Version: wrong\n").unwrap();
        std::fs::write(manual.join("A.toc"), "## Version: fallback\n").unwrap();
        std::fs::create_dir_all(addon.with_file_name("IncompleteAddon")).unwrap();
        std::fs::create_dir_all(addon.with_file_name("TomTom")).unwrap();
    }
    let output = std::process::Command::new(executable)
        .arg("--ui-fixture")
        .arg(fixture.path())
        .arg("--scan")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<serde_json::Value> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    for (row, (client, version)) in rows.iter().zip([("Retail", "1.2.3"), ("Forever", "2.3.4")]) {
        assert_eq!(row["client"], client);
        let addons = row["addons"].as_array().unwrap();
        assert_eq!(addons.len(), 3);
        assert_eq!(addons[0]["folder"], "TomTom");
        assert_eq!(addons[0]["version"], "Unknown");
        assert_eq!(addons[1]["folder"], "KeyUI");
        assert_eq!(addons[1]["version"], version);
        assert_eq!(addons[2]["folder"], "ManualFixture");
        assert_eq!(addons[2]["version"], "fallback");
    }
    if let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") {
        let electron = std::process::Command::new(node)
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
            .args(["-e", r#"
                const engine = require('./desktop/engine.cjs');
                const path = require('node:path');
                (async () => {
                    for (const [client, folder] of [['Retail', '_retail_'], ['Forever', '_forever_']]) {
                        const { addons } = await engine.inventory(path.join(process.argv[1], folder));
                        console.log(JSON.stringify({ client, addons }));
                    }
                })().catch(error => { console.error(error); process.exitCode = 1; });
            "#])
            .arg(fixture.path())
            .output()
            .unwrap();
        assert!(
            electron.status.success(),
            "{}",
            String::from_utf8_lossy(&electron.stderr)
        );
        let inventory = |bytes: &[u8]| -> Vec<_> {
            std::str::from_utf8(bytes)
                .unwrap()
                .lines()
                .map(|line| {
                    let row: serde_json::Value = serde_json::from_str(line).unwrap();
                    let addons: Vec<_> = row["addons"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|addon| {
                            serde_json::json!([
                                addon["folder"],
                                addon["name"],
                                addon["version"],
                                addon["source"],
                                addon["modules"]
                            ])
                        })
                        .collect();
                    (row["client"].clone(), addons)
                })
                .collect()
        };
        assert_eq!(inventory(&output.stdout), inventory(&electron.stdout));
    }
    assert!(!fixture.path().join("preferences.json").exists());
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::{
        rand::SystemRandom,
        signature::{Ed25519KeyPair, KeyPair},
    };
    use serde_json::json;

    #[test]
    fn native_release_selection_skips_electron_and_uses_highest_stable_version() {
        let item = |tag: &str, draft, prerelease, state: &str| {
            json!({
                "tag_name": tag, "draft": draft, "prerelease": prerelease,
                "assets": [{ "name": MANIFEST_NAME, "state": state }]
            })
        };
        let releases: Vec<PublishedRelease> = serde_json::from_value(json!([
            item("v12.0.0", false, false, "uploaded"),
            item("native-v1.9.0", false, false, "uploaded"),
            item("native-v2.0.0", true, false, "uploaded"),
            item("native-v3.0.0", false, true, "uploaded"),
            item("native-v4.0.0", false, false, "new"),
            item("native-v5.0.0-beta.1", false, false, "uploaded"),
            item("native-v1.10.0", false, false, "uploaded")
        ]))
        .unwrap();
        assert_eq!(native_release_tag(&releases), Some("native-v1.10.0"));
        assert_eq!(native_release_tag(&releases[..1]), None);
    }

    #[test]
    fn signed_native_manifest_rejects_tampering_and_electron_packages() {
        let key = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
        let manifest = json!({
            "schema": 1, "repository": REPO, "app": "astral-native", "version": "1.2.3",
            "assets": {
                "darwin-arm64": {
                    "url": format!("{BASE}native-v1.2.3/Astral-Native-1.2.3-mac-arm64.zip"),
                    "sha256": "a".repeat(64), "size": 3
                }
            }
        });
        let signed = |manifest: &serde_json::Value| {
            let bytes = serde_json::to_vec(manifest).unwrap();
            Envelope {
                signature: STANDARD.encode(key.sign(&bytes).as_ref()),
                payload: STANDARD.encode(bytes),
            }
        };
        let public = key.public_key().as_ref();
        assert_eq!(
            verify(signed(&manifest), public, "native-v1.2.3", Target::MacArm64)
                .unwrap()
                .version
                .to_string(),
            "1.2.3"
        );
        assert!(verify(signed(&manifest), public, "native-v1.2.4", Target::MacArm64).is_err());
        assert!(
            verify(
                signed(&manifest),
                public,
                "native-v1.2.3",
                Target::WindowsX64
            )
            .is_err()
        );
        let mut wrong = manifest.clone();
        wrong["assets"]["darwin-arm64"]["url"] =
            json!(format!("{BASE}native-v1.2.3/Astral-1.2.3-mac-arm64.zip"));
        assert!(verify(signed(&wrong), public, "native-v1.2.3", Target::MacArm64).is_err());
        let mut tampered = signed(&manifest);
        tampered.payload = STANDARD.encode(serde_json::to_vec(&wrong).unwrap());
        assert!(verify(tampered, public, "native-v1.2.3", Target::MacArm64).is_err());
        assert_eq!(public_key().unwrap().len(), 32);
    }

    #[test]
    fn streamed_update_reports_bytes_and_rejects_truncation_extra_bytes_and_wrong_hash() {
        let bytes = b"verified zip";
        let asset = Asset {
            url: String::new(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            size: bytes.len() as u64,
        };
        let mut staged = Vec::new();
        let mut progress = Vec::new();
        stream_verified_bytes(bytes.as_slice(), &mut staged, &asset, |count| {
            progress.push(count);
        })
        .unwrap();
        assert_eq!(staged, bytes);
        assert_eq!(progress.last(), Some(&asset.size));
        for wrong in [
            &b"verified zi"[..],
            &b"verified zip!"[..],
            &b"Verified zip"[..],
        ] {
            assert!(verify_bytes(wrong, io::sink(), &asset).is_err());
        }
    }

    #[test]
    fn recheck_keeps_staged_bytes_only_for_the_same_verified_package_or_on_check_failure() {
        let release = Release {
            version: Version::new(1, 2, 3),
            notes: "Old notes".into(),
            asset: Asset {
                url: "https://example.invalid/update.zip".into(),
                sha256: "a".repeat(64),
                size: 42,
            },
        };
        let staged = || Some((release.clone(), tempfile::NamedTempFile::new().unwrap()));
        let mut latest = release.clone();
        latest.notes = "Updated notes".into();
        let (phase, error) = checked_phase(Ok(Check::Available(latest.clone())), staged());
        assert!(matches!(phase, Phase::Staged(release, _) if release.notes == latest.notes));
        assert!(error.is_none());

        let (phase, error) = checked_phase(Err("Check failed".into()), staged());
        assert!(matches!(phase, Phase::Staged(..)));
        assert_eq!(error.as_deref(), Some("Check failed"));

        latest.asset.sha256 = "b".repeat(64);
        let (phase, _) = checked_phase(Ok(Check::Available(latest)), staged());
        assert!(matches!(phase, Phase::Available(_)));
        let (phase, _) = checked_phase(Ok(Check::Unpublished), staged());
        assert!(matches!(phase, Phase::Unpublished));
    }
}

use crate::{Astral, Page, Palette, icon};
use gpui::{Context, div, prelude::*, px, rgb};
use std::time::Instant;

#[derive(Default)]
pub struct State {
    phase: Phase,
    last_check: Option<Instant>,
    notes_open: bool,
    check_error: Option<String>,
    install_error: Option<String>,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Checking,
    Unpublished,
    Current(Release),
    Available(Release),
    Downloading(Arc<AtomicU8>),
    Installing,
    Staged(Release, tempfile::NamedTempFile),
    Error(String),
}

impl State {
    pub(crate) fn show_recovery(&mut self, reason: &str) -> bool {
        let message = match reason {
            "wow-running" => "WoW opened during the update. Close it and try again.",
            "game-check-failed" => "Astral couldn’t check whether WoW was running. Try again.",
            "replacement-failed" => {
                "The update couldn’t be installed. Astral reopened the previous version."
            }
            "rolled-back" => "The update couldn’t start. Astral restored the previous version.",
            _ => return false,
        };
        self.install_error = Some(message.into());
        true
    }

    pub(crate) fn installing(&self) -> bool {
        matches!(self.phase, Phase::Installing)
    }

    fn busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Checking | Phase::Downloading(_) | Phase::Installing
        )
    }
}

fn checked_phase(
    result: Result<Check, String>,
    staged: Option<(Release, tempfile::NamedTempFile)>,
) -> (Phase, Option<String>) {
    match result {
        Ok(Check::Unpublished) => (Phase::Unpublished, None),
        Ok(Check::Current(release)) => (Phase::Current(release), None),
        Ok(Check::Available(release)) => {
            let phase = match staged {
                Some((prior, file))
                    if prior.version == release.version && prior.asset == release.asset =>
                {
                    Phase::Staged(release, file)
                }
                _ => Phase::Available(release),
            };
            (phase, None)
        }
        Err(error) => match staged {
            Some((release, file)) => (Phase::Staged(release, file), Some(error)),
            None => (Phase::Error(error), None),
        },
    }
}

impl Astral {
    pub fn app_update_notice(&self, palette: Palette, cx: &Context<Self>) -> Option<gpui::Div> {
        let (Phase::Available(release) | Phase::Staged(release, _)) = &self.app_updates.phase
        else {
            return None;
        };
        Some(
            div()
                .relative()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(20.))
                .py(px(24.))
                .mb(px(24.))
                .border_t_1()
                .border_color(rgb(palette.text))
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(px(1.))
                        .bg(palette.line),
                )
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(16.))
                        .child(format!("Astral {} is available", release.version)),
                )
                .child(crate::outlined_button(
                    div()
                        .id("view-app-update")
                        .debug_selector(|| "view-app-update".into())
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(rgb(palette.accent))
                        .child("View update")
                        .child(icon("arrow-right", 14., palette.accent)),
                    palette,
                    &self.settings_focus.update_banner,
                    cx.listener(|app, _: &(), window, cx| app.navigate(Page::Settings, window, cx)),
                )),
        )
    }

    pub fn start_app_update_timer(&self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_secs(3)).await;
            loop {
                if this
                    .update(cx, |app, cx| {
                        if app.preferences.app_update_checks
                            && app
                                .app_updates
                                .last_check
                                .is_none_or(|last| last.elapsed() >= Duration::from_secs(86_400))
                        {
                            app.check_app_update(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
                executor.timer(Duration::from_secs(60)).await;
            }
        })
        .detach();
    }

    fn check_app_update(&mut self, cx: &mut Context<Self>) {
        if self.fixture || self.app_updates.busy() {
            return;
        }
        self.app_updates.last_check = Some(Instant::now());
        self.app_updates.check_error = None;
        let staged = match std::mem::replace(&mut self.app_updates.phase, Phase::Checking) {
            Phase::Staged(release, file) => Some((release, file)),
            _ => None,
        };
        let task = cx.background_executor().spawn(async { check() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                let (phase, error) = checked_phase(result, staged);
                app.app_updates.phase = phase;
                app.app_updates.check_error = error;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn stage_app_update(&mut self, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        let Phase::Available(release) = &self.app_updates.phase else {
            return;
        };
        let Some(data) = self.preferences_path.as_deref().and_then(Path::parent) else {
            return;
        };
        let release = release.clone();
        let directory = data.join("app-updates");
        let progress = Arc::new(AtomicU8::new(0));
        self.app_updates.phase = Phase::Downloading(progress.clone());
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let mut displayed = 0;
            loop {
                executor.timer(Duration::from_millis(150)).await;
                if !this
                    .update(cx, |app, cx| {
                        let Phase::Downloading(progress) = &app.app_updates.phase else {
                            return false;
                        };
                        let percent = progress.load(Ordering::Relaxed);
                        if displayed != percent {
                            displayed = percent;
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        })
        .detach();
        let task = cx.background_executor().spawn(async move {
            let file = download(&release, &directory, |bytes| {
                progress.store((bytes * 100 / release.asset.size) as u8, Ordering::Relaxed);
            })?;
            validate_staged(&file, &release)?;
            Ok::<_, String>((release, file))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.app_updates.phase = match result {
                    Ok((release, file)) => Phase::Staged(release, file),
                    Err(error) => Phase::Error(error),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn can_install_app_update(&self) -> bool {
        !self.fixture
            && install_supported()
            && !self.profile_game_running()
            && !self.daily_running
            && !self.account_work_in_progress()
            && !self.sync_dialog_open()
            && !self.settings_write_in_progress()
            && !self.talent_refreshing
            && self.batch.is_none()
            && self.release_review.is_none()
            && self.removal.is_none()
            && self.project_review.is_none()
            && !self.activity.has_confirmation()
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn install_app_update(&mut self, cx: &mut Context<Self>) {
        if !self.can_install_app_update() {
            return;
        }
        let Some(data) = self
            .preferences_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
        else {
            return;
        };
        if !matches!(self.app_updates.phase, Phase::Staged(..)) {
            return;
        }
        let Phase::Staged(release, file) =
            std::mem::replace(&mut self.app_updates.phase, Phase::Installing)
        else {
            unreachable!()
        };
        self.app_updates.install_error = None;
        let task = cx.background_executor().spawn(async move {
            #[cfg(target_os = "macos")]
            let result = mac::launch(&file, &release, &data);
            #[cfg(target_os = "windows")]
            let result = windows::launch(&file, &release, &data);
            (result, release, file)
        });
        cx.spawn(async move |this, cx| {
            let (result, release, file) = task.await;
            let _ = this.update(cx, |app, cx| match result {
                Ok(()) => cx.quit(),
                Err(error) => {
                    app.app_updates.phase = Phase::Staged(release, file);
                    app.app_updates.install_error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn install_app_update(&mut self, _: &mut Context<Self>) {}

    pub fn app_update_settings(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let state = &self.app_updates;
        let enabled = self.preferences.app_update_checks;
        let (message, release) = match &state.phase {
            Phase::Idle => (String::new(), None),
            Phase::Checking => ("Checking for a new version…".into(), None),
            Phase::Unpublished => ("A native release hasn’t been published yet.".into(), None),
            Phase::Current(release) => ("You’re up to date.".into(), Some(release)),
            Phase::Available(release) => (
                format!("Astral {} is available.", release.version),
                Some(release),
            ),
            Phase::Downloading(progress) => (
                format!("Downloading… {}%", progress.load(Ordering::Relaxed)),
                None,
            ),
            Phase::Installing => ("Preparing to restart…".into(), None),
            Phase::Staged(release, _file) => (
                format!("Astral {} is ready to install.", release.version),
                Some(release),
            ),
            Phase::Error(error) => (error.clone(), None),
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(palette.label(format!("Version {CURRENT}")))
            .when(
                matches!(state.phase, Phase::Staged(..)) && !install_supported(),
                |view| {
                    view.child(palette.label(if cfg!(target_os = "windows") {
                        "Install Astral Native in your local Programs folder to update and restart."
                    } else {
                        "Open the packaged Astral Native app to update and restart."
                    }))
                },
            )
            .when_some(state.install_error.as_ref(), |view, error| {
                view.child(palette.label(error.clone()).text_color(rgb(palette.error)))
            })
            .when_some(state.check_error.as_ref(), |view, error| {
                view.child(palette.label(error.clone()).text_color(rgb(palette.error)))
            })
            .child(palette.setting_checkbox(
                "app-update-auto",
                "Check daily",
                enabled,
                !self.fixture && !state.busy() && self.preferences_path.is_some(),
                |app, cx| {
                    let previous = app.preferences.app_update_checks;
                    app.preferences.app_update_checks = !previous;
                    app.save_preferences();
                    if app.preferences_error.is_some() {
                        app.preferences.app_update_checks = previous;
                    }
                    cx.notify();
                },
                cx,
            ))
            .when(!message.is_empty(), |view| {
                view.child(
                    palette
                        .label(message)
                        .when(matches!(state.phase, Phase::Error(_)), |label| {
                            label.text_color(rgb(palette.error))
                        }),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child({
                        let button = palette.button("check-app-update", "Check for updates");
                        if self.fixture || state.busy() {
                            button.opacity(0.45).cursor_default()
                        } else {
                            crate::outlined_button(
                                button,
                                palette,
                                &self.settings_focus.update_check,
                                cx.listener(|app, _: &(), _, cx| {
                                    app.check_app_update(cx);
                                }),
                            )
                        }
                    })
                    .when(
                        matches!(state.phase, Phase::Staged(..)) && install_supported(),
                        |row| {
                            let ready = self.can_install_app_update();
                            row.child(
                                palette
                                    .primary_button("restart-for-update", "Update & restart")
                                    .when(!ready, |button| button.opacity(0.45).cursor_default())
                                    .when(ready, |button| {
                                        crate::outlined_button(
                                            button,
                                            palette,
                                            &self.settings_focus.update_restart,
                                            cx.listener(|app, _: &(), _, cx| {
                                                app.install_app_update(cx);
                                            }),
                                        )
                                    }),
                            )
                        },
                    )
                    .when(
                        matches!(state.phase, Phase::Available(_))
                            && self.preferences_path.is_some(),
                        |row| {
                            row.child(crate::outlined_button(
                                palette.primary_button("download-app-update", "Download update"),
                                palette,
                                &self.settings_focus.update_download,
                                cx.listener(|app, _: &(), _, cx| app.stage_app_update(cx)),
                            ))
                        },
                    ),
            )
            .when(
                matches!(state.phase, Phase::Staged(..)) && self.profile_game_running(),
                |view| view.child(palette.label("Close WoW before installing the update.")),
            )
            .when_some(
                release.filter(|release| !release.notes.is_empty()),
                |view, release| {
                    view.child(
                        div().mt(px(11.)).child(crate::outlined_button(
                            div()
                                .id("app-release-notes")
                                .debug_selector(|| "app-release-notes".into())
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .cursor_pointer()
                                .text_size(px(11.))
                                .text_color(rgb(palette.accent))
                                .child(icon(
                                    if state.notes_open {
                                        "chevron"
                                    } else {
                                        "chevron-right"
                                    },
                                    12.,
                                    palette.accent,
                                ))
                                .child(format!("What’s new in {}", release.version)),
                            palette,
                            &self.settings_focus.update_notes,
                            cx.listener(|app, _: &(), _, cx| {
                                app.app_updates.notes_open = !app.app_updates.notes_open;
                                cx.notify();
                            }),
                        )),
                    )
                    .when(state.notes_open, |view| {
                        view.child(
                            palette
                                .label(release.notes.clone())
                                .text_size(px(11.))
                                .line_height(px(20.))
                                .font_family(if cfg!(target_os = "macos") {
                                    "Menlo"
                                } else {
                                    "Consolas"
                                })
                                .p(px(12.))
                                .rounded(px(5.))
                                .border_1()
                                .border_color(palette.line)
                                .bg(rgb(palette.surface)),
                        )
                    })
                },
            )
    }
}

#[cfg(all(test, feature = "headless-ui-tests"))]
mod ui_tests {
    use super::*;
    use crate::{Client, focus_visible, ui_tests::press};
    use gpui::{Modifiers, TestAppContext, size};

    #[gpui::test]
    fn update_text_actions_keep_bounds_and_keyboard_pointer_behaviour_in_both_themes(
        cx: &mut TestAppContext,
    ) {
        for light in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut app = Astral::new(
                    [None, None],
                    Some(directory.path()),
                    Some(Client::Retail),
                    light,
                    cx,
                );
                app.page = Page::Settings;
                app.app_updates.phase = Phase::Available(Release {
                    version: Version::new(9, 9, 9),
                    notes: "Fixture release notes".into(),
                    asset: Asset {
                        url: "https://example.invalid/update.zip".into(),
                        sha256: "a".repeat(64),
                        size: 42,
                    },
                });
                app
            });
            cx.simulate_resize(size(px(1250.), px(1200.)));
            cx.run_until_parked();
            let before = cx.debug_bounds("app-release-notes").unwrap();
            view.update_in(cx, |app, window, _| {
                app.settings_focus.update_notes.focus(window)
            });
            cx.run_until_parked();
            assert_eq!(before, cx.debug_bounds("app-release-notes").unwrap());
            press(cx, "enter");
            assert!(view.read_with(cx, |app, _| app.app_updates.notes_open));
            assert_eq!(before, cx.debug_bounds("app-release-notes").unwrap());
            view.update_in(cx, |app, window, _| app.brand_focus.focus(window));
            cx.simulate_click(before.center(), Modifiers::default());
            cx.run_until_parked();
            view.update_in(cx, |app, window, cx| {
                assert!(!app.app_updates.notes_open);
                assert!(!focus_visible::indicated(
                    &app.settings_focus.update_notes,
                    window,
                    cx
                ));
            });
            press(cx, "space");
            view.update_in(cx, |app, window, cx| {
                assert!(app.app_updates.notes_open);
                assert!(focus_visible::indicated(
                    &app.settings_focus.update_notes,
                    window,
                    cx
                ));
            });
            assert_eq!(before, cx.debug_bounds("app-release-notes").unwrap());
            view.update_in(cx, |app, window, cx| {
                app.navigate(Page::Library, window, cx);
            });
            cx.run_until_parked();
            let banner = cx.debug_bounds("view-app-update").unwrap();
            view.update_in(cx, |app, window, _| {
                app.settings_focus.update_banner.focus(window)
            });
            cx.run_until_parked();
            assert_eq!(banner, cx.debug_bounds("view-app-update").unwrap());
            press(cx, "enter");
            assert!(view.read_with(cx, |app, _| app.page == Page::Settings));
        }
    }
}
