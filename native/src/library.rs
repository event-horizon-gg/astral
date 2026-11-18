use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env, fs, io,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Client {
    #[default]
    Retail,
    Forever,
}

impl Client {
    pub const ALL: [Self; 2] = [Self::Retail, Self::Forever];

    pub fn index(self) -> usize {
        match self {
            Self::Retail => 0,
            Self::Forever => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Retail => "Retail",
            Self::Forever => "Forever",
        }
    }

    pub(crate) fn folder(self) -> &'static str {
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

#[derive(Clone, Deserialize, Serialize)]
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
    #[serde(rename = "sourceUrl")]
    pub source_url: Option<String>,
    pub bundled: Option<bool>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub mark: Option<String>,
    pub color: Option<String>,
    pub local: Option<bool>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default, rename = "allowRetailPatchMismatch")]
    pub allow_retail_patch_mismatch: bool,
}

impl Definition {
    pub fn module_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.folder.as_str()).chain(
            self.folders
                .iter()
                .filter(|folder| *folder != &self.folder)
                .map(String::as_str),
        )
    }
    pub fn github_project(
        repo: &str,
        client: Client,
        folders: Vec<String>,
        folder: String,
        asset: String,
    ) -> Self {
        Self {
            id: format!("github:{}:{}", repo.to_ascii_lowercase(), client.label()),
            name: folder.clone(),
            folder,
            folders,
            asset: Some(asset),
            repo: Some(repo.to_owned()),
            flavors: Some(vec![client.label().into()]),
            version_scheme: None,
            version_companion: None,
            prefix: None,
            source_url: None,
            bundled: None,
            description: None,
            category: Some("Community".into()),
            mark: None,
            color: Some("slate".into()),
            local: None,
            requires: Vec::new(),
            allow_retail_patch_mismatch: false,
        }
    }
    pub fn supports(&self, client: Client) -> bool {
        self.flavors.as_ref().is_none_or(|flavors| {
            flavors
                .iter()
                .any(|flavor| flavor.eq_ignore_ascii_case(client.label()))
        })
    }

    pub fn includes(&self, folder: &str) -> bool {
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
        if self.bundled == Some(true) || self.local == Some(true) {
            "Astral"
        } else if self.repo.is_some() {
            "GitHub"
        } else {
            "Manual"
        }
    }

    pub(crate) fn installed_version(
        &self,
        addons: &Path,
        client: Client,
        version: Option<&str>,
    ) -> io::Result<String> {
        let version = version.unwrap_or("Unknown");
        if let Some(companion) = &self.version_companion {
            let database_version = toc(&addons.join(companion), companion, client)?
                .and_then(|mut metadata| metadata.remove("Version"))
                .unwrap_or_else(|| "0".into());
            Ok(format!("{version}+v{database_version}"))
        } else {
            Ok(version.to_owned())
        }
    }
}

pub fn catalog() -> Vec<Definition> {
    serde_json::from_str(include_str!("../catalog.json")).expect("valid embedded Astral catalog")
}

#[derive(Clone, Serialize)]
pub struct Addon {
    pub name: String,
    pub category: String,
    pub folder: String,
    pub version: String,
    pub source: &'static str,
    pub modules: Vec<String>,
    #[serde(skip_serializing)]
    pub artwork: Option<std::sync::Arc<gpui::Image>>,
}

impl Addon {
    pub fn matches_library_query(&self, lowercase_query: &str) -> bool {
        format!("{} {}", self.name, self.category)
            .to_lowercase()
            .contains(lowercase_query)
    }
}

pub fn toc_names(folder: &str, client: Client) -> Vec<String> {
    let mut candidates = Vec::new();
    if client == Client::Forever {
        candidates.push(format!("{folder}_Camelot.toc"));
    }
    candidates.extend([format!("{folder}_Mainline.toc"), format!("{folder}.toc")]);
    candidates
}

