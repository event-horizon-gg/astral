//! A browseable collection of links. It never copies, moves or recursively deletes backups.
use crate::preferences;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};
#[cfg(target_os = "windows")]
#[path = "backup_links_windows.rs"]
mod windows;

#[derive(Clone)]
pub(crate) struct Folder {
    pub label: String,
    pub path: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
struct Link {
    name: String,
    target: PathBuf,
}
const PREFIX: &str = "astral-backup-";
const MAX_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn update(
    preferences: PathBuf,
    folders: Vec<Folder>,
    complete: bool,
) -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows::run(move || update_inner(&preferences, &folders, complete))
    }
    #[cfg(not(target_os = "windows"))]
    {
        update_inner(&preferences, &folders, complete)
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn shell_path(path: &Path) -> PathBuf {
    windows::normalize(path)
}

pub(crate) fn existing(preferences: &Path) -> io::Result<PathBuf> {
    let folder = preferences
        .parent()
        .ok_or(io::ErrorKind::InvalidInput)?
        .join("backups");
    if !plain_directory(&folder.symlink_metadata()?) {
        return Err(io::Error::other(
            "The backup collection must be a regular folder.",
        ));
    }
    folder.canonicalize()
}

fn plain_directory(metadata: &fs::Metadata) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.is_dir()
            && metadata.file_attributes()
                & ::windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT.0
                == 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        metadata.is_dir() && !metadata.file_type().is_symlink()
    }
}

fn valid_name(name: &str) -> bool {
    name.starts_with(PREFIX)
        && name.len() < 180
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn same_target(left: &Path, right: &Path) -> bool {
    if let (Ok(left), Ok(right)) = (left.canonicalize(), right.canonicalize()) {
        return left == right;
    }
    #[cfg(target_os = "windows")]
    {
        windows::normalize(left) == windows::normalize(right)
    }
    #[cfg(not(target_os = "windows"))]
    {
        left == right
    }
}

fn read_link(path: &Path) -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if !path.symlink_metadata()?.is_file() {
            return Err(io::Error::other("Not a backup shortcut."));
        }
        windows::read(path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        fs::read_link(path)
    }
}

fn create_link(target: &Path, path: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows::create(target, path)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, path)
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = (target, path);
        Err(io::Error::other(
            "Backup collections aren't supported on this system.",
        ))
    }
}

