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
    name: String,
    state: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

#[derive(Clone)]
pub struct Package {
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

pub fn project_packages(repo: &str) -> Result<ProjectRelease, String> {
    let release = fetch_release(&client(), repo)?;
    let prefix = format!("https://github.com/{repo}/releases/download/");
    let packages: Vec<_> = release
        .assets
        .into_iter()
        .filter(|asset| {
            asset.state == "uploaded"
                && asset.name.to_ascii_lowercase().ends_with(".zip")
                && (1..=crate::packages::MAX_ZIP).contains(&asset.size)
                && asset.browser_download_url.starts_with(&prefix)
        })
        .map(|asset| Package {
            name: asset.name,
            size: asset.size,
            digest: asset.digest,
            url: asset.browser_download_url,
        })
        .collect();
    Ok(ProjectRelease {
        tag: release.tag_name,
        notes: release
            .body
            .unwrap_or_default()
            .chars()
            .take(12_000)
            .collect(),
        packages,
    })
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
            name: asset.name,
            size: asset.size,
            digest: asset.digest,
            url: asset.browser_download_url,
        },
    })
}

pub fn download(package: &Package) -> Result<Vec<u8>, String> {
    use sha2::{Digest, Sha256};

    if package.size > crate::packages::MAX_ZIP {
        return Err("The ZIP exceeds the 128 MB download limit.".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into();
    let bytes = agent
        .get(&package.url)
        .header("User-Agent", "Astral-Native")
        .call()
        .map_err(|err| format!("Couldn't download the package. {err}"))?
        .body_mut()
        .with_config()
        .limit(package.size + 1)
        .read_to_vec()
        .map_err(|err| format!("Couldn't read the downloaded package. {err}"))?;
    if bytes.len() as u64 != package.size {
        return Err("The download is incomplete.".into());
    }
    if let Some(digest) = &package.digest {
        if digest.strip_prefix("sha256:") != Some(format!("{:x}", Sha256::digest(&bytes)).as_str())
        {
            return Err("The download checksum doesn't match GitHub.".into());
        }
    }
    Ok(bytes)
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
