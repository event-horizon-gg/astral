use crate::library::Definition;
use regex::Regex;
use serde::Deserialize;
use std::{cmp::Ordering, time::Duration};

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    published_at: Option<String>,
    body: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
    state: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

#[derive(Clone)]
pub struct Package {
    pub id: u64,
    pub name: String,
    pub size: u64,
    pub digest: Option<String>,
    pub url: String,
}

pub struct Release {
    pub tag: String,
    pub published: Option<String>,
    pub notes: String,
    pub package: Package,
}

pub struct Check {
    pub tag: String,
    pub comparison: Option<Ordering>,
    pub notes: String,
}

/// engine.cjs exposes the release version separately from its original tag.
pub(crate) fn display_version(source: &Definition, tag: &str) -> String {
    use std::sync::OnceLock;
    if source.version_scheme.as_deref() == Some("questie-bundle") {
        tag.strip_prefix("bundle/v").unwrap_or(tag).to_owned()
    } else {
        static PREFIX: OnceLock<Regex> = OnceLock::new();
        PREFIX
            .get_or_init(|| Regex::new(r"^v\.?\s*").expect("valid version prefix"))
            .replace(tag, "")
            .into_owned()
    }
}

pub fn client() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(12)))
        .build()
        .into()
}

pub fn latest(agent: &ureq::Agent, source: &Definition) -> Result<Release, String> {
    let repo = source.repo.as_deref().ok_or("No connected source.")?;
    select_release(source, repo, fetch_release(agent, repo)?)
}

fn fetch_release(agent: &ureq::Agent, repo: &str) -> Result<GitHubRelease, String> {
    let release: GitHubRelease = agent
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .header("User-Agent", "Astral-Native")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|err| match err {
            ureq::Error::StatusCode(403 | 429) => "GitHub rate limit reached.".into(),
            ureq::Error::StatusCode(code) => format!("GitHub returned HTTP {code}."),
            _ => format!("Couldn't reach GitHub. {err}"),
        })?
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_json()
        .map_err(|err| format!("Couldn't read GitHub release. {err}"))?;
    if release.draft || release.prerelease {
        return Err("This project has no stable release.".into());
    }
    Ok(release)
}

pub struct ProjectRelease {
    pub tag: String,
    pub notes: String,
    pub packages: Vec<Package>,
}

impl ProjectRelease {
    pub fn package_index(&self, id: u64) -> Result<usize, String> {
        self.packages
            .iter()
            .position(|package| package.id == id)
            .ok_or_else(|| "Choose a published addon ZIP.".into())
    }
}

pub fn project_packages(repo: &str) -> Result<ProjectRelease, String> {
    Ok(project_release(repo, fetch_release(&client(), repo)?))
}

fn project_release(repo: &str, release: GitHubRelease) -> ProjectRelease {
    let prefix = format!("https://github.com/{repo}/releases/download/");
    let packages: Vec<_> = release
        .assets
        .into_iter()
        .filter(|asset| {
            asset.state == "uploaded"
                && asset.name.to_ascii_lowercase().ends_with(".zip")
                && (1..=crate::packages::MAX_DOWNLOAD).contains(&asset.size)
                && asset.browser_download_url.starts_with(&prefix)
        })
        .map(|asset| Package {
            id: asset.id,
            name: asset.name,
            size: asset.size,
            digest: asset.digest,
            url: asset.browser_download_url,
        })
        .collect();
    ProjectRelease {
        tag: release.tag_name,
        notes: release
            .body
            .unwrap_or_default()
            .chars()
            .take(12_000)
            .collect(),
        packages,
    }
}