fn update_inner(preferences: &Path, folders: &[Folder], complete: bool) -> io::Result<PathBuf> {
    let data = preferences.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(data)?;
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(data.join("backup-collection.lock"))?;
    lock.lock()?;
    let folder = data.join("backups");
    match folder.symlink_metadata() {
        Ok(metadata) if !plain_directory(&metadata) => {
            return Err(io::Error::other(
                "The backup collection must be a regular folder.",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&folder)?,
        Err(error) => return Err(error),
    }
    let folder = folder.canonicalize()?;
    let index = data.join("backup-collection.json");
    let old: Vec<Link> = match index.symlink_metadata() {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_BYTES as u64 => {
            let bytes = fs::read(&index)?;
            if bytes.len() > MAX_BYTES {
                return Err(io::Error::other("Backup collection metadata is too large."));
            }
            serde_json::from_slice(&bytes)?
        }
        Ok(_) => return Err(io::Error::other("Invalid backup collection metadata.")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    if old
        .iter()
        .any(|entry| !valid_name(&entry.name) || !entry.target.is_absolute())
    {
        return Err(io::Error::other("Invalid backup collection entries."));
    }
    let mut desired = Vec::new();
    let mut targets = HashSet::new();
    for entry in folders {
        let target = entry.path.canonicalize()?;
        if !target.is_dir() {
            return Err(io::Error::other("A backup folder is unavailable."));
        }
        if folder.starts_with(&target) {
            return Err(io::Error::other("A backup cannot contain its collection."));
        }
        // Real backups already directly inside this directory stay unmanaged.
        if target.parent() == Some(folder.as_path()) || !targets.insert(target.clone()) {
            continue;
        }
        let label: String = entry
            .label
            .chars()
            .take(70)
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '-'
                }
            })
            .collect();
        let digest = format!(
            "{:x}",
            Sha256::digest(target.as_os_str().as_encoded_bytes())
        );
        let extension = if cfg!(target_os = "windows") {
            ".lnk"
        } else {
            ""
        };
        let name = format!("{PREFIX}{label}-{}{extension}", &digest[..24]);
        desired.push(Link { name, target });
    }
    if !complete {
        for previous in &old {
            if !desired.iter().any(|entry| entry.name == previous.name) {
                desired.push(previous.clone());
            }
        }
    }
    if serde_json::to_vec(&desired)?.len() > MAX_BYTES {
        return Err(io::Error::other("Backup collection metadata is too large."));
    }
    for entry in &desired {
        let path = folder.join(&entry.name);
        match path.symlink_metadata() {
            Ok(_) => {
                if !same_target(&read_link(&path)?, &entry.target) {
                    return Err(io::Error::other(
                        "A backup shortcut was changed outside Astral.",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Missing targets in an incomplete scan retain their index only.
                if entry.target.is_dir() {
                    create_link(&entry.target, &path)?;
                }
            }
            Err(error) => return Err(error),
        }
    }
    if complete {
        for entry in &old {
            if desired.iter().any(|desired| desired.name == entry.name) {
                continue;
            }
            let path = folder.join(&entry.name);
            match read_link(&path) {
                Ok(target) if same_target(&target, &entry.target) => fs::remove_file(path)?,
                Ok(_) => desired.push(entry.clone()), // Preserve externally changed shortcuts.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => desired.push(entry.clone()), // Never remove an unrecognized file/directory.
            }
        }
    }
    if serde_json::to_vec(&desired)?.len() > MAX_BYTES {
        return Err(io::Error::other("Backup collection metadata is too large."));
    }
    preferences::save_json(&index, &desired)?;
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_links_multiple_stores_and_prunes_only_owned_links() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let prefs = data.join("preferences.json");
        let first = temp.path().join("game/backup-one");
        let second = temp.path().join("archive/backup-two");
        for path in [&first, &second] {
            fs::create_dir_all(path).unwrap();
            fs::write(path.join("preserved.lua"), "original").unwrap();
        }
        let folders = vec![
            Folder {
                label: "First addon".into(),
                path: first.clone(),
            },
            Folder {
                label: "Second addon".into(),
                path: second.clone(),
            },
        ];
        let collection = update(prefs.clone(), folders.clone(), true).unwrap();
        fs::write(collection.join("my-notes.txt"), "keep").unwrap();
        // Opening either link reaches the actual backup without copying it.
        #[cfg(unix)]
        for entry in fs::read_dir(&collection).unwrap() {
            let path = entry.unwrap().path();
            if path.symlink_metadata().unwrap().file_type().is_symlink() {
                assert_eq!(
                    fs::read_to_string(path.join("preserved.lua")).unwrap(),
                    "original"
                );
            }
        }
        let index: Vec<Link> =
            serde_json::from_slice(&fs::read(data.join("backup-collection.json")).unwrap())
                .unwrap();
        assert_eq!(index.len(), 2);
        #[cfg(target_os = "windows")]
        windows::run({
            let collection = collection.clone();
            let index = index.clone();
            move || {
                for entry in index {
                    assert!(same_target(
                        &read_link(&collection.join(entry.name))?,
                        &entry.target
                    ));
                }
                Ok(())
            }
        })
        .unwrap();
        update(prefs.clone(), vec![folders[0].clone()], false).unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<Link>>(
                &fs::read(data.join("backup-collection.json")).unwrap()
            )
            .unwrap()
            .len(),
            2
        );
        update(prefs, vec![folders[0].clone()], true).unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<Link>>(
                &fs::read(data.join("backup-collection.json")).unwrap()
            )
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            fs::read_to_string(second.join("preserved.lua")).unwrap(),
            "original"
        );
        assert_eq!(
            fs::read_to_string(collection.join("my-notes.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn corrupt_metadata_and_unmanaged_name_collisions_do_not_remove_files() {
        let temp = tempfile::tempdir().unwrap();
        let prefs = temp.path().join("preferences.json");
        let target = temp.path().join("source");
        fs::create_dir_all(&target).unwrap();
        let folders = vec![Folder {
            label: "Fixture".into(),
            path: target.clone(),
        }];
        let collection = update(prefs.clone(), folders.clone(), true).unwrap();
        let index: Vec<Link> =
            serde_json::from_slice(&fs::read(temp.path().join("backup-collection.json")).unwrap())
                .unwrap();
        let alias = collection.join(&index[0].name);
        fs::remove_file(&alias).unwrap();
        fs::create_dir(&alias).unwrap();
        fs::write(alias.join("user.txt"), "keep").unwrap();
        assert!(update(prefs.clone(), folders, true).is_err());
        assert_eq!(fs::read_to_string(alias.join("user.txt")).unwrap(), "keep");
        fs::write(temp.path().join("backup-collection.json"), "broken").unwrap();
        assert!(update(prefs, vec![], true).is_err());
        assert_eq!(fs::read_to_string(alias.join("user.txt")).unwrap(), "keep");
        assert!(target.is_dir());
    }

    #[test]
    fn real_backup_directories_and_externally_retargeted_links_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let prefs = temp.path().join("preferences.json");
        let direct = temp.path().join("backups/real-backup");
        fs::create_dir_all(&direct).unwrap();
        fs::write(direct.join("original.lua"), "original").unwrap();
        let folder = update(
            prefs.clone(),
            vec![Folder {
                label: "Real backup".into(),
                path: direct.clone(),
            }],
            true,
        )
        .unwrap();
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for target in [&first, &second] {
            fs::create_dir_all(target).unwrap();
            fs::write(target.join("keep.lua"), "keep").unwrap();
        }
        update(
            prefs.clone(),
            vec![Folder {
                label: "Fixture".into(),
                path: first.clone(),
            }],
            true,
        )
        .unwrap();
        let index: Vec<Link> =
            serde_json::from_slice(&fs::read(temp.path().join("backup-collection.json")).unwrap())
                .unwrap();
        let alias = folder.join(&index[0].name);
        fs::remove_file(&alias).unwrap();
        #[cfg(target_os = "windows")]
        windows::run({
            let second = second.clone();
            let alias = alias.clone();
            move || create_link(&second, &alias)
        })
        .unwrap();
        #[cfg(not(target_os = "windows"))]
        create_link(&second, &alias).unwrap();
        update(prefs, vec![], true).unwrap();
        assert!(alias.symlink_metadata().is_ok());
        assert_eq!(
            fs::read_to_string(direct.join("original.lua")).unwrap(),
            "original"
        );
        for target in [first, second] {
            assert_eq!(fs::read_to_string(target.join("keep.lua")).unwrap(), "keep");
        }
    }
}