pub fn toc_metadata(text: &str) -> HashMap<String, String> {
    text.trim_start_matches('\u{feff}')
        .lines()
        .filter_map(|line| {
            let (key, value) = line.strip_prefix("##")?.split_once(':')?;
            let key = key.trim();
            matches!(
                key,
                "Title"
                    | "Version"
                    | "IconTexture"
                    | "Interface"
                    | "Dependencies"
                    | "RequiredDeps"
                    | "DefaultState"
            )
            .then(|| (key.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

pub(crate) fn toc(
    dir: &Path,
    folder: &str,
    client: Client,
) -> io::Result<Option<HashMap<String, String>>> {
    for candidate in toc_names(folder, client) {
        let file = dir.join(candidate);
        let metadata = match fs::symlink_metadata(&file) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !metadata.is_file() {
            continue;
        }
        return read_toc(&file).map(Some);
    }
    Ok(None)
}

fn read_toc(file: &Path) -> io::Result<HashMap<String, String>> {
    // Some addons embed databases in the TOC; only the header is needed.
    let mut bytes = Vec::new();
    fs::File::open(file)?
        .take(64 * 1024)
        .read_to_end(&mut bytes)?;
    Ok(toc_metadata(&String::from_utf8_lossy(&bytes)))
}

fn inventory_toc(
    dir: &Path,
    folder: &str,
    client: Client,
    managed: bool,
) -> io::Result<Option<HashMap<String, String>>> {
    if managed {
        if let Some(metadata) = toc(dir, folder, client)? {
            return Ok(Some(metadata));
        }
    } else {
        let file = dir.join(format!("{folder}.toc"));
        match fs::symlink_metadata(&file) {
            Ok(metadata) if metadata.is_file() => return read_toc(&file).map(Some),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let mut candidate = None;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "toc")
            && entry.file_type()?.is_file()
            && candidate.as_ref().is_none_or(|previous| &path < previous)
        {
            candidate = Some(path);
        }
    }
    candidate.as_deref().map(read_toc).transpose()
}

pub(crate) fn plain_title(text: &str) -> String {
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

fn has_client_folder(root: &Path, client: Client) -> bool {
    let name = root.file_name().and_then(|name| name.to_str());
    name == Some(client.folder()) || (client == Client::Forever && name == Some("_classic_beta_"))
}

pub fn validate_root(root: &Path, client: Client) -> io::Result<()> {
    if !has_client_folder(root, client) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Choose the {} folder.", client.folder()),
        ));
    }
    if !root.join("Interface").join("AddOns").is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "The selected folder has no Interface/AddOns directory.",
        ));
    }
    Ok(())
}

pub fn client_for_root(root: &Path) -> io::Result<Client> {
    let client = Client::ALL
        .into_iter()
        .find(|client| has_client_folder(root, *client))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Choose a Retail or Forever folder containing Interface/AddOns.",
            )
        })?;
    validate_root(root, client)?;
    Ok(client)
}

#[cfg(test)]
#[test]
fn folder_picker_identifies_both_clients_and_requires_an_addon_directory() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    for (folder, client) in [
        ("_retail_", Client::Retail),
        ("_forever_", Client::Forever),
        ("_classic_beta_", Client::Forever),
    ] {
        let root = directory.path().join(folder);
        assert!(client_for_root(&root).is_err());
        fs::create_dir_all(root.join("Interface/AddOns"))?;
        assert!(client_for_root(&root)? == client);
        assert!(scan(&root, client, &[] as &[Definition])?.is_empty());
    }
    assert!(client_for_root(directory.path()).is_err());
    Ok(())
}

pub fn scan(root: &Path, client: Client, catalog: &[Definition]) -> io::Result<Vec<Addon>> {
    scan_addons(root, client, catalog, true)
}

pub fn scan_library(root: &Path, client: Client, catalog: &[Definition]) -> io::Result<Vec<Addon>> {
    scan_inventory(root, client, catalog, true, true)
}

