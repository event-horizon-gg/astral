use crate::library::{self, Client, Definition};
use async_zip::base::read::mem::ZipFileReader;
use futures_lite::{future::block_on, io::AsyncReadExt};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_ENTRIES: usize = 50_000;
pub const MAX_ZIP: u64 = 128 * 1024 * 1024;
const MAX_EXPANDED: u64 = 512 * 1024 * 1024;
const MAX_TOC: u64 = 256 * 1024;
const MAX_QUESTIE_DB_TOC: u64 = 32 * 1024 * 1024;

pub fn read_zip(path: &Path) -> Result<Vec<u8>, String> {
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        return Err("Choose a ZIP file.".into());
    }
    let metadata = path.symlink_metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_ZIP {
        return Err("Choose a ZIP file under 128 MB.".into());
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_ZIP + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ZIP {
        return Err("The ZIP exceeds the 128 MB limit.".into());
    }
    Ok(bytes)
}

pub struct Module {
    pub folder: String,
    pub version: Option<String>,
    pub replaces: bool,
    pub(crate) reviewed_version: Option<String>,
}

pub struct Inspection {
    pub interface: u32,
    pub modules: Vec<Module>,
    pub missing: BTreeSet<String>,
    pub installed_version: Option<String>,
    pub(crate) root: PathBuf,
    pub(crate) client: Client,
    pub(crate) dependencies: BTreeSet<String>,
    archive: ZipFileReader,
    files: Vec<(PathBuf, usize)>,
}

impl Inspection {
    pub fn stage(&self, destination: &Path) -> Result<(), String> {
        block_on(async {
            for (path, index) in &self.files {
                let file = destination.join(path);
                fs::create_dir_all(file.parent().ok_or("Invalid ZIP file path.")?)
                    .map_err(|e| e.to_string())?;
                let mut output = fs::File::create_new(&file).map_err(|e| e.to_string())?;
                let entry = &self.archive.file().entries()[*index];
                let mut reader = self
                    .archive
                    .reader_without_entry(*index)
                    .await
                    .map_err(|e| e.to_string())?;
                let expected = entry.uncompressed_size();
                let mut total = 0;
                let mut buffer = [0; 64 * 1024];
                loop {
                    let read = reader.read(&mut buffer).await.map_err(|e| e.to_string())?;
                    if read == 0 {
                        break;
                    }
                    total += read as u64;
                    if total > expected {
                        return Err("A ZIP file exceeds its declared size.".into());
                    }
                    output
                        .write_all(&buffer[..read])
                        .map_err(|e| e.to_string())?;
                }
                if total != expected || reader.compute_hash() != entry.crc32() {
                    return Err(format!(
                        "The ZIP contains a damaged file: {}",
                        path.display()
                    ));
                }
            }
            Ok(())
        })
    }
}