fn select_release(
    source: &Definition,
    repo: &str,
    release: GitHubRelease,
) -> Result<Release, String> {
    if release.draft || release.prerelease {
        return Err("This project has no stable release.".into());
    }
    let pattern = Regex::new(&format!(
        "(?i){}",
        source.asset.as_deref().ok_or("No package pattern.")?
    ))
    .map_err(|_| "Invalid package pattern.")?;
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.state == "uploaded" && asset.size > 0 && pattern.is_match(&asset.name))
        .ok_or("No matching addon package in the latest release.")?;
    let prefix = format!("https://github.com/{repo}/releases/download/");
    if !asset.browser_download_url.starts_with(&prefix) {
        return Err("Unexpected release download address.".into());
    }
    Ok(Release {
        tag: release.tag_name,
        published: release.published_at,
        notes: release
            .body
            .unwrap_or_default()
            .chars()
            .take(12_000)
            .collect(),
        package: Package {
            id: asset.id,
            name: asset.name,
            size: asset.size,
            digest: asset.digest,
            url: asset.browser_download_url,
        },
    })
}

pub fn download(package: &Package) -> Result<crate::packages::Input, String> {
    download_with_progress(package, |_| {})
}

pub fn download_with_progress(
    package: &Package,
    progress: impl FnMut(u8),
) -> Result<crate::packages::Input, String> {
    if package.size > crate::packages::MAX_DOWNLOAD {
        return Err("The ZIP exceeds the 512 MB download limit.".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(180)))
        .build()
        .into();
    let mut response = agent
        .get(&package.url)
        .header("User-Agent", "Astral-Native")
        .call()
        .map_err(|err| format!("Couldn't download the package. {err}"))?;
    read_download(
        response
            .body_mut()
            .with_config()
            .limit(package.size + 1)
            .reader(),
        package,
        progress,
    )
}

fn read_download(
    mut reader: impl std::io::Read,
    package: &Package,
    mut progress: impl FnMut(u8),
) -> Result<crate::packages::Input, String> {
    use sha2::{Digest, Sha256};
    if package.size > crate::packages::MAX_DOWNLOAD {
        return Err("The ZIP exceeds the 512 MB download limit.".into());
    }
    let mut output = tempfile::NamedTempFile::new()
        .map_err(|error| format!("Couldn't save the downloaded package. {error}"))?;
    let mut bytes = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 32768];
    let mut last = 0;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Couldn't read the downloaded package. {error}")),
        };
        if bytes + count as u64 > package.size {
            return Err("The download is incomplete.".into());
        }
        std::io::Write::write_all(&mut output, &buffer[..count])
            .map_err(|error| format!("Couldn't save the downloaded package. {error}"))?;
        hash.update(&buffer[..count]);
        bytes += count as u64;
        if package.size > 0 {
            // Electron reserves 0–85 for actual downloaded bytes, then 90 for
            // a validated package and 100 only after the install commits.
            let percent = ((bytes * 85 + package.size / 2) / package.size).min(85) as u8;
            if percent != last {
                progress(percent);
                last = percent;
            }
        }
    }
    if bytes != package.size {
        return Err("The download is incomplete.".into());
    }
    if let Some(digest) = &package.digest {
        if digest.strip_prefix("sha256:") != Some(format!("{:x}", hash.finalize()).as_str()) {
            return Err("The download checksum doesn't match GitHub.".into());
        }
    }
    Ok(crate::packages::Input::File(output))
}

pub fn check(agent: &ureq::Agent, source: &Definition, installed: &str) -> Result<Check, String> {
    let (tag, notes) = if source.bundled == Some(true) {
        let tag = crate::bundled::version(&source.folder)
            .ok_or("This addon is missing from the Astral build.")?;
        (tag, String::new())
    } else {
        let release = latest(agent, source)?;
        (release.tag, release.notes)
    };
    Ok(Check {
        comparison: compare(source, installed, &tag),
        tag,
        notes,
    })
}

fn numbers(value: &str) -> Option<Vec<u64>> {
    let mut parts: Vec<u64> = value
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    while parts.last() == Some(&0) {
        parts.pop();
    }
    Some(parts)
}

fn trim_prefix(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('v')
        .unwrap_or(value)
        .trim_start_matches('.')
        .trim_start()
}

