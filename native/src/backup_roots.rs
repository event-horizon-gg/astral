//! Local discovery metadata; backup contents and restore ownership stay in the installer.
use crate::{library::Client, preferences};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ROOTS: usize = 4096;

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
struct Root {
    client: Client,
    root: PathBuf,
}

fn read(path: &Path) -> io::Result<Vec<Root>> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid backup discovery metadata.",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Backup discovery metadata is too large.",
        ));
    }
    let roots: Vec<Root> = serde_json::from_slice(&bytes)?;
    if roots.len() > MAX_ROOTS || roots.iter().any(|root| !root.root.is_absolute()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid backup installation paths.",
        ));
    }
    Ok(roots)
}

fn merge(roots: impl IntoIterator<Item = Root>) -> io::Result<Vec<Root>> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for root in roots {
        if !root.root.is_absolute() {
            continue;
        }
        // Registry maintenance must not probe offline game volumes while
        // holding its writer lock. Discovery deduplicates actual backup paths.
        if seen.insert((root.client.index(), root.root.clone())) {
            result.push(root);
        }
    }
    if result.len() > MAX_ROOTS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Too many recorded backup installations.",
        ));
    }
    Ok(result)
}

pub(crate) fn remember(preferences: &Path, current: &[(Client, PathBuf)]) -> io::Result<()> {
    let folder = preferences.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(folder)?;
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(folder.join("backup-roots.lock"))?;
    lock.lock()?;
    let path = folder.join("backup-roots.json");
    let previous = read(&path)?;
    // Preserve historical roots before the activity ring is truncated, including
    // installations removed from current preferences. Missing offline roots stay.
    let history = read(&folder.join("addon-activity.json")).unwrap_or_default();
    let roots = merge(
        previous
            .iter()
            .cloned()
            .chain(history)
            .chain(current.iter().map(|(client, root)| Root {
                client: *client,
                root: root.clone(),
            })),
    )?;
    if roots != previous {
        preferences::save_json(&path, &roots)?;
    }
    Ok(())
}

pub(crate) fn discover(
    preferences: Option<&Path>,
    current: &[(Client, PathBuf)],
) -> (Vec<(Client, PathBuf)>, Option<String>) {
    let mut error = None;
    let mut roots = Vec::new();
    if let Some(folder) = preferences.and_then(Path::parent) {
        match read(&folder.join("backup-roots.json")) {
            Ok(found) => roots.extend(found),
            Err(problem) => error = Some(format!("Couldn't read backup installations: {problem}")),
        }
        if let Ok(history) = read(&folder.join("addon-activity.json")) {
            roots.extend(history);
        }
    }
    roots.extend(current.iter().map(|(client, root)| Root {
        client: *client,
        root: root.clone(),
    }));
    // Current valid selections remain usable even if the registry is damaged.
    let roots = match merge(roots) {
        Ok(roots) => roots,
        Err(problem) => {
            error = Some(problem.to_string());
            current
                .iter()
                .filter(|(_, root)| root.is_absolute())
                .map(|(client, root)| Root {
                    client: *client,
                    root: root.clone(),
                })
                .collect()
        }
    };
    (
        roots
            .into_iter()
            .map(|root| (root.client, root.root))
            .collect(),
        error,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historic_roots_survive_preferences_changes_and_activity_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let prefs = temp.path().join("preferences.json");
        let old = temp.path().join("old/_retail_");
        let current = temp.path().join("current/_retail_");
        preferences::save_json(
            &temp.path().join("addon-activity.json"),
            &serde_json::json!([
                { "root": old, "client": "retail", "name": "Earlier addon", "time": 1 }
            ]),
        )
        .unwrap();
        remember(&prefs, &[(Client::Retail, current.clone())]).unwrap();
        preferences::save_json(
            &temp.path().join("addon-activity.json"),
            &Vec::<Root>::new(),
        )
        .unwrap();
        let (roots, error) = discover(Some(&prefs), &[]);
        assert!(error.is_none());
        assert!(roots.contains(&(Client::Retail, old)));
        assert!(roots.contains(&(Client::Retail, current)));
    }

    #[test]
    fn parallel_writers_merge_roots_and_damaged_registry_is_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let prefs = temp.path().join("preferences.json");
        let tasks: Vec<_> = (0..8)
            .map(|index| {
                let prefs = prefs.clone();
                let root = temp.path().join(format!("game-{index}/_retail_"));
                std::thread::spawn(move || remember(&prefs, &[(Client::Retail, root)]).unwrap())
            })
            .collect();
        for task in tasks {
            task.join().unwrap();
        }
        assert_eq!(discover(Some(&prefs), &[]).0.len(), 8);
        let path = temp.path().join("backup-roots.json");
        fs::write(&path, "broken").unwrap();
        let current = vec![(Client::Forever, temp.path().join("_forever_"))];
        let (roots, error) = discover(Some(&prefs), &current);
        assert!(roots == current);
        assert!(error.is_some());
        assert!(remember(&prefs, &current).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "broken");
    }
}