pub fn inspect(
    bytes: Vec<u8>,
    root: &Path,
    client: Client,
    source: &Definition,
    catalog: &[Definition],
) -> Result<Inspection, String> {
    library::validate_root(root, client).map_err(|e| e.to_string())?;
    if !source.supports(client) {
        return Err("This addon doesn't support the selected game.".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let interface = active_interface(&root, client)?;
    let archive = open_archive(bytes)?;
    block_on(inspect_archive(
        archive, &root, client, interface, source, catalog,
    ))
}

fn open_archive(bytes: Vec<u8>) -> Result<ZipFileReader, String> {
    check_directory(&bytes)?;
    block_on(ZipFileReader::new(bytes)).map_err(|error| format!("Couldn't open the ZIP. {error}"))
}

pub fn inspect_project(
    bytes: Vec<u8>,
    root: &Path,
    client: Client,
    repo: &str,
    asset: String,
    catalog: &[Definition],
) -> Result<(Definition, Inspection), String> {
    library::validate_root(root, client).map_err(|error| error.to_string())?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let interface = active_interface(&root, client)?;
    let archive = open_archive(bytes)?;
    let mut folders = BTreeSet::new();
    for entry in archive.file().entries() {
        let path = entry
            .filename()
            .as_str()
            .map_err(|_| "A ZIP path is not UTF-8.")?;
        let Some((folder, file)) = path.split_once('/') else {
            continue;
        };
        if safe_folder(folder)
            && library::toc_names(folder, client)
                .iter()
                .any(|toc| toc == file)
        {
            folders.insert(folder.to_owned());
        }
    }
    if folders.is_empty() || folders.len() > 64 {
        return Err("Choose a packaged addon ZIP with valid addon folders.".into());
    }
    let repo_name = repo.rsplit('/').next().unwrap_or_default();
    let primary = folders
        .iter()
        .find(|folder| folder.eq_ignore_ascii_case(repo_name))
        .or_else(|| (folders.len() == 1).then(|| folders.first()).flatten())
        .or_else(|| {
            let mut roots = folders.iter().filter(|name| {
                folders.iter().all(|other| {
                    other == *name
                        || other
                            .strip_prefix(name.as_str())
                            .is_some_and(|suffix| suffix.starts_with('_'))
                })
            });
            roots.next()
        })
        .ok_or("Couldn't identify the main addon folder. Choose a ZIP for a single addon.")?
        .clone();
    if let Some(folder) = folders.iter().find(|folder| {
        catalog
            .iter()
            .any(|other| other.supports(client) && other.includes(folder))
    }) {
        return Err(format!("{folder} already has a source in Astral."));
    }
    let source =
        Definition::github_project(repo, client, folders.into_iter().collect(), primary, asset);
    let package = block_on(inspect_archive(
        archive, &root, client, interface, &source, catalog,
    ))?;
    if package.modules.iter().any(|module| module.replaces) {
        return Err("This ZIP would replace an installed addon. Remove the existing copy before installing from this source.".into());
    }
    Ok((source, package))
}

async fn inspect_archive(
    archive: ZipFileReader,
    root: &Path,
    client: Client,
    interface: u32,
    source: &Definition,
    catalog: &[Definition],
) -> Result<Inspection, String> {
    let directory_start = check_directory(archive.data())?;
    let entries = archive.file().entries();
    if entries.is_empty() || entries.len() > MAX_ENTRIES {
        return Err("The ZIP has an unexpected number of entries.".into());
    }
    let mut paths = HashSet::new();
    let mut files = HashMap::new();
    let mut folders = BTreeSet::new();
    let mut expanded = 0u64;
    for (index, entry) in entries.iter().enumerate() {
        let name = entry
            .filename()
            .as_str()
            .map_err(|_| "A ZIP path is not UTF-8.")?;
        let dir = name.ends_with('/');
        let path = name.strip_suffix('/').unwrap_or(name);
        let top = path.split('/').next().unwrap_or_default();
        let kind = (entry.external_file_attribute() >> 16) & 0o170000;
        if !path.split('/').all(safe_component)
            || !safe_folder(top)
            || !source.includes(top)
            || (!dir && !path.contains('/'))
            || !matches!(kind, 0 | 0o040000 | 0o100000)
            || (dir && kind == 0o100000)
            || (!dir && kind == 0o040000)
            || !paths.insert(path.to_ascii_lowercase())
            || entry.header_offset() >= directory_start
            || entry.compressed_size() > archive.data().len() as u64
        {
            return Err(format!(
                "The ZIP contains an unsafe or unexpected path: {name}"
            ));
        }
        if catalog.iter().any(|other| {
            other.id != source.id
                && other.folder.eq_ignore_ascii_case(top)
                && other.supports(client)
        }) {
            return Err(format!("The ZIP contains another project's addon: {top}"));
        }
        expanded = expanded.saturating_add(entry.uncompressed_size());
        if expanded > MAX_EXPANDED {
            return Err("The ZIP exceeds the 512 MB unpacked size limit.".into());
        }
        if !dir {
            folders.insert(top);
            files.insert(path, index);
        }
    }
    if folders.len() > 64 {
        return Err("The ZIP contains more than 64 addon folders.".into());
    }
    if !folders.contains(source.folder.as_str()) {
        return Err("The main addon folder is missing from the ZIP.".into());
    }
    if files.keys().any(|file| {
        file.rsplit_once('/')
            .is_some_and(|(parent, _)| parents(parent).any(|p| files.contains_key(p)))
    }) {
        return Err("A ZIP path is both a file and a directory.".into());
    }

    let addons = root.join("Interface").join("AddOns");
    let mut modules = Vec::new();
    let mut manifests = Vec::new();
    let mut missing: BTreeSet<_> = source.requires.iter().cloned().collect();
    for folder in folders {
        if !for_client(source, folder, client, &addons)? {
            continue;
        }
        let toc = library::toc_names(folder, client)
            .into_iter()
            .find_map(|toc| files.get(format!("{folder}/{toc}").as_str()).copied())
            .ok_or_else(|| format!("Client addon metadata is missing for {folder}."))?;
        // Questie stores its generated Forever database in TOC metadata.
        let maximum =
            if source.id == "questie" && folder == "QuestieDB" && client == Client::Forever {
                MAX_QUESTIE_DB_TOC
            } else {
                MAX_TOC
            };
        let data = String::from_utf8(read_entry(&archive, toc, maximum).await?)
            .map_err(|_| format!("Couldn't read {folder}'s addon metadata."))?;
        let metadata = library::toc_metadata(&data);
        let compatible = metadata.get("Interface").is_some_and(|value| {
            tokens(value)
                .filter_map(|token| token.parse::<u32>().ok())
                .any(|number| {
                    number == interface
                        || (source.allow_retail_patch_mismatch
                            && client == Client::Retail
                            && number / 100 == interface / 100)
                })
        });
        if !compatible {
            if folder == source.folder {
                return Err(format!(
                    "{folder} doesn't list the installed {} build ({interface}).",
                    client.label()
                ));
            }
            continue;
        }
        for key in ["Dependencies", "RequiredDeps"] {
            if let Some(value) = metadata.get(key) {
                for dependency in tokens(value) {
                    if !safe_folder(dependency) {
                        return Err(format!("{folder} has invalid dependency metadata."));
                    }
                    missing.insert(dependency.to_owned());
                }
            }
        }
        let replaces = existing_folder(&addons.join(folder))?;
        let reviewed_version = if replaces {
            installed_module_version(&addons, folder, client)?
        } else {
            None
        };
        modules.push(Module {
            folder: folder.into(),
            version: metadata
                .get("Version")
                .filter(|version| !version.is_empty())
                .cloned(),
            replaces,
            reviewed_version,
        });
        manifests.push((folder, data));
    }
    let provided: HashSet<_> = modules.iter().map(|m| m.folder.as_str()).collect();
    files.retain(|path, _| {
        path.split('/')
            .next()
            .is_some_and(|top| provided.contains(top))
    });
    let folded_files = if root.join("interface").is_dir() {
        files.keys().map(|path| path.to_ascii_lowercase()).collect()
    } else {
        HashSet::new()
    };
    for (folder, manifest) in manifests {
        validate_toc_files(&manifest, folder, client, &files, &folded_files)?;
    }
    missing.retain(|dependency| !provided.contains(dependency.as_str()));
    let unavailable = missing_dependencies(&addons, &missing)?;
    let mut files: Vec<_> = files
        .into_iter()
        .filter(|(path, _)| {
            path.split('/')
                .next()
                .is_some_and(|top| provided.contains(top))
        })
        .map(|(path, index)| (PathBuf::from(path), index))
        .collect();
    files.sort_unstable();
    let installed_version = modules
        .iter()
        .find(|module| module.folder == source.folder && module.replaces)
        .map(|module| source.installed_version(&addons, client, module.reviewed_version.as_deref()))
        .transpose()
        .map_err(|error| error.to_string())?;
    Ok(Inspection {
        interface,
        modules,
        missing: unavailable,
        installed_version,
        root: root.to_path_buf(),
        client,
        dependencies: missing,
        archive,
        files,
    })
}

pub(crate) fn check_directory(bytes: &[u8]) -> Result<u64, String> {
    const END: &[u8] = b"PK\x05\x06";
    const ZIP64_LOCATOR: &[u8] = b"PK\x06\x07";
    const END_LENGTH: usize = 22;
    let tail_start = bytes.len().saturating_sub(END_LENGTH + u16::MAX as usize);
    let end = bytes[tail_start..]
        .windows(4)
        .rposition(|window| window == END)
        .map(|position| tail_start + position)
        .ok_or("Couldn't find the ZIP directory.")?;
    let trailer = bytes
        .get(end..end + END_LENGTH)
        .ok_or("Incomplete ZIP directory.")?;
    let short = |index| u16::from_le_bytes(trailer[index..index + 2].try_into().unwrap()) as usize;
    let long = |index| u32::from_le_bytes(trailer[index..index + 4].try_into().unwrap()) as u64;
    let entries = short(10);
    if end
        .checked_sub(20)
        .is_some_and(|offset| bytes.get(offset..offset + 4) == Some(ZIP64_LOCATOR))
    {
        // Bound allocations before passing the directory to the ZIP reader.
        return Err("ZIP64 packages are not supported in this preview.".into());
    }
    let size = long(12);
    let start = long(16);
    if end + END_LENGTH + short(20) != bytes.len()
        || entries == 0
        || entries > MAX_ENTRIES
        || short(4) != 0
        || short(6) != 0
        || short(8) != entries
        || start + size > end as u64
        || size < entries as u64 * 46
    {
        return Err("The ZIP directory is incomplete or exceeds the package limits.".into());
    }
    Ok(start)
}

fn parents(path: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(path), |part| {
        part.rsplit_once('/').map(|(parent, _)| parent)
    })
}

async fn read_entry(
    archive: &ZipFileReader,
    index: usize,
    maximum: u64,
) -> Result<Vec<u8>, String> {
    let entry = &archive.file().entries()[index];
    if entry.uncompressed_size() > maximum {
        return Err("A file exceeds its allowed size.".into());
    }
    let mut reader = archive
        .reader_without_entry(index)
        .await
        .map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    (&mut reader)
        .take(entry.uncompressed_size() + 1)
        .read_to_end(&mut data)
        .await
        .map_err(|e| e.to_string())?;
    if data.len() as u64 != entry.uncompressed_size() || reader.compute_hash() != entry.crc32() {
        return Err("The ZIP contains a damaged file.".into());
    }
    Ok(data)
}

pub(crate) fn active_interface(root: &Path, client: Client) -> Result<u32, String> {
    let path = root
        .parent()
        .ok_or("Choose the WoW folder first.")?
        .join(".build.info");
    let build = fs::read_to_string(path).map_err(|_| "Couldn't read the installed WoW build.")?;
    let mut rows = build.lines();
    let headers: Vec<_> = rows
        .next()
        .unwrap_or_default()
        .split('|')
        .map(|field| field.split('!').next().unwrap_or_default())
        .collect();
    let col = |name| {
        headers
            .iter()
            .position(|key| *key == name)
            .ok_or("Couldn't identify the active WoW build.")
    };
    let (product, active, version) = (col("Product")?, col("Active")?, col("Version")?);
    let product_id = if client == Client::Forever {
        "wowf"
    } else {
        "wow"
    };
    let build = rows
        .find_map(|row| {
            let columns: Vec<_> = row.split('|').collect();
            (columns.get(product) == Some(&product_id) && columns.get(active) == Some(&"1"))
                .then(|| columns.get(version).copied())
                .flatten()
        })
        .ok_or("Couldn't identify the active WoW build.")?;
    let mut parts = build.split('.').map(str::parse::<u32>);
    let (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch))) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Err("Couldn't identify the active WoW build.".into());
    };
    if major > 999 || minor > 99 || patch > 99 {
        return Err("Couldn't identify the active WoW build.".into());
    }
    Ok(major * 10_000 + minor * 100 + patch)
}

