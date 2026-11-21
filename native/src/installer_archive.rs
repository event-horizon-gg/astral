//! Durable addon restore points. Local journals still protect same-volume swaps.
use super::*;

const RECORD: &str = "native-manifest.json";

#[derive(Serialize, Deserialize)]
struct Saved {
    root: PathBuf,
    client: Client,
    journal: PathBuf,
    transaction: Transaction,
}

pub(super) fn directory(preferences: &Path) -> Result<PathBuf, String> {
    let data = preferences
        .parent()
        .ok_or("Couldn't find the Astral settings folder.")?;
    fs::create_dir_all(data).map_err(|error| error.to_string())?;
    let folder = data.join("backups");
    match fs::create_dir(&folder) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    if !plain_folder(&folder)? {
        return Err("The addon backup store must be a regular folder.".into());
    }
    folder.canonicalize().map_err(|error| error.to_string())
}

fn plain_folder(path: &Path) -> Result<bool, String> {
    match path.symlink_metadata() {
        Ok(metadata) => Ok(metadata.is_dir() && !crate::settings_backup::link(&metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn read(path: &Path) -> Result<Saved, String> {
    if !plain_folder(path)? {
        return Err("The addon backup is missing or linked.".into());
    }
    let valid_id = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("native-"))
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok());
    if !valid_id {
        return Err("Invalid addon backup identifier.".into());
    }
    let manifest = path.join(RECORD);
    if crate::settings_backup::link(
        &manifest
            .symlink_metadata()
            .map_err(|error| error.to_string())?,
    ) {
        return Err("The addon backup manifest is linked.".into());
    }
    let saved: Saved = read_manifest(&manifest)?;
    validate_transaction(&saved.transaction)?;
    if !saved.root.is_absolute()
        || !saved.journal.is_absolute()
        || saved.journal.parent() != Some(saved.root.join("Interface").join(BACKUP_DIR).as_path())
    {
        return Err("Invalid addon backup ownership.".into());
    }
    Ok(saved)
}

pub(super) fn snapshot(
    paths: &Paths,
    journal: &Path,
    transaction: &Transaction,
) -> Result<Option<PathBuf>, String> {
    snapshot_from(paths, journal, transaction, &paths.addons)
}

pub(super) fn snapshot_from(
    paths: &Paths,
    journal: &Path,
    transaction: &Transaction,
    source: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(store) = &paths.archive else {
        return Ok(None);
    };
    if !plain_folder(store)? {
        return Err("The addon backup store is missing or linked.".into());
    }
    let stage = tempfile::Builder::new()
        .prefix(".astral-backup-")
        .tempdir_in(store)
        .map_err(|error| error.to_string())?;
    let prior = stage.path().join("previous");
    fs::create_dir(&prior).map_err(|error| error.to_string())?;
    for folder in &transaction.previous {
        copy_tree(&source.join(folder), &prior.join(folder)).map_err(|error| error.to_string())?;
    }
    let root = paths
        .interface
        .parent()
        .ok_or("The WoW folder has moved.")?;
    let record = serde_json::json!({
        "root": root, "client": paths.client, "journal": journal,
        "transaction": transaction,
    });
    let bytes = serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?;
    let mut file =
        fs::File::create_new(stage.path().join(RECORD)).map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    let destination = store.join(format!("native-{}", uuid::Uuid::new_v4()));
    fs::rename(stage.path(), &destination).map_err(|error| error.to_string())?;
    // TempDir's old staging path no longer exists; it never owns the published copy.
    Ok(Some(destination))
}

pub(super) fn list(
    store: &Path,
    scope: Option<(&Path, Client)>,
) -> Result<Vec<(Backup, PathBuf)>, String> {
    if !plain_folder(store)? {
        return Ok(Vec::new());
    }
    let store = store.canonicalize().map_err(|error| error.to_string())?;
    let root = scope.map(|(root, client)| {
        (
            root.canonicalize().unwrap_or_else(|_| root.to_owned()),
            client,
        )
    });
    let mut backups = Vec::new();
    for entry in fs::read_dir(store).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_prefix("native-")) else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err() {
            continue;
        }
        let saved = read(&entry.path())?;
        if root
            .as_ref()
            .is_some_and(|(root, client)| &saved.root != root || saved.client != *client)
        {
            continue;
        }
        backups.push((
            Backup {
                path: entry.path(),
                transaction: saved.transaction,
                interrupted: false,
                recovered: false,
                electron: false,
            },
            saved.journal,
        ));
    }
    Ok(backups)
}

pub(super) fn restore_record(paths: &Paths, backup: &Path) -> Result<Transaction, String> {
    let store = paths
        .archive
        .as_ref()
        .ok_or("Couldn't find the Astral addon backup store.")?;
    if backup.parent() != Some(store.as_path()) {
        return Err("This backup belongs to a different backup store.".into());
    }
    let saved = read(backup)?;
    if Some(saved.root.as_path()) != paths.interface.parent() || saved.client != paths.client {
        return Err("This backup belongs to a different WoW folder.".into());
    }
    Ok(saved.transaction)
}