#[cfg(test)]
#[test]
fn inventory_uses_client_tocs_for_known_addons_and_base_tocs_for_manual_addons() -> io::Result<()> {
    let fixture = tempfile::tempdir()?;
    let root = fixture.path().join(Client::Forever.folder());
    let addons = root.join("Interface/AddOns");
    for folder in ["KeyUI", "ManualAddon"] {
        let dir = addons.join(folder);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("{folder}.toc")), "## Version: base\n")?;
        fs::write(
            dir.join(format!("{folder}_Camelot.toc")),
            "## Version: forever\n",
        )?;
    }
    let catalog = catalog();
    let rows = scan(&root, Client::Forever, &catalog)?;
    assert_eq!(
        rows.iter()
            .find(|addon| addon.folder == "KeyUI")
            .unwrap()
            .version,
        "forever"
    );
    assert_eq!(
        rows.iter()
            .find(|addon| addon.folder == "ManualAddon")
            .unwrap()
            .version,
        "base"
    );

    fs::remove_file(addons.join("KeyUI/KeyUI.toc"))?;
    fs::remove_file(addons.join("KeyUI/KeyUI_Camelot.toc"))?;
    fs::write(
        addons.join("KeyUI/Alternate.toc"),
        "## Version: alternate\n",
    )?;
    let rows = scan(&root, Client::Forever, &catalog)?;
    assert_eq!(
        rows.iter()
            .find(|addon| addon.folder == "KeyUI")
            .unwrap()
            .version,
        "alternate"
    );
    Ok(())
}

pub fn scan_addons(
    root: &Path,
    client: Client,
    catalog: &[Definition],
    artwork: bool,
) -> io::Result<Vec<Addon>> {
    scan_inventory(root, client, catalog, artwork, false)
}

fn scan_inventory(
    root: &Path,
    client: Client,
    catalog: &[Definition],
    artwork: bool,
    show_incomplete: bool,
) -> io::Result<Vec<Addon>> {
    validate_root(root, client)?;
    let base = root.join("Interface").join("AddOns").canonicalize()?;
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
        let meta = inventory_toc(
            &base.join(&definition.folder),
            &definition.folder,
            client,
            true,
        )?;
        if meta.is_none() && !show_incomplete {
            continue;
        }
        let meta = meta.unwrap_or_default();
        let modules: Vec<_> = folders
            .iter()
            .filter(|folder| definition.includes(folder) && !claimed.contains(*folder))
            .cloned()
            .collect();
        claimed.extend(modules.iter().cloned());
        let version =
            definition.installed_version(&base, client, meta.get("Version").map(String::as_str))?;
        addons.push(Addon {
            name: definition.name.clone(),
            category: definition
                .category
                .clone()
                .unwrap_or_else(|| "Other".into()),
            folder: definition.folder.clone(),
            source: definition.source(),
            version,
            artwork: meta
                .get("IconTexture")
                .filter(|_| artwork)
                .and_then(|reference| crate::artwork::load(&base, reference)),
            modules,
        });
    }
    for folder in folders
        .into_iter()
        .filter(|folder| !claimed.contains(folder))
    {
        if let Some(meta) = inventory_toc(&base.join(&folder), &folder, client, false)? {
            addons.push(Addon {
                category: "Other".into(),
                name: meta
                    .get("Title")
                    .map(|name| plain_title(name))
                    .unwrap_or_else(|| folder.clone()),
                version: meta
                    .get("Version")
                    .cloned()
                    .unwrap_or_else(|| "Unknown".into()),
                source: "Manual",
                artwork: meta
                    .get("IconTexture")
                    .filter(|_| artwork)
                    .and_then(|reference| crate::artwork::load(&base, reference)),
                modules: vec![folder.clone()],
                folder,
            });
        }
    }
    Ok(addons)
}
