use crate::{
    library::{self, Addon, Client, Definition},
    packages::{self, Inspection},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const BACKUP_DIR: &str = ".astral-native-backups";
const PENDING: &str = "pending.json";
const MANIFEST: &str = "manifest.json";
const RECOVERED: &str = "recovered.json";

#[derive(Serialize, Deserialize)]
pub struct Transaction {
    pub addon: String,
    pub release: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    pub created: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) created_ms: Option<u64>,
    pub folders: Vec<String>,
    pub previous: Vec<String>,
    #[serde(default)]
    pub restored: bool,
    #[serde(default)]
    pub removed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_switch: Option<ProfileSwitch>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProfileSwitch {
    pub account: String,
    pub id: uuid::Uuid,
}

pub struct Backup {
    pub path: PathBuf,
    pub transaction: Transaction,
    pub interrupted: bool,
    pub recovered: bool,
    pub electron: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElectronManifest {
    id: String,
    root: PathBuf,
    flavor: Client,
    addon_id: String,
    name: String,
    version: Option<String>,
    release: String,
    #[serde(default)]
    notes: String,
    definition: Option<Definition>,
    folders: Vec<String>,
    previous_folders: Vec<String>,
}

fn read_manifest<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let metadata = path.symlink_metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return Err("Invalid backup manifest.".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn read_transaction(path: &Path) -> Result<Transaction, String> {
    let record = read_manifest(path)?;
    validate_transaction(&record)?;
    Ok(record)
}

fn validate_transaction(record: &Transaction) -> Result<(), String> {
    let unique: HashSet<_> = record
        .folders
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let old: HashSet<_> = record.previous.iter().collect();
    if record.folders.is_empty()
        || record.folders.len() > 64
        || unique.len() != record.folders.len()
        || old.len() != record.previous.len()
        || !record
            .folders
            .iter()
            .all(|name| packages::safe_folder(name))
        || !record
            .previous
            .iter()
            .all(|name| record.folders.contains(name))
    {
        return Err("Invalid backup folders.".into());
    }
    Ok(())
}

pub fn list(root: &Path, client: Client) -> Result<Vec<Backup>, String> {
    let paths = Paths::open(root, client)?;
    if !packages::existing_folder(&paths.backups)? {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&paths.backups).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let path = entry.path();
        let interrupted = path.join(PENDING).symlink_metadata().is_ok();
        let recovered = path.join(RECOVERED).symlink_metadata().is_ok();
        let manifest = path.join(if interrupted {
            PENDING
        } else if recovered {
            RECOVERED
        } else {
            MANIFEST
        });
        if manifest.symlink_metadata().is_ok() {
            let transaction = read_transaction(&manifest)?;
            records.push(Backup {
                path,
                transaction,
                interrupted,
                recovered,
                electron: false,
            });
        }
    }
    sort_backups(&mut records);
    Ok(records)
}

pub fn open_backup_folder(root: &Path, client: Client) -> Result<PathBuf, String> {
    let paths = Paths::open(root, client)?;
    paths.ensure_backup_folder()?;
    Ok(paths.backups)
}

pub(crate) fn ensure_no_pending_profile_packages(root: &Path) -> Result<(), String> {
    let folder = root.join("Interface").join(BACKUP_DIR);
    if !packages::existing_folder(&folder)? {
        return Ok(());
    }
    for entry in fs::read_dir(folder).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            let pending = entry.path().join(PENDING);
            if pending.symlink_metadata().is_ok()
                && read_transaction(&pending)?.profile_switch.is_some()
            {
                return Err(
                    "Recover the interrupted Handheld setup in Activity & backups first.".into(),
                );
            }
        }
    }
    Ok(())
}

pub fn sort_backups(records: &mut [Backup]) {
    let millis = |backup: &Backup| {
        backup
            .transaction
            .created_ms
            .unwrap_or_else(|| backup.transaction.created.saturating_mul(1000))
    };
    records.sort_by(|a, b| {
        b.interrupted
            .cmp(&a.interrupted)
            .then_with(|| millis(b).cmp(&millis(a)))
            .then_with(|| b.path.cmp(&a.path))
    });
}

pub fn list_electron(
    archive: &Path,
    root: &Path,
    client: Client,
    catalog: &[Definition],
) -> Result<Vec<Backup>, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    if !packages::existing_folder(archive)? {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(archive).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            continue;
        }
        if let Ok(Some(record)) = read_electron_backup(&entry.path(), &root, client, catalog) {
            records.push(record);
        }
    }
    Ok(records)
}

fn read_electron_backup(
    path: &Path,
    root: &Path,
    client: Client,
    catalog: &[Definition],
) -> Result<Option<Backup>, String> {
    let manifest: ElectronManifest = read_manifest(&path.join(MANIFEST))?;
    if manifest.flavor != client || manifest.root.canonicalize().ok().as_deref() != Some(root) {
        return Ok(None);
    }
    let (created, id) = manifest.id.split_once('-').ok_or("Invalid backup ID.")?;
    let created_ms = created.parse::<u64>().map_err(|_| "Invalid backup date.")?;
    if id != manifest.addon_id
        || path.file_name().and_then(|name| name.to_str()) != Some(&manifest.id)
    {
        return Err("Invalid backup ID.".into());
    }
    let source = catalog
        .iter()
        .find(|source| source.id == id && source.supports(client))
        .or_else(|| {
            let hash = id.strip_prefix("gh-")?;
            let source = manifest.definition.as_ref()?;
            (hash.len() == 16
                && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                && source.id == id
                && packages::safe_folder(&source.folder))
            .then_some(source)
        })
        .ok_or("Unknown addon backup source.")?;
    if !manifest.folders.iter().all(|folder| {
        source.includes(folder)
            || (id == "rio"
                && ["EU", "US", "KR", "TW", "CN"].into_iter().any(|region| {
                    ["M", "R", "F"]
                        .into_iter()
                        .any(|kind| folder == &format!("RaiderIO_DB_{region}_{kind}"))
                }))
    }) {
        return Err("The backup contains unexpected addon folders.".into());
    }
    let transaction = Transaction {
        addon: manifest.name,
        release: manifest.release,
        previous_version: manifest.version,
        notes: manifest.notes,
        created: created_ms / 1000,
        created_ms: Some(created_ms),
        folders: manifest.folders,
        previous: manifest.previous_folders,
        restored: false,
        removed: false,
        profile_switch: None,
    };
    validate_transaction(&transaction)?;
    Ok(Some(Backup {
        path: path.to_path_buf(),
        transaction,
        interrupted: false,
        recovered: false,
        electron: true,
    }))
}

struct Paths {
    interface: PathBuf,
    addons: PathBuf,
    backups: PathBuf,
}

struct InstallLock(fs::File);

impl Drop for InstallLock {
    fn drop(&mut self) {
        // A concurrent process launch can briefly inherit the file descriptor.
        // Release the lock when the transaction ends even if that descriptor is still open.
        let _ = self.0.unlock();
    }
}