pub(crate) fn existing_folder(path: &Path) -> Result<bool, String> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(format!("{} isn't a regular addon folder.", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn installed_module_version(
    addons: &Path,
    folder: &str,
    client: Client,
) -> Result<Option<String>, String> {
    Ok(library::toc(&addons.join(folder), folder, client)
        .map_err(|error| error.to_string())?
        .and_then(|mut metadata| metadata.remove("Version")))
}

pub(crate) fn missing_dependencies(
    addons: &Path,
    dependencies: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    let mut missing = BTreeSet::new();
    for dependency in dependencies {
        if !safe_folder(dependency) {
            return Err("The addon lists an invalid dependency.".into());
        }
        if !existing_folder(&addons.join(dependency))? {
            missing.insert(dependency.clone());
        }
    }
    Ok(missing)
}

pub(crate) fn safe_folder(value: &str) -> bool {
    safe_component(value)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'!'))
}

pub(crate) fn safe_component(value: &str) -> bool {
    if value.is_empty()
        || value.ends_with(['.', ' '])
        || value.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
        })
    {
        return false;
    }
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                suffix.chars().count() == 1
                    && suffix
                        .chars()
                        .any(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³'))
            })
}

pub(crate) fn tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|v| !v.is_empty())
}

fn for_client(
    source: &Definition,
    folder: &str,
    client: Client,
    addons: &Path,
) -> Result<bool, String> {
    Ok(match (source.id.as_str(), client) {
        ("ellesmere", Client::Forever) => !matches!(
            folder,
            "EllesmereUIMythicTimer"
                | "EllesmereUIFriends"
                | "EllesmereUILoadoutManager"
                | "EllesmereUIGroupFinder"
        ),
        ("ellesmere", Client::Retail) => folder != "EllesmereUIForeverEssentials",
        ("rio", _) => {
            matches!(
                folder,
                "RaiderIO" | "RaiderIO_DB_EU_M" | "RaiderIO_DB_EU_R" | "RaiderIO_DB_EU_F"
            ) || existing_folder(&addons.join(folder))?
        }
        _ => true,
    })
}