fn ordinary(value: &str) -> Option<Vec<u64>> {
    numbers(trim_prefix(value).split(['-', '+']).next()?)
}

fn letter(value: &str) -> Option<(Vec<u64>, Option<char>)> {
    let value = trim_prefix(value);
    match value.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => {
            let base = value[..value.len() - 1].trim_end_matches([' ', '-']);
            Some((numbers(base)?, Some(c.to_ascii_lowercase())))
        }
        _ => Some((numbers(value)?, None)),
    }
}

fn revision(value: &str) -> Option<u64> {
    value.trim().trim_start_matches('r').parse().ok()
}

fn bundle(value: &str) -> Option<(Vec<u64>, Vec<u64>)> {
    let (addon, database) = value
        .strip_prefix("bundle/v")
        .unwrap_or(value)
        .split_once("+v")?;
    Some((numbers(addon)?, numbers(database)?))
}

fn rio(value: &str) -> Option<&str> {
    let version = value.split_once('v')?.1.get(..12)?;
    version
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then_some(version)
}

pub fn compare(source: &Definition, installed: &str, tag: &str) -> Option<Ordering> {
    if source.id == "rio" {
        return Some(rio(tag)?.cmp(rio(installed)?));
    }
    Some(match source.version_scheme.as_deref() {
        Some("revision") => revision(tag)?.cmp(&revision(installed)?),
        Some("letter-patch") => letter(tag)?.cmp(&letter(installed)?),
        Some("questie-bundle") => bundle(tag)?.cmp(&bundle(installed)?),
        _ => ordinary(tag)?.cmp(&ordinary(installed)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn download_fixture(size: u64) -> Package {
        Package {
            id: 42,
            name: "Addon.zip".into(),
            size,
            digest: None,
            url: String::new(),
        }
    }

    struct Chunks(std::io::Cursor<Vec<u8>>);
    impl std::io::Read for Chunks {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let length = output.len().min(25);
            std::io::Read::read(&mut self.0, &mut output[..length])
        }
    }

    fn content(input: crate::packages::Input) -> Vec<u8> {
        match input {
            crate::packages::Input::Bytes(bytes) => bytes,
            crate::packages::Input::File(file) => std::fs::read(file.path()).unwrap(),
        }
    }

    #[test]
    fn download_progress_tracks_bytes_and_reserves_install_completion() {
        let mut progress = vec![];
        let data = vec![7u8; 100];
        assert_eq!(
            content(
                read_download(
                    Chunks(std::io::Cursor::new(data.clone())),
                    &download_fixture(100),
                    |percent| progress.push(percent)
                )
                .unwrap()
            ),
            data
        );
        assert_eq!(progress, [21, 43, 64, 85]);
    }

    #[test]
    fn streamed_download_rejects_short_and_excess_data_and_bad_checksums() {
        for length in [99, 101] {
            assert!(
                read_download(
                    std::io::Cursor::new(vec![7u8; length]),
                    &download_fixture(100),
                    |_| {}
                )
                .is_err()
            );
        }
        let mut package = download_fixture(5);
        package.digest = Some("sha256:wrong".into());
        assert!(
            read_download(std::io::Cursor::new(b"hello"), &package, |_| {})
                .unwrap_err()
                .contains("checksum")
        );
        package.digest =
            Some("sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into());
        assert_eq!(
            content(read_download(std::io::Cursor::new(b"hello"), &package, |_| {}).unwrap()),
            b"hello"
        );
        assert!(
            read_download(
                std::io::empty(),
                &download_fixture(crate::packages::MAX_DOWNLOAD + 1),
                |_| {}
            )
            .unwrap_err()
            .contains("limit")
        );
    }

    #[test]
    fn a_download_over_the_previous_limit_is_spooled_to_disk_with_bounded_reads() {
        use std::io::Read;
        let size = 129 * 1024 * 1024;
        struct Reader {
            remaining: u64,
        }
        impl std::io::Read for Reader {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                assert!(output.len() <= 32 * 1024);
                let count = (output.len() as u64).min(self.remaining) as usize;
                output[..count].fill(7);
                self.remaining -= count as u64;
                Ok(count)
            }
        }
        let input =
            read_download(Reader { remaining: size }, &download_fixture(size), |_| {}).unwrap();
        let crate::packages::Input::File(file) = input else {
            panic!("Downloaded archives must remain on disk")
        };
        assert_eq!(file.as_file().metadata().unwrap().len(), size);
        let mut check = file.reopen().unwrap();
        let mut sample = [0; 32];
        check.read_exact(&mut sample).unwrap();
        assert_eq!(sample, [7; 32]);
    }

    #[test]
    fn project_release_keeps_github_asset_identity_after_package_filtering() {
        let release: GitHubRelease = serde_json::from_value(serde_json::json!({
            "tag_name": "v2", "body": "Current notes", "published_at": null,
            "draft": false, "prerelease": false,
            "assets": [
                {"id": 40, "name": "Addon.zip", "state": "uploaded", "size": 100,
                 "browser_download_url": "https://github.com/owner/other/releases/download/v2/Addon.zip"},
                {"id": 41, "name": "Addon.txt", "state": "uploaded", "size": 100,
                 "browser_download_url": "https://github.com/owner/addon/releases/download/v2/Addon.txt"},
                {"id": 42, "name": "Addon.zip", "state": "uploaded", "size": 100,
                 "digest": "sha256:current", "browser_download_url": "https://github.com/owner/addon/releases/download/v2/Addon.zip"}
            ]
        })).unwrap();
        let release = project_release("owner/addon", release);
        assert_eq!(release.packages.len(), 1);
        assert_eq!(release.packages[0].id, 42);
        assert_eq!(
            release.packages[0].digest.as_deref(),
            Some("sha256:current")
        );
        assert_eq!(release.tag, "v2");
        assert_eq!(release.notes, "Current notes");
        assert!(release.package_index(40).is_err());
        assert!(release.package_index(41).is_err());
    }

    #[test]
    fn a_refreshed_release_must_still_contain_the_selected_asset_id() {
        let package = |id, name: &str| Package {
            id,
            name: name.into(),
            size: 100,
            digest: None,
            url: format!("https://github.com/owner/addon/releases/download/v2/{name}"),
        };
        let release = ProjectRelease {
            tag: "v2".into(),
            notes: "Updated notes".into(),
            packages: vec![package(42, "Addon.zip"), package(43, "Addon-Forever.zip")],
        };
        // Identical filenames/download paths do not authorize a replacement asset.
        assert!(release.package_index(41).is_err());
        assert_eq!(release.package_index(42).unwrap(), 0);
        assert_eq!(release.package_index(43).unwrap(), 1);
        assert!(
            ProjectRelease {
                tag: "v3".into(),
                notes: String::new(),
                packages: vec![]
            }
            .package_index(42)
            .is_err()
        );
    }

    #[test]
    fn compares_author_versions_and_refuses_unknown_formats() {
        let catalog = crate::library::catalog();
        for (id, current, tag, expected) in [
            ("consoleport", "3.3.9", "3.3.13", Some(Ordering::Greater)),
            ("consoleport", "3.3", "v3.3.0", Some(Ordering::Equal)),
            ("narcissus", "1.8.7 b", "v1.8.7-c", Some(Ordering::Greater)),
            ("narcissus", "1.8.7 b", "v1.8.8-beta", None),
            (
                "questie",
                "12.6.1+v1.0.10",
                "bundle/v12.6.1+v1.0.11",
                Some(Ordering::Greater),
            ),
            (
                "rio",
                "v202611051200",
                "v202611041200",
                Some(Ordering::Less),
            ),
        ] {
            let source = catalog.iter().find(|source| source.id == id).unwrap();
            assert_eq!(compare(source, current, tag), expected, "{id}");
        }
    }
}