impl Paths {
    fn open(root: &Path, client: Client) -> Result<Self, String> {
        library::validate_root(root, client).map_err(|e| e.to_string())?;
        let interface = root
            .canonicalize()
            .map_err(|e| e.to_string())?
            .join("Interface");
        let addons = interface.join("AddOns");
        if !packages::existing_folder(&interface)? || !packages::existing_folder(&addons)? {
            return Err("The WoW addon folder has moved. Review the release again.".into());
        }
        let backups = interface.join(BACKUP_DIR);
        Ok(Self {
            interface,
            addons,
            backups,
        })
    }

    fn stage(&self) -> Result<tempfile::TempDir, String> {
        tempfile::Builder::new()
            .prefix(".astral-native-stage-")
            .tempdir_in(&self.interface)
            .map_err(|e| e.to_string())
    }

    fn ensure_backup_folder(&self) -> Result<(), String> {
        if !packages::existing_folder(&self.backups)? {
            match fs::create_dir(&self.backups) {
                Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e.to_string()),
                _ => {}
            }
            if !packages::existing_folder(&self.backups)? {
                return Err("Couldn't create the addon backup folder.".into());
            }
        }
        Ok(())
    }

    fn lock(&self) -> Result<InstallLock, String> {
        self.ensure_backup_folder()?;
        let lock_path = self.backups.join("install.lock");
        if lock_path
            .symlink_metadata()
            .is_ok_and(|meta| !meta.is_file())
        {
            return Err("Couldn't use the addon installation lock.".into());
        }
        let lock = fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|e| e.to_string())?;
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => {
                "Another Astral operation is using this WoW folder.".into()
            }
            fs::TryLockError::Error(error) => {
                format!("Couldn't lock the addon installation. {error}")
            }
        })?;
        Ok(InstallLock(lock))
    }

    fn ensure_clean(&self) -> Result<(), String> {
        for entry in fs::read_dir(&self.backups).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.path().join(PENDING).is_file() {
                return Err(format!(
                    "An interrupted operation needs attention. Recovery files: {}",
                    entry.path().display()
                ));
            }
        }
        Ok(())
    }
}

fn timestamp() -> Result<u64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    u64::try_from(elapsed.as_millis()).map_err(|e| e.to_string())
}

pub fn install(
    package: &Inspection,
    addon: &str,
    release: &str,
    notes: &str,
) -> Result<PathBuf, String> {
    install_with(package, addon, (release, notes), game_closed, |from, to| {
        fs::rename(from, to)
    })
}

pub fn refresh_talents(
    root: &Path,
    fetch: impl FnOnce(u32) -> Result<String, String>,
) -> Result<bool, String> {
    refresh_talents_with(root, fetch, game_closed)
}

fn refresh_talents_with(
    root: &Path,
    fetch: impl FnOnce(u32) -> Result<String, String>,
    ensure_game_closed: impl Fn() -> Result<(), String>,
) -> Result<bool, String> {
    ensure_game_closed()?;
    let paths = Paths::open(root, Client::Retail)?;
    let folder = paths.addons.join("AstralTalents");
    let file = folder.join("Builds.lua");
    let verify_target = || {
        if packages::existing_folder(&folder)?
            && file.symlink_metadata().is_ok_and(|meta| meta.is_file())
        {
            Ok(())
        } else {
            Err("Reinstall Astral Talents to restore its build file.".to_owned())
        }
    };
    verify_target()?;
    let interface = packages::active_interface(root, Client::Retail)?;
    let data = fetch(interface)?;
    ensure_game_closed()?;
    let _lock = paths.lock()?;
    paths.ensure_clean()?;
    verify_target()?;
    if packages::active_interface(root, Client::Retail)? != interface {
        return Err("WoW updated during the talent refresh. Try again.".into());
    }
    let previous = fs::read_to_string(&file).map_err(|error| error.to_string())?;
    if data == previous {
        return Ok(false);
    }
    let checked = regex::Regex::new(r"(?m)^ns\.checked = .*$").expect("valid checked-date pattern");
    let changed = checked.replace(&data, "") != checked.replace(&previous, "");
    let mut staged =
        tempfile::NamedTempFile::new_in(&paths.interface).map_err(|e| e.to_string())?;
    use std::io::Write;
    staged
        .write_all(data.as_bytes())
        .map_err(|e| e.to_string())?;
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    ensure_game_closed()?;
    staged.persist(file).map_err(|e| e.to_string())?;
    Ok(changed)
}

