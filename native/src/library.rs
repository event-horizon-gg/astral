use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env, fs, io,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Client {
    Retail,
    Forever,
}

impl Client {
    pub fn label(self) -> &'static str {
        match self {
            Self::Retail => "Retail",
            Self::Forever => "Forever",
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Self::Retail => "_retail_",
            Self::Forever => "_forever_",
        }
    }

    pub fn detect(self) -> Option<PathBuf> {
        let mut roots = vec![PathBuf::from("/Applications/World of Warcraft")];
        for key in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(base) = env::var_os(key) {
                roots.push(PathBuf::from(base).join("World of Warcraft"));
            }
        }
        roots.push(PathBuf::from(r"C:\Games\World of Warcraft"));
        roots
            .into_iter()
            .map(|root| root.join(self.folder()))
            .find(|root| root.join("Interface").join("AddOns").is_dir())
    }
}

#[derive(Deserialize)]
pub struct Definition {
    pub id: String,
    pub asset: Option<String>,
    #[serde(rename = "versionScheme")]
    pub version_scheme: Option<String>,
    #[serde(rename = "versionCompanion")]
    version_companion: Option<String>,
    pub name: String,
    pub folder: String,
    #[serde(default)]
    folders: Vec<String>,
    prefix: Option<String>,
    flavors: Option<Vec<String>>,
    pub repo: Option<String>,
    pub bundled: Option<bool>,
    pub description: Option<String>,
}

impl Definition {
    pub fn supports(&self, client: Client) -> bool {
        self.flavors.as_ref().is_none_or(|flavors| {
            flavors
                .iter()
                .any(|flavor| flavor.eq_ignore_ascii_case(client.label()))
        })
    }

    fn includes(&self, folder: &str) -> bool {
        self.folder == folder
            || (self.id == "ellesmere" && folder.starts_with("EllesmereUI"))
            || self.folders.iter().any(|name| name == folder)
            || self.prefix.as_ref().is_some_and(|prefix| {
                folder
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('_'))
            })
    }

    pub fn source(&self) -> &'static str {
        if self.bundled == Some(true) {
            "Astral"
        } else if self.repo.is_some() {
            "GitHub"
        } else {
            "Manual"
        }
    }
}

pub fn catalog() -> Vec<Definition> {
    serde_json::from_str(include_str!("../catalog.json")).expect("valid embedded Astral catalog")
}

#[derive(Serialize)]
pub struct Addon {
    pub name: String,
    pub folder: String,
    pub version: String,
    pub source: &'static str,
    pub modules: Vec<String>,
}

fn toc(dir: &Path, folder: &str, client: Client) -> io::Result<Option<HashMap<String, String>>> {
    let mut candidates = Vec::new();
    if client == Client::Forever {
        candidates.push(format!("{folder}_Camelot.toc"));
    }
    candidates.extend([format!("{folder}_Mainline.toc"), format!("{folder}.toc")]);
    for candidate in candidates {
        let file = dir.join(candidate);
        let metadata = match fs::symlink_metadata(&file) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !metadata.is_file() {
            continue;
        }
        // Some addons embed databases in the TOC; only the header is needed.
        let mut bytes = Vec::new();
        fs::File::open(file)?
            .take(64 * 1024)
            .read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes);
        return Ok(Some(
            text.trim_start_matches('\u{feff}')
                .lines()
                .filter_map(|line| {
                    let (key, value) = line.strip_prefix("##")?.split_once(':')?;
                    let key = key.trim();
                    matches!(key, "Title" | "Version")
                        .then(|| (key.to_owned(), value.trim().to_owned()))
                })
                .collect(),
        ));
    }
    Ok(None)
}

fn plain_title(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('|') {
        result.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(code) = rest.get(2..10).filter(|code| {
            rest.starts_with("|c") && code.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            rest = &rest[2 + code.len()..];
        } else if let Some(suffix) = rest.strip_prefix("|r") {
            rest = suffix;
        } else {
            result.push('|');
            rest = &rest[1..];
        }
    }
    result.push_str(rest);
    result
}

pub fn scan(root: &Path, client: Client, catalog: &[Definition]) -> io::Result<Vec<Addon>> {
    if root.file_name().and_then(|name| name.to_str()) != Some(client.folder()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Choose the {} folder.", client.folder()),
        ));
    }
    let base = root.join("Interface").join("AddOns");
    let mut folders = Vec::new();
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        if let (true, Ok(name)) = (entry.file_type()?.is_dir(), entry.file_name().into_string()) {
            folders.push(name);
        }
    }
    folders.sort();
    let mut claimed = HashSet::new();
    let mut addons = Vec::new();
    for definition in catalog
        .iter()
        .filter(|definition| definition.supports(client))
    {
        if claimed.contains(&definition.folder) || !folders.contains(&definition.folder) {
            continue;
        }
        let Some(meta) = toc(&base.join(&definition.folder), &definition.folder, client)? else {
            continue;
        };
        let modules: Vec<_> = folders
            .iter()
            .filter(|folder| definition.includes(folder) && !claimed.contains(*folder))
            .cloned()
            .collect();
        claimed.extend(modules.iter().cloned());
        let mut version = meta
            .get("Version")
            .cloned()
            .unwrap_or_else(|| "Unknown".into());
        if let Some(companion) = &definition.version_companion {
            let database_version = toc(&base.join(companion), companion, client)?
                .and_then(|meta| meta.get("Version").cloned())
                .unwrap_or_else(|| "0".into());
            version = format!("{version}+v{database_version}");
        }
        addons.push(Addon {
            name: definition.name.clone(),
            folder: definition.folder.clone(),
            source: definition.source(),
            version,
            modules,
        });
    }
    for folder in folders
        .into_iter()
        .filter(|folder| !claimed.contains(folder))
    {
        if let Some(meta) = toc(&base.join(&folder), &folder, client)? {
            addons.push(Addon {
                name: meta
                    .get("Title")
                    .map(|name| plain_title(name))
                    .unwrap_or_else(|| folder.clone()),
                version: meta
                    .get("Version")
                    .cloned()
                    .unwrap_or_else(|| "Unknown".into()),
                source: "Manual",
                modules: vec![folder.clone()],
                folder,
            });
        }
    }
    addons.sort_by_cached_key(|addon| addon.name.to_lowercase());
    Ok(addons)
}