fn validate_toc_files(
    toc: &str,
    folder: &str,
    client: Client,
    files: &HashMap<&str, usize>,
    folded_files: &HashSet<String>,
) -> Result<(), String> {
    // Match the client's TOC directives before checking the paths it will load.
    const LOCALES: [&str; 11] = [
        "enUS", "deDE", "esES", "esMX", "frFR", "itIT", "koKR", "ptBR", "ruRU", "zhCN", "zhTW",
    ];
    let conditions =
        regex::Regex::new(r"\s*\[((?:Allow|Exclude)Load(?:GameType|TextLocale))\s+([^\]]+)\]")
            .unwrap();
    let game_types: &[&str] = if client == Client::Forever {
        &["camelot"]
    } else {
        &["standard", "mainline"]
    };
    let family = if client == Client::Forever {
        "Classic"
    } else {
        "Mainline"
    };
    for line in toc
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let directives: HashMap<_, _> = conditions
            .captures_iter(line)
            .map(|m| (m.get(1).unwrap().as_str(), m.get(2).unwrap().as_str()))
            .collect();
        if directives
            .get("AllowLoadGameType")
            .is_some_and(|v| !tokens(v).any(|v| game_types.contains(&v)))
            || directives
                .get("ExcludeLoadGameType")
                .is_some_and(|v| tokens(v).any(|v| game_types.contains(&v)))
        {
            continue;
        }
        let name = conditions
            .replace_all(line, "")
            .trim()
            .replace('\\', "/")
            .replace("[Family]", family)
            .replace(
                "[Game]",
                if client == Client::Forever {
                    "Camelot"
                } else {
                    "Standard"
                },
            );
        let variants = if name.contains("[TextLocale]") {
            let locales = directives
                .get("AllowLoadTextLocale")
                .map_or_else(|| LOCALES.to_vec(), |v| tokens(v).collect());
            if locales.iter().any(|v| !LOCALES.contains(v)) {
                return Err(format!("{folder} lists an unsupported text locale."));
            }
            locales
                .into_iter()
                .filter(|locale| {
                    !directives
                        .get("ExcludeLoadTextLocale")
                        .is_some_and(|v| tokens(v).any(|v| v == *locale))
                })
                .map(|locale| name.replace("[TextLocale]", locale))
                .collect()
        } else {
            vec![name]
        };
        for variant in variants {
            let mut parts = vec![folder];
            for part in variant.split('/') {
                match part {
                    ".." if !parts.is_empty() => {
                        parts.pop();
                    }
                    "." => (),
                    value if safe_component(value) => parts.push(value),
                    _ => return Err(format!("{folder} lists an unsafe file: {line}")),
                }
            }
            let relative = parts.join("/");
            if !files.contains_key(relative.as_str())
                && !folded_files.contains(&relative.to_ascii_lowercase())
            {
                return Err(format!(
                    "{folder} is missing a file listed in its TOC: {line}"
                ));
            }
        }
    }
    Ok(())
}