fn install_with(
    package: &Inspection,
    addon: &str,
    (release, notes): (&str, &str),
    ensure_game_closed: impl Fn() -> Result<(), String>,
    move_into_place: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<PathBuf, String> {
    ensure_game_closed()?;
    let paths = Paths::open(&package.root, package.client)?;
    let _lock = paths.lock()?;
    paths.ensure_clean()?;
    let stage = paths.stage()?;
    package.stage(stage.path())?;

    ensure_game_closed()?;
    if packages::active_interface(&package.root, package.client)? != package.interface {
        return Err("WoW updated since the package check. Review the release again.".into());
    }
    let missing = packages::missing_dependencies(&paths.addons, &package.dependencies)?;
    if !missing.is_empty() {
        return Err(format!(
            "Required addons missing: {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    let mut previous = Vec::new();
    let folders: Vec<_> = package
        .modules
        .iter()
        .map(|module| module.folder.clone())
        .collect();
    for module in &package.modules {
        let exists = packages::existing_folder(&paths.addons.join(&module.folder))?;
        if exists != module.replaces {
            return Err("The installed addon folders changed. Review the release again.".into());
        }
        if exists {
            let current =
                packages::installed_module_version(&paths.addons, &module.folder, package.client)?;
            if current != module.reviewed_version {
                return Err("An installed addon version changed. Review the release again.".into());
            }
            previous.push(module.folder.clone());
        }
    }

    let created_ms = timestamp()?;
    let transaction = Transaction {
        addon: addon.into(),
        release: release.into(),
        previous_version: package.installed_version.clone(),
        notes: notes.chars().take(12_000).collect(),
        created: created_ms / 1000,
        created_ms: Some(created_ms),
        folders,
        previous,
        restored: false,
        removed: false,
        profile_switch: None,
    };
    apply_transaction(
        &paths,
        stage.path(),
        &transaction,
        &transaction.folders,
        move_into_place,
    )
}

pub(crate) fn install_profile_packages(
    root: &Path,
    client: Client,
    packages: &[Inspection],
    profile_switch: ProfileSwitch,
    check: impl Fn() -> io::Result<()>,
    finish: impl FnOnce() -> io::Result<()>,
) -> Result<PathBuf, String> {
    let paths = Paths::open(root, client)?;
    let _lock = paths.lock()?;
    paths.ensure_clean()?;
    let stage = paths.stage()?;
    let mut folders = Vec::new();
    let mut keys = HashSet::new();
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let interface = packages::active_interface(root, client)?;
    for package in packages {
        if package.root != canonical_root
            || package.interface != interface
            || package.client != client
        {
            return Err("WoW updated since the package check. Review the profile again.".into());
        }
        for module in &package.modules {
            if module.replaces
                || !keys.insert(module.folder.to_ascii_lowercase())
                || packages::existing_folder(&paths.addons.join(&module.folder))?
            {
                return Err("An addon folder changed. Review the profile again.".into());
            }
            folders.push(module.folder.clone());
        }
        package.stage(stage.path())?;
    }
    if folders.is_empty() || folders.len() > 64 {
        return Err("Unexpected number of required addon folders.".into());
    }
    for package in packages {
        let external = package
            .dependencies
            .iter()
            .filter(|name| !keys.contains(&name.to_ascii_lowercase()))
            .cloned()
            .collect();
        let missing = packages::missing_dependencies(&paths.addons, &external)?;
        if !missing.is_empty() {
            return Err(format!(
                "Required addons missing: {}",
                missing.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    check().map_err(|error| error.to_string())?;
    if packages::active_interface(root, client)? != interface {
        return Err("WoW updated since the package check. Review the profile again.".into());
    }
    let created_ms = timestamp()?;
    let transaction = Transaction {
        addon: "Handheld profile addons".into(),
        release: "Profile setup".into(),
        previous_version: None,
        notes: String::new(),
        created: created_ms / 1000,
        created_ms: Some(created_ms),
        folders,
        previous: Vec::new(),
        restored: false,
        removed: false,
        profile_switch: Some(profile_switch),
    };
    apply_transaction_with(
        &paths,
        stage.path(),
        &transaction,
        &transaction.folders,
        |from, to| {
            if to.symlink_metadata().is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "An addon folder appeared during installation.",
                ));
            }
            fs::rename(from, to)
        },
        finish,
    )
}

pub fn remove(
    root: &Path,
    client: Client,
    reviewed: &Addon,
    catalog: &[Definition],
) -> Result<PathBuf, String> {
    remove_with(root, client, reviewed, catalog, game_closed)
}

fn remove_with(
    root: &Path,
    client: Client,
    reviewed: &Addon,
    catalog: &[Definition],
    ensure_game_closed: impl Fn() -> Result<(), String>,
) -> Result<PathBuf, String> {
    ensure_game_closed()?;
    let paths = Paths::open(root, client)?;
    let _lock = paths.lock()?;
    paths.ensure_clean()?;
    let installed = library::scan(root, client, catalog).map_err(|error| error.to_string())?;
    let addon = installed
        .iter()
        .find(|addon| addon.folder == reviewed.folder)
        .filter(|addon| addon.modules == reviewed.modules && addon.version == reviewed.version)
        .ok_or("The installed addon changed. Refresh the library and try again.")?;
    if addon.modules.is_empty()
        || addon.modules.len() > 64
        || addon
            .modules
            .iter()
            .any(|name| !packages::safe_folder(name))
    {
        return Err("This addon contains unsupported folder names.".into());
    }
    let removed: HashSet<_> = addon
        .modules
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    if removed.len() != addon.modules.len() {
        return Err("This addon contains conflicting folder names.".into());
    }
    let mut dependents = std::collections::BTreeSet::new();
    for other in &installed {
        for module in &other.modules {
            if removed.contains(&module.to_ascii_lowercase()) {
                continue;
            }
            let Some(metadata) = library::toc(&paths.addons.join(module), module, client)
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            let dependencies = ["Dependencies", "RequiredDeps"]
                .into_iter()
                .filter_map(|key| metadata.get(key))
                .flat_map(|value| packages::tokens(value));
            if dependencies
                .into_iter()
                .any(|name| removed.contains(&name.to_ascii_lowercase()))
            {
                dependents.insert(other.name.as_str());
            }
        }
    }
    if !dependents.is_empty() {
        return Err(format!(
            "Required by {}. Remove those addons first.",
            dependents.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    let created_ms = timestamp()?;
    let transaction = Transaction {
        addon: addon.name.clone(),
        release: addon.version.clone(),
        previous_version: None,
        notes: String::new(),
        created: created_ms / 1000,
        created_ms: Some(created_ms),
        folders: addon.modules.clone(),
        previous: addon.modules.clone(),
        restored: false,
        removed: true,
        profile_switch: None,
    };
    let stage = paths.stage()?;
    ensure_game_closed()?;
    apply_transaction(&paths, stage.path(), &transaction, &[], |a, b| {
        fs::rename(a, b)
    })
}

pub fn restore(root: &Path, client: Client, backup: &Path) -> Result<(), String> {
    restore_with(root, client, backup, game_closed)
}

fn restore_with(
    root: &Path,
    client: Client,
    backup: &Path,
    ensure_game_closed: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    ensure_game_closed()?;
    let paths = Paths::open(root, client)?;
    let _lock = paths.lock()?;
    paths.ensure_clean()?;
    if backup.parent() != Some(paths.backups.as_path()) || !packages::existing_folder(backup)? {
        return Err("This backup belongs to a different WoW folder.".into());
    }
    let original = read_transaction(&backup.join(MANIFEST))?;
    if original.previous.is_empty() && original.profile_switch.is_some() {
        return Err("This backup has no previous addon files.".into());
    }
    let prior = backup.join("previous");
    if !packages::existing_folder(&prior)? {
        return Err("Previous addon files are missing.".into());
    }
    restore_transaction(&paths, original, &prior, ensure_game_closed)
}

pub fn restore_electron(
    root: &Path,
    client: Client,
    backup: &Path,
    catalog: &[Definition],
) -> Result<(), String> {
    restore_electron_with(root, client, backup, catalog, game_closed)
}

fn restore_electron_with(
    root: &Path,
    client: Client,
    backup: &Path,
    catalog: &[Definition],
    ensure_game_closed: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    ensure_game_closed()?;
    let paths = Paths::open(root, client)?;
    let root = paths
        .interface
        .parent()
        .ok_or("The WoW folder has moved.")?;
    if !packages::existing_folder(backup)? {
        return Err("The earlier addon backup is missing.".into());
    }
    let archive = backup
        .parent()
        .ok_or("The earlier addon backup is missing.")?;
    let data = archive
        .parent()
        .ok_or("The earlier addon backup is missing.")?;
    if archive.file_name().and_then(|name| name.to_str()) != Some("backups")
        || !packages::existing_folder(archive)?
    {
        return Err("The earlier addon backup is missing.".into());
    }
    let electron_lock = data.join("install.lock");
    fs::create_dir(&electron_lock).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            "Another Astral operation is using the earlier backup archive.".into()
        } else {
            error.to_string()
        }
    })?;
    let result = (|| {
        let _lock = paths.lock()?;
        paths.ensure_clean()?;
        let original = read_electron_backup(backup, root, client, catalog)?
            .ok_or("This backup belongs to a different WoW folder.")?;
        restore_transaction(&paths, original.transaction, backup, ensure_game_closed)
    })();
    let _ = fs::remove_dir(electron_lock);
    result
}

fn restore_transaction(
    paths: &Paths,
    original: Transaction,
    prior: &Path,
    ensure_game_closed: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    let stage = paths.stage()?;
    for folder in &original.previous {
        copy_tree(&prior.join(folder), &stage.path().join(folder)).map_err(|e| e.to_string())?;
    }

    ensure_game_closed()?;
    let mut previous = Vec::new();
    for folder in &original.folders {
        if packages::existing_folder(&paths.addons.join(folder))? {
            previous.push(folder.clone());
        }
    }
    let created_ms = timestamp()?;
    let transaction = Transaction {
        addon: original.addon,
        release: original.release,
        previous_version: None,
        notes: String::new(),
        created: created_ms / 1000,
        created_ms: Some(created_ms),
        folders: original.folders,
        previous,
        restored: true,
        removed: false,
        profile_switch: None,
    };
    apply_transaction(
        paths,
        stage.path(),
        &transaction,
        &original.previous,
        |a, b| fs::rename(a, b),
    )?;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.symlink_metadata()?.is_dir() {
        return Err(io::Error::other("A backup folder is missing or linked."));
    }
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((source, destination)) = pending.pop() {
        fs::create_dir(&destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let target = destination.join(entry.file_name());
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push((entry.path(), target));
            } else if kind.is_file() {
                fs::copy(entry.path(), target)?;
            } else {
                return Err(io::Error::other(
                    "The backup contains a linked or special file.",
                ));
            }
        }
    }
    Ok(())
}

pub fn recover(root: &Path, client: Client, backup: &Path) -> Result<(), String> {
    recover_with(root, client, backup, game_closed, |_| {
        Err("Recover this operation through its WoW profile.".into())
    })
}

pub(crate) fn recover_with(
    root: &Path,
    client: Client,
    backup: &Path,
    ensure_game_closed: impl Fn() -> Result<(), String>,
    resolve_profile: impl FnOnce(&ProfileSwitch) -> Result<bool, String>,
) -> Result<(), String> {
    ensure_game_closed()?;
    let paths = Paths::open(root, client)?;
    let _lock = paths.lock()?;
    if backup.parent() != Some(paths.backups.as_path()) || !packages::existing_folder(backup)? {
        return Err("This backup belongs to a different WoW folder.".into());
    }
    let pending = backup.join(PENDING);
    let record = read_transaction(&pending)?;
    if let Some(profile_switch) = &record.profile_switch
        && resolve_profile(profile_switch)?
    {
        return fs::rename(&pending, backup.join(MANIFEST)).map_err(|error| error.to_string());
    }
    let prior = backup.join("previous");
    if !packages::existing_folder(&prior)? {
        return Err("Previous addon files are missing.".into());
    }
    let recovered = backup.join("interrupted-files");
    if !packages::existing_folder(&recovered)? {
        fs::create_dir(&recovered).map_err(|e| e.to_string())?;
    }
    let mut moves = Vec::new();
    for folder in &record.folders {
        let old = prior.join(folder);
        let live = paths.addons.join(folder);
        let saved = recovered.join(folder);
        let old_exists = packages::existing_folder(&old)?;
        let live_exists = packages::existing_folder(&live)?;
        if record.previous.contains(folder) {
            if !old_exists {
                if !live_exists {
                    return Err(format!("The previous files for {folder} are missing."));
                }
                continue;
            }
        } else if old_exists {
            return Err(format!("Unexpected recovery files for {folder}."));
        }
        if live_exists && packages::existing_folder(&saved)? {
            return Err(format!("Two copies of {folder} need manual review."));
        }
        moves.push((old, live, saved, old_exists, live_exists));
    }
    ensure_game_closed()?;
    // Each move can be resumed. If a previous folder is back in AddOns, it
    // will be left in place on the next attempt.
    for (old, live, saved, old_exists, live_exists) in moves {
        if live_exists {
            fs::rename(&live, saved).map_err(|e| e.to_string())?;
        }
        if old_exists {
            fs::rename(old, live).map_err(|e| e.to_string())?;
        }
    }
    fs::rename(pending, backup.join(RECOVERED)).map_err(|e| e.to_string())
}

fn apply_transaction(
    paths: &Paths,
    stage: &Path,
    transaction: &Transaction,
    incoming: &[String],
    move_into_place: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<PathBuf, String> {
    apply_transaction_with(paths, stage, transaction, incoming, move_into_place, || {
        Ok(())
    })
}

fn apply_transaction_with(
    paths: &Paths,
    stage: &Path,
    transaction: &Transaction,
    incoming: &[String],
    move_into_place: impl Fn(&Path, &Path) -> io::Result<()>,
    finish: impl FnOnce() -> io::Result<()>,
) -> Result<PathBuf, String> {
    let backup = tempfile::Builder::new()
        .prefix("release-")
        .tempdir_in(&paths.backups)
        .map_err(|e| e.to_string())?;
    let prior = backup.path().join("previous");
    fs::create_dir(&prior).map_err(|e| e.to_string())?;
    let pending = backup.path().join(PENDING);
    let manifest = serde_json::to_vec_pretty(&transaction).map_err(|e| e.to_string())?;
    let mut journal = fs::File::create_new(&pending).map_err(|e| e.to_string())?;
    journal.write_all(&manifest).map_err(|e| e.to_string())?;
    journal.sync_all().map_err(|e| e.to_string())?;
    drop(journal);
    // Keep recovery files even if the process exits or a rollback can't finish.
    let backup = backup.keep();

    let mut moved = Vec::new();
    let mut installed = Vec::new();
    let swap = (|| -> io::Result<()> {
        for folder in &transaction.folders {
            let target = paths.addons.join(folder);
            if transaction.previous.contains(folder) {
                fs::rename(&target, prior.join(folder))?;
                moved.push(folder);
            }
            if incoming.contains(folder) {
                move_into_place(&stage.join(folder), &target)?;
                installed.push(folder);
            }
        }
        if transaction.profile_switch.is_some() {
            finish()
        } else {
            fs::rename(&pending, backup.join(MANIFEST))
        }
    })();
    if let Err(error) = swap {
        let mut recovered = true;
        for folder in installed.iter().rev() {
            if fs::rename(paths.addons.join(folder), stage.join(folder)).is_err() {
                recovered = false;
            }
        }
        for folder in moved.iter().rev() {
            let target = paths.addons.join(folder);
            if target.symlink_metadata().is_ok() || fs::rename(prior.join(folder), target).is_err()
            {
                recovered = false;
            }
        }
        if recovered {
            let _ = fs::remove_dir_all(&backup);
            return Err(format!(
                "Operation failed. Previous files were restored. {error}"
            ));
        }
        if let Err(journal_error) = fs::rename(backup.join(MANIFEST), &pending)
            && !pending.exists()
        {
            return Err(format!(
                "Operation stopped during rollback. Recovery files: {}. {journal_error}",
                backup.display()
            ));
        }
        return Err(format!(
            "Operation stopped during rollback. Recovery files: {}",
            backup.display()
        ));
    }

    if transaction.profile_switch.is_some() {
        fs::rename(&pending, backup.join(MANIFEST)).map_err(|error| {
            format!(
                "The profile was saved. Finish the addon operation in Activity & backups. {error}"
            )
        })?;
    }
    Ok(backup)
}

pub(crate) fn game_running() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    let running = {
        use std::os::windows::process::CommandExt;
        let output = Command::new("tasklist.exe")
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|_| "Couldn't check whether WoW is running.")?;
        if !output.status.success() {
            return Err("Couldn't check whether WoW is running.".into());
        }
        String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            line.split('"').nth(1).is_some_and(|name| {
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    "wow.exe" | "wow-64.exe" | "wowclassic.exe" | "wowf.exe" | "wowforever.exe"
                )
            })
        })
    };
    #[cfg(not(target_os = "windows"))]
    let running = {
        let output = Command::new("/usr/bin/pgrep")
            .args(["-x", "Wow|World of Warcraft|WowClassic|WowF|WowForever"])
            .output()
            .map_err(|_| "Couldn't check whether WoW is running.")?;
        match output.status.code() {
            Some(0) => true,
            Some(1) if output.stderr.is_empty() => false,
            _ => return Err("Couldn't check whether WoW is running.".into()),
        }
    };
    Ok(running)
}

pub(crate) fn game_closed() -> Result<(), String> {
    if game_running()? {
        Err("Close World of Warcraft before changing addon files.".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{self, Client, Definition};
    use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};
    use futures_lite::future::block_on;
    use std::cell::Cell;

    #[test]
    fn backups_in_the_same_second_sort_by_time_before_folder_name() {
        let backup = |path: &str, created_ms| Backup {
            path: path.into(),
            transaction: Transaction {
                addon: "KeyUI".into(),
                release: "v2".into(),
                previous_version: None,
                notes: String::new(),
                created: 123,
                created_ms,
                folders: vec!["KeyUI".into()],
                previous: vec!["KeyUI".into()],
                restored: false,
                removed: false,
                profile_switch: None,
            },
            interrupted: false,
            recovered: false,
            electron: false,
        };
        let mut records = [
            backup("release-z", Some(123_100)),
            backup("release-y", None),
            backup("release-a", Some(123_900)),
        ];
        sort_backups(&mut records);
        assert_eq!(
            records.map(|record| record.path),
            ["release-a", "release-z", "release-y"].map(PathBuf::from),
        );
    }

    fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
        block_on(async {
            let mut bytes = Vec::new();
            let mut writer = ZipFileWriter::new(&mut bytes);
            for (path, contents) in entries {
                writer
                    .write_entry_whole(
                        ZipEntryBuilder::new((*path).into(), Compression::Deflate),
                        contents.as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            writer.close().await.unwrap();
            bytes
        })
    }

    fn game() -> (tempfile::TempDir, PathBuf, Vec<Definition>) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("_retail_");
        let addons = root.join("Interface/AddOns");
        fs::create_dir_all(addons.join("KeyUI")).unwrap();
        fs::create_dir_all(root.join("WTF")).unwrap();
        fs::write(
            temp.path().join(".build.info"),
            "Product!STRING:0|Active!DEC:1|Version!STRING:0\nwow|1|12.1.5.123",
        )
        .unwrap();
        fs::write(addons.join("KeyUI/old.lua"), "-- original").unwrap();
        fs::write(root.join("WTF/config.wtf"), "settings").unwrap();
        (temp, root, library::catalog())
    }

    fn keyui_release() -> Vec<u8> {
        zip(&[
            (
                "KeyUI/KeyUI.toc",
                "## Interface: 120105\n## Version: 2\nnew.lua\n",
            ),
            ("KeyUI/new.lua", "-- new release"),
            (
                "KeyUI_Extra/KeyUI_Extra.toc",
                "## Interface: 120105\n## Dependencies: KeyUI\n",
            ),
        ])
    }

    #[test]
    fn profile_package_batch_rolls_back_all_modules_when_profile_apply_fails() {
        let (_temp, root, mut catalog) = game();
        for folder in ["FixtureCore", "FixtureBar"] {
            catalog.push(Definition::github_project(
                &format!("example/{folder}"),
                Client::Retail,
                vec![folder.into()],
                folder.into(),
                ".*".into(),
            ));
        }
        let core = packages::inspect(
            zip(&[(
                "FixtureCore/FixtureCore.toc",
                "## Interface: 120105\n## Version: 1\n",
            )]),
            &root,
            Client::Retail,
            &catalog[catalog.len() - 2],
            &catalog,
        )
        .unwrap();
        let bars = packages::inspect(
            zip(&[(
                "FixtureBar/FixtureBar.toc",
                "## Interface: 120105\n## Version: 1\n## Dependencies: FixtureCore\n",
            )]),
            &root,
            Client::Retail,
            &catalog[catalog.len() - 1],
            &catalog,
        )
        .unwrap();
        let addons = root.join("Interface/AddOns");
        let packages = [core, bars];
        let operation = ProfileSwitch {
            account: "Test".into(),
            id: uuid::Uuid::new_v4(),
        };
        let error = install_profile_packages(
            &root,
            Client::Retail,
            &packages,
            operation.clone(),
            || Ok(()),
            || {
                assert!(addons.join("FixtureCore").is_dir() && addons.join("FixtureBar").is_dir());
                Err(io::Error::other("Could not save profile."))
            },
        )
        .unwrap_err();
        assert!(error.contains("Could not save profile"));
        assert!(!addons.join("FixtureCore").exists() && !addons.join("FixtureBar").exists());
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
        assert!(list(&root, Client::Retail).unwrap().is_empty());
        install_profile_packages(
            &root,
            Client::Retail,
            &packages,
            operation,
            || Ok(()),
            || Ok(()),
        )
        .unwrap();
        assert!(addons.join("FixtureCore").is_dir() && addons.join("FixtureBar").is_dir());
    }

    #[test]
    fn electron_backup_restores_modules_and_keeps_a_native_undo() {
        let (temp, root, catalog) = game();
        let archive = temp.path().join("astral-companion/backups");
        let id = "1793959050798-keyui";
        let backup = archive.join(id);
        let addons = root.join("Interface/AddOns");
        fs::create_dir_all(backup.join("KeyUI")).unwrap();
        fs::create_dir_all(addons.join("KeyUI_Extra")).unwrap();
        fs::write(backup.join("KeyUI/old.lua"), "-- earlier version").unwrap();
        crate::preferences::save_json(
            &backup.join(MANIFEST),
            &serde_json::json!({
                "id": id,
                "root": root,
                "flavor": "retail",
                "addonId": "keyui",
                "name": "KeyUI",
                "version": "1",
                "release": "v2",
                "notes": "Updated settings",
                "folders": ["KeyUI", "KeyUI_Extra"],
                "previousFolders": ["KeyUI"],
                "time": "2026-11-06T09:57:31.519Z"
            }),
        )
        .unwrap();
        let listed = list_electron(&archive, &root, Client::Retail, &catalog).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].transaction.previous_version.as_deref(), Some("1"));
        assert!(
            list_electron(&archive, &root, Client::Forever, &catalog)
                .unwrap()
                .is_empty()
        );
        assert!(
            restore_electron_with(&root, Client::Retail, &backup, &catalog, || Err(
                "WoW started".into()
            ))
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(addons.join("KeyUI/old.lua")).unwrap(),
            "-- original"
        );
        restore_electron_with(&root, Client::Retail, &backup, &catalog, || Ok(())).unwrap();
        assert_eq!(
            fs::read_to_string(addons.join("KeyUI/old.lua")).unwrap(),
            "-- earlier version"
        );
        assert!(!addons.join("KeyUI_Extra").exists());
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
        let undo = list(&root, Client::Retail).unwrap().remove(0);
        restore_with(&root, Client::Retail, &undo.path, || Ok(())).unwrap();
        assert_eq!(
            fs::read_to_string(addons.join("KeyUI/old.lua")).unwrap(),
            "-- original"
        );
        assert!(addons.join("KeyUI_Extra").exists());
        assert_eq!(
            fs::read_to_string(backup.join("KeyUI/old.lua")).unwrap(),
            "-- earlier version"
        );
    }

    #[test]
    fn reviewed_github_project_installs_and_is_recognized_on_the_next_scan() {
        let (_temp, root, mut catalog) = game();
        let bytes = || {
            zip(&[
                (
                    "SampleAddon/SampleAddon.toc",
                    "## Interface: 120105\n## Version: 1.2.3\nMain.lua\n",
                ),
                ("SampleAddon/Main.lua", "-- sample"),
            ])
        };
        let inspect = |catalog: &[Definition]| {
            packages::inspect_project(
                bytes(),
                &root,
                Client::Retail,
                "owner/SampleAddon",
                "^SampleAddon.*\\.zip$".into(),
                catalog,
            )
        };
        let (source, package) = inspect(&catalog).unwrap();
        install_with(
            &package,
            &source.name,
            ("v1.2.3", ""),
            || Ok(()),
            |a, b| fs::rename(a, b),
        )
        .unwrap();
        let (_, preview) = inspect(&catalog).unwrap();
        assert!(preview.modules.iter().any(|module| module.replaces));
        assert_eq!(
            fs::read_to_string(root.join("Interface/AddOns/SampleAddon/Main.lua")).unwrap(),
            "-- sample",
            "reviewing a replacement does not install it"
        );
        catalog.push(serde_json::from_str(&serde_json::to_string(&source).unwrap()).unwrap());
        let (registered, preview) = inspect(&catalog).unwrap();
        assert_eq!(registered.id, source.id);
        assert!(preview.modules.iter().any(|module| module.replaces));
        assert!(
            packages::inspect_project(
                bytes(),
                &root,
                Client::Retail,
                "someone-else/SampleAddon",
                "^other.zip$".into(),
                &catalog
            )
            .is_err(),
            "an unrelated repository cannot replace a registered source"
        );
        let installed = library::scan(&root, Client::Retail, &catalog).unwrap();
        let addon = installed
            .iter()
            .find(|addon| addon.folder == "SampleAddon")
            .unwrap();
        assert_eq!(addon.source, "GitHub");
        assert_eq!(addon.version, "1.2.3");
        assert!(!source.supports(Client::Forever));
    }

    #[test]
    fn project_review_updates_a_curated_addon_with_its_existing_identity_and_backup() {
        let (_temp, root, catalog) = game();
        let source = catalog.iter().find(|source| source.id == "keyui").unwrap();
        fs::write(
            root.join("Interface/AddOns/KeyUI/KeyUI.toc"),
            "## Interface: 120105\n## Version: 1\nold.lua\n",
        )
        .unwrap();
        let bytes = zip(&[
            (
                "KeyUI/KeyUI.toc",
                "## Interface: 120105\n## Title: |cff00ff00Key UI|r\n## Version: 2\nnew.lua\n",
            ),
            ("KeyUI/new.lua", "-- reviewed update"),
        ]);
        let (reviewed, package) = packages::inspect_project(
            bytes,
            &root,
            Client::Retail,
            source.repo.as_ref().unwrap(),
            "^different-selection.zip$".into(),
            &catalog,
        )
        .unwrap();
        assert_eq!(reviewed.id, source.id);
        assert_eq!(
            reviewed.asset, source.asset,
            "curated release matching is retained"
        );
        assert_eq!(reviewed.requires, source.requires);
        assert_eq!(package.modules[0].name, "Key UI");
        assert!(package.modules[0].replaces);
        let backup = install_with(
            &package,
            &reviewed.name,
            ("2", "reviewed release"),
            || Ok(()),
            |from, to| fs::rename(from, to),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(backup.join("previous/KeyUI/old.lua")).unwrap(),
            "-- original"
        );
        assert_eq!(
            fs::read_to_string(root.join("Interface/AddOns/KeyUI/new.lua")).unwrap(),
            "-- reviewed update"
        );
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
    }

    #[test]
    fn curated_project_review_rejects_another_catalog_projects_folders() {
        let (_temp, root, catalog) = game();
        let source = catalog.iter().find(|source| source.id == "keyui").unwrap();
        let bytes = zip(&[
            ("KeyUI/KeyUI.toc", "## Interface: 120105\n"),
            ("BigWigs/BigWigs.toc", "## Interface: 120105\n"),
        ]);
        let result = packages::inspect_project(
            bytes,
            &root,
            Client::Retail,
            source.repo.as_ref().unwrap(),
            "^reviewed.zip$".into(),
            &catalog,
        );
        assert!(result.err().unwrap().contains("different source"));
        assert_eq!(
            fs::read_to_string(root.join("Interface/AddOns/KeyUI/old.lua")).unwrap(),
            "-- original"
        );
        assert!(!root.join("Interface/AddOns/BigWigs").exists());
    }

    #[test]
    fn new_project_review_keeps_external_dependencies_and_clean_display_names() {
        let (_temp, root, catalog) = game();
        let bytes = zip(&[
            (
                "SampleAddon/SampleAddon.toc",
                "## Interface: 120105\n## Title: |cff00ff00Sample Tools|r\n## Dependencies: ExternalDep, Helper\n## Version: 1\n",
            ),
            (
                "Helper/Helper.toc",
                "## Interface: 120105\n## Title: Internal helper\n",
            ),
        ]);
        let (source, package) = packages::inspect_project(
            bytes,
            &root,
            Client::Retail,
            "owner/SampleAddon",
            "^SampleAddon.zip$".into(),
            &catalog,
        )
        .unwrap();
        assert_eq!(source.name, "Sample Tools");
        assert_eq!(source.requires, vec!["ExternalDep"]);
        assert_eq!(
            package
                .missing
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["ExternalDep"]
        );
        assert!(!root.join("Interface/AddOns/SampleAddon").exists());
        assert!(!root.join("Interface/AddOns/Helper").exists());
    }

    #[test]
    fn installs_bundled_talents_through_the_package_review() {
        let (_temp, root, catalog) = game();
        let source = catalog
            .iter()
            .find(|source| source.id == "talents")
            .unwrap();
        let bytes = crate::bundled::package(&source.folder).unwrap();
        let package = packages::inspect(bytes, &root, Client::Retail, source, &catalog).unwrap();
        let version = crate::bundled::version(&source.folder).unwrap();
        assert_eq!(
            package.modules[0].version.as_deref(),
            Some(version.as_str())
        );
        install_with(
            &package,
            &source.name,
            (&version, ""),
            || Ok(()),
            |from, to| fs::rename(from, to),
        )
        .unwrap();
        let path = root.join("Interface/AddOns/AstralTalents");
        assert!(path.join("Builds.lua").is_file());
        assert!(path.join("Talents.lua").is_file());
    }

    #[test]
    fn uninstall_moves_the_full_addon_to_a_restorable_backup_and_preserves_settings() {
        let (_temp, root, catalog) = game();
        let addons = root.join("Interface/AddOns");
        fs::write(addons.join("KeyUI/KeyUI.toc"), "## Version: 1\n").unwrap();
        fs::create_dir(addons.join("KeyUI_Extra")).unwrap();
        fs::write(addons.join("KeyUI_Extra/KeyUI_Extra.toc"), "").unwrap();
        let addon = library::scan(&root, Client::Retail, &catalog)
            .unwrap()
            .remove(0);
        assert_eq!(addon.modules, ["KeyUI", "KeyUI_Extra"]);

        let dependent = addons.join("!Example");
        fs::create_dir(&dependent).unwrap();
        fs::write(
            dependent.join("!Example.toc"),
            "## RequiredDeps: keyui_extra\n",
        )
        .unwrap();
        let error = remove_with(&root, Client::Retail, &addon, &catalog, || Ok(())).unwrap_err();
        assert!(error.contains("Required by !Example"));
        assert!(addons.join("KeyUI/old.lua").is_file());
        fs::remove_dir_all(dependent).unwrap();

        let backup = remove_with(&root, Client::Retail, &addon, &catalog, || Ok(())).unwrap();
        assert!(read_transaction(&backup.join(MANIFEST)).unwrap().removed);
        assert!(!addons.join("KeyUI").exists());
        assert!(!addons.join("KeyUI_Extra").exists());
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
        restore_with(&root, Client::Retail, &backup, || Ok(())).unwrap();
        assert!(addons.join("KeyUI/old.lua").is_file());
        assert!(addons.join("KeyUI_Extra/KeyUI_Extra.toc").is_file());
    }

    #[test]
    fn talent_refresh_leaves_builds_in_place_if_wow_opens_while_fetching() {
        let (_temp, root, _) = game();
        let folder = root.join("Interface/AddOns/AstralTalents");
        fs::create_dir(&folder).unwrap();
        let file = folder.join("Builds.lua");
        fs::write(&file, "previous data").unwrap();
        let running = Cell::new(false);
        let result = refresh_talents_with(
            &root,
            |interface| {
                assert_eq!(interface, 120105);
                running.set(true);
                Ok("new data".into())
            },
            || {
                if running.get() {
                    Err("WoW is open".into())
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(result, Err("WoW is open".into()));
        assert_eq!(fs::read_to_string(&file).unwrap(), "previous data");
        assert!(refresh_talents_with(&root, |_| Ok("new data".into()), || Ok(())).unwrap());
        assert_eq!(fs::read_to_string(file).unwrap(), "new data");
    }

    #[test]
    fn talent_refresh_counts_build_changes_but_still_saves_the_checked_date() {
        let (_temp, root, _) = game();
        let folder = root.join("Interface/AddOns/AstralTalents");
        fs::create_dir(&folder).unwrap();
        let file = folder.join("Builds.lua");
        let original = "local _, ns = ...\nns.checked = \"2026-11-16\"\nns.builds = { 1 }\n";
        fs::write(&file, original).unwrap();
        assert!(!refresh_talents_with(&root, |_| Ok(original.into()), || Ok(())).unwrap());
        let date_only = original.replace("2026-11-16", "2026-11-17");
        assert!(!refresh_talents_with(&root, |_| Ok(date_only.clone()), || Ok(())).unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), date_only);
        let changed = date_only.replace("{ 1 }", "{ 2 }");
        assert!(refresh_talents_with(&root, |_| Ok(changed.clone()), || Ok(())).unwrap());
        assert_eq!(fs::read_to_string(file).unwrap(), changed);
    }

    #[test]
    fn refuses_a_review_if_another_updater_changed_the_installed_version() {
        let (_temp, root, catalog) = game();
        let source = catalog.iter().find(|source| source.id == "keyui").unwrap();
        let installed = root.join("Interface/AddOns/KeyUI/KeyUI.toc");
        fs::write(&installed, "## Version: 1\n").unwrap();
        let package =
            packages::inspect(keyui_release(), &root, Client::Retail, source, &catalog).unwrap();
        assert_eq!(package.installed_version.as_deref(), Some("1"));
        fs::write(&installed, "## Version: 3\n").unwrap();
        let error = install_with(
            &package,
            &source.name,
            ("2", ""),
            || Ok(()),
            |from, to| fs::rename(from, to),
        )
        .unwrap_err();
        assert!(error.contains("version changed"));
        assert_eq!(fs::read_to_string(installed).unwrap(), "## Version: 3\n");
    }

    #[test]
    fn open_backups_works_before_the_first_update() {
        let (_temp, root, _) = game();
        let path = open_backup_folder(&root, Client::Retail).unwrap();
        assert_eq!(
            path,
            root.canonicalize()
                .unwrap()
                .join("Interface")
                .join(BACKUP_DIR)
        );
        assert!(path.is_dir());
        assert!(list(&root, Client::Retail).unwrap().is_empty());
        assert_eq!(open_backup_folder(&root, Client::Retail).unwrap(), path);
    }

    #[test]
    fn ending_an_install_unlocks_even_if_a_duplicated_handle_is_still_open() {
        let (_temp, root, _) = game();
        let paths = Paths::open(&root, Client::Retail).unwrap();
        let lock = paths.lock().unwrap();
        let duplicate = lock.0.try_clone().unwrap();
        drop(lock);
        let next = paths.lock();
        drop(duplicate);
        assert!(next.is_ok());
    }

    #[test]
    fn install_and_restore_preserve_settings_and_keep_both_versions() {
        let (_temp, root, catalog) = game();
        let source = catalog.iter().find(|source| source.id == "keyui").unwrap();
        fs::write(
            root.join("Interface/AddOns/KeyUI/KeyUI.toc"),
            "## Version: 1\n",
        )
        .unwrap();
        let package =
            packages::inspect(keyui_release(), &root, Client::Retail, source, &catalog).unwrap();
        assert_eq!(package.modules.len(), 2);
        assert!(package.missing.is_empty());
        let backup = install_with(
            &package,
            "KeyUI",
            ("v2", "Changelog"),
            || Ok(()),
            |a, b| fs::rename(a, b),
        )
        .unwrap();
        assert!(backup.join(MANIFEST).is_file());
        assert_eq!(
            fs::read_to_string(backup.join("previous/KeyUI/old.lua")).unwrap(),
            "-- original"
        );
        assert_eq!(
            fs::read_to_string(root.join("Interface/AddOns/KeyUI/new.lua")).unwrap(),
            "-- new release"
        );
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
        assert!(
            root.join("Interface/AddOns/KeyUI_Extra/KeyUI_Extra.toc")
                .is_file()
        );
        let records = list(&root, Client::Retail).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path, backup);
        assert_eq!(records[0].transaction.previous, ["KeyUI"]);
        assert_eq!(
            records[0].transaction.previous_version.as_deref(),
            Some("1")
        );
        // Simulate an addon update after the backup was selected in the app.
        let live = root.join("Interface/AddOns/KeyUI");
        fs::write(live.join("KeyUI.toc"), "## Version: 3\n").unwrap();
        fs::write(live.join("new.lua"), "-- installed after review").unwrap();
        restore_with(&root, Client::Retail, &records[0].path, || Ok(())).unwrap();
        assert!(root.join("Interface/AddOns/KeyUI/old.lua").is_file());
        assert!(!root.join("Interface/AddOns/KeyUI_Extra").exists());
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
        let restored = list(&root, Client::Retail)
            .unwrap()
            .into_iter()
            .find(|record| record.transaction.restored)
            .unwrap();
        assert_eq!(
            fs::read_to_string(restored.path.join("previous/KeyUI/new.lua")).unwrap(),
            "-- installed after review"
        );
        assert!(
            restored
                .path
                .join("previous/KeyUI_Extra/KeyUI_Extra.toc")
                .is_file()
        );
        restore_with(&root, Client::Retail, &restored.path, || Ok(())).unwrap();
        assert_eq!(
            fs::read_to_string(live.join("KeyUI.toc")).unwrap(),
            "## Version: 3\n"
        );
        assert_eq!(
            fs::read_to_string(live.join("new.lua")).unwrap(),
            "-- installed after review"
        );
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
    }

    #[test]
    fn restoring_first_install_removes_addon_files_and_keeps_settings() {
        let (_temp, root, catalog) = game();
        let source = catalog.iter().find(|source| source.id == "keyui").unwrap();
        let addons = root.join("Interface/AddOns");
        fs::remove_dir_all(addons.join("KeyUI")).unwrap();
        let package =
            packages::inspect(keyui_release(), &root, Client::Retail, source, &catalog).unwrap();
        let backup = install_with(
            &package,
            "KeyUI",
            ("2", ""),
            || Ok(()),
            |from, to| fs::rename(from, to),
        )
        .unwrap();
        restore_with(&root, Client::Retail, &backup, || Ok(())).unwrap();
        assert!(!addons.join("KeyUI").exists());
        assert!(!addons.join("KeyUI_Extra").exists());
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
        let records = list(&root, Client::Retail).unwrap();
        let restored = records
            .iter()
            .find(|entry| entry.transaction.restored)
            .unwrap();
        assert!(restored.path.join("previous/KeyUI/new.lua").is_file());
    }

    #[test]
    fn recovers_an_interrupted_operation_without_losing_either_copy() {
        let (_temp, root, _) = game();
        let paths = Paths::open(&root, Client::Retail).unwrap();
        let backup = paths.backups.join("release-interrupted");
        let prior = backup.join("previous");
        fs::create_dir_all(&prior).unwrap();
        fs::rename(paths.addons.join("KeyUI"), prior.join("KeyUI")).unwrap();
        fs::create_dir(paths.addons.join("KeyUI")).unwrap();
        fs::write(paths.addons.join("KeyUI/new.lua"), "-- new").unwrap();
        fs::create_dir(paths.addons.join("KeyUI_Extra")).unwrap();
        let record = Transaction {
            addon: "KeyUI".into(),
            release: "v2".into(),
            previous_version: None,
            notes: String::new(),
            created: 1,
            created_ms: None,
            folders: vec!["KeyUI".into(), "KeyUI_Extra".into()],
            previous: vec!["KeyUI".into()],
            restored: false,
            removed: false,
            profile_switch: None,
        };
        fs::write(backup.join(PENDING), serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(list(&root, Client::Retail).unwrap()[0].interrupted);
        recover_with(
            &root,
            Client::Retail,
            &backup,
            || Ok(()),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(paths.addons.join("KeyUI/old.lua").is_file());
        assert!(!paths.addons.join("KeyUI_Extra").exists());
        assert!(backup.join("interrupted-files/KeyUI/new.lua").is_file());
        assert!(backup.join("interrupted-files/KeyUI_Extra").is_dir());
        assert!(!backup.join(PENDING).exists());
        assert!(list(&root, Client::Retail).unwrap()[0].recovered);
    }

    #[test]
    fn failed_module_swap_rolls_back_the_whole_release() {
        let (_temp, root, catalog) = game();
        let source = catalog.iter().find(|source| source.id == "keyui").unwrap();
        let package =
            packages::inspect(keyui_release(), &root, Client::Retail, source, &catalog).unwrap();
        let moves = Cell::new(0);
        let error = install_with(
            &package,
            "KeyUI",
            ("v2", ""),
            || Ok(()),
            |a, b| {
                moves.set(moves.get() + 1);
                if moves.get() == 2 {
                    return Err(io::Error::other("simulated install failure"));
                }
                fs::rename(a, b)
            },
        )
        .unwrap_err();
        assert!(error.contains("Previous files were restored"));
        assert!(root.join("Interface/AddOns/KeyUI/old.lua").is_file());
        assert!(!root.join("Interface/AddOns/KeyUI/new.lua").exists());
        assert!(!root.join("Interface/AddOns/KeyUI_Extra").exists());
    }

    #[test]
    fn inspection_blocks_unsafe_paths_and_reports_missing_dependencies() {
        let (_temp, root, catalog) = game();
        let source = catalog.iter().find(|source| source.id == "keyui").unwrap();
        for (entries, expected) in [
            (vec![("KeyUI/../escaped.lua", "")], "unsafe"),
            (
                vec![("KeyUI/KeyUI.toc", "## Interface: 16001")],
                "doesn't list",
            ),
            (
                vec![("KeyUI/KeyUI.toc", "## Interface: 120105\nmissing.lua")],
                "missing a file",
            ),
            (
                vec![("KeyUI/KeyUI.toc", "## Interface: 120105\n../../escaped.lua")],
                "unsafe file",
            ),
        ] {
            let err = packages::inspect(zip(&entries), &root, Client::Retail, source, &catalog)
                .err()
                .unwrap();
            assert!(err.contains(expected), "{err}");
        }
        let bytes = zip(&[(
            "KeyUI/KeyUI.toc",
            "## Interface: 120105\n## Dependencies: LibStub",
        )]);
        let package = packages::inspect(bytes, &root, Client::Retail, source, &catalog).unwrap();
        assert!(package.missing.contains("LibStub"));
        assert!(
            install_with(
                &package,
                "KeyUI",
                ("v2", ""),
                || Ok(()),
                |a, b| fs::rename(a, b)
            )
            .is_err()
        );
        assert!(root.join("Interface/AddOns/KeyUI/old.lua").is_file());
    }

    #[test]
    fn rejects_unbounded_zip64_directory_before_parsing_it() {
        let (_temp, root, catalog) = game();
        let source = catalog.iter().find(|source| source.id == "keyui").unwrap();
        let mut bytes = keyui_release();
        let end = bytes.len() - 22;
        let mut locator = [0xff; 20];
        locator[..4].copy_from_slice(b"PK\x06\x07");
        bytes.splice(end..end, locator);
        let error = packages::inspect(bytes, &root, Client::Retail, source, &catalog)
            .err()
            .unwrap();
        assert!(error.contains("ZIP64"), "{error}");
    }

    #[test]
    fn game_start_during_staging_leaves_live_folders_untouched() {
        let (_temp, root, catalog) = game();
        let source = catalog.iter().find(|source| source.id == "keyui").unwrap();
        let package =
            packages::inspect(keyui_release(), &root, Client::Retail, source, &catalog).unwrap();
        let attempts = Cell::new(0);
        let error = install_with(
            &package,
            "KeyUI",
            ("v2", ""),
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    Ok(())
                } else {
                    Err("Close WoW".into())
                }
            },
            |_, _| panic!("must not modify live folders"),
        )
        .unwrap_err();
        assert_eq!(error, "Close WoW");
        assert!(root.join("Interface/AddOns/KeyUI/old.lua").is_file());
    }
}
