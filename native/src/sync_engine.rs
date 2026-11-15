use crate::{
    Client, Definition,
    cloud::{Service, Session},
    preferences,
    settings_backup::{self, Profile, Store},
    shared_library,
    sync_format::{Bundle, invalid},
};
use serde::{Deserialize, Serialize};
use std::{error::Error, io, path::Path};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone)]
pub struct Conflict {
    profile: Profile,
    store: std::path::PathBuf,
    revision: u64,
    local: String,
    remote: Option<String>,
    session: String,
}

impl Conflict {
    pub(crate) fn fixture() -> Self {
        Self {
            profile: Profile::Regular,
            store: Default::default(),
            revision: 0,
            local: String::new(),
            remote: None,
            session: String::new(),
        }
    }

    pub(crate) fn issue_key(&self) -> String {
        serde_json::json!([
            self.profile.cloud_id(),
            self.store,
            self.revision,
            self.local,
            self.remote,
            self.session
        ])
        .to_string()
    }

    fn matches(&self, current: &Self) -> bool {
        self.profile == current.profile
            && self.store == current.store
            && self.revision == current.revision
            && self.local == current.local
            && self.remote == current.remote
            && self.session == current.session
    }
}

#[derive(Clone, Copy)]
pub enum Choice {
    Local,
    Cloud,
}

pub enum Outcome {
    Synced {
        message: String,
        installed: usize,
    },
    LibraryOnly {
        collection: shared_library::CollectionResult,
        has_accounts: bool,
    },
    Incomplete(shared_library::CollectionResult),
    Conflict(Conflict),
    ProfilePending,
}

#[derive(Clone)]
pub struct RestorePoint {
    pub profile: Profile,
    pub revision: u64,
    pub id: String,
}

fn setup_matches_after_install(
    before: &Bundle,
    after: &Bundle,
    installed: usize,
) -> io::Result<bool> {
    if installed == 0 {
        return Ok(before.normalized_hash()? == after.normalized_hash()?);
    }
    if before.input != after.input
        || serde_json::to_vec(&before.files)? != serde_json::to_vec(&after.files)?
    {
        return Ok(false);
    }
    let (Some(before), Some(after)) = (&before.addon_selection, &after.addon_selection) else {
        return Ok(false);
    };
    if before.scopes.len() != after.scopes.len() {
        return Ok(false);
    }
    Ok(before.scopes.iter().all(|(scope, choices)| {
        let Some(updated) = after.scopes.get(scope) else {
            return false;
        };
        // The installer may add disabled entries and normalize folder casing.
        let choices: std::collections::BTreeMap<_, _> = choices
            .iter()
            .map(|(name, enabled)| (name.to_lowercase(), *enabled))
            .collect();
        let updated: std::collections::BTreeMap<_, _> = updated
            .iter()
            .map(|(name, enabled)| (name.to_lowercase(), *enabled))
            .collect();
        choices
            .iter()
            .all(|(name, enabled)| updated.get(name) == Some(enabled))
            && updated
                .iter()
                .all(|(name, enabled)| !enabled || choices.contains_key(name))
    }))
}

pub fn restore_history(
    store: &Store,
    root: &Path,
    client: Client,
    catalog: &[Definition],
    session: &Session,
    point: &RestorePoint,
) -> Result<String> {
    let _lock = store.lock()?;
    let portable_catalog = crate::library::catalog();
    let (profile, before) = store.cloud_bundle(&portable_catalog)?;
    if profile != point.profile {
        return Err(
            invalid("The active profile changed. Refresh cloud backups before restoring.").into(),
        );
    }
    let service = Service::new();
    if service.current_snapshot(session, client, profile)?.revision != point.revision {
        return Err(
            invalid("The cloud backup changed. Refresh cloud backups before restoring.").into(),
        );
    }
    let historical = service.historical_snapshot(session, client, profile, &point.id)?;
    let (target, hash) = Bundle::decode_with_hash(
        &historical.payload,
        historical.hash.as_deref(),
        &portable_catalog,
    )?;
    let installed = shared_library::install_for_restore(root, client, catalog, store, &target)?;
    let (current_profile, after) = store.cloud_bundle(&portable_catalog)?;
    if current_profile != profile || !setup_matches_after_install(&before, &after, installed)? {
        return Err(
            invalid("Your local setup changed. Refresh cloud backups before restoring.").into(),
        );
    }
    let mut revision = None;
    store.apply_cloud_bundle_with(target, &portable_catalog, || {
        revision = Some(
            service
                .promote_snapshot(session, client, profile, &point.id, point.revision, &hash)
                .map_err(io::Error::other)?
                .revision,
        );
        Ok(())
    })?;
    let revision =
        revision.ok_or_else(|| invalid("Astral could not confirm the restored revision."))?;
    let (_, local) = store.cloud_bundle(&portable_catalog)?;
    preferences::save_json(
        &store.sync_record(profile),
        &Record {
            session: session.fingerprint(),
            local: local.normalized_hash()?,
            remote: Some(hash),
            revision,
            synced_at: Some(time::OffsetDateTime::now_utc().unix_timestamp()),
        },
    )?;
    store.clear_sync_pending(profile)?;
    Ok(if installed > 0 {
        format!("Cloud backup restored. Installed {installed} required addons.")
    } else {
        "Cloud backup restored.".into()
    })
}

#[derive(Deserialize, Serialize)]
struct Record {
    session: String,
    local: String,
    remote: Option<String>,
    revision: u64,
    #[serde(default)]
    synced_at: Option<i64>,
}

fn read_record(path: &Path, session_id: &str) -> io::Result<Option<Record>> {
    match settings_backup::read_bounded(path, 4096) {
        Ok(bytes) => {
            let record: Record = serde_json::from_slice(&bytes)?;
            Ok((record.session == session_id).then_some(record))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Preserve the settings baseline when a collection-only install is the sole change.
/// The caller holds the settings lock through capture, installation, and recapture.
pub(crate) fn with_collection_baseline(
    store: &Store,
    session: &Session,
    install: impl FnOnce() -> Result<usize>,
) -> Result<usize> {
    let catalog = crate::library::catalog();
    with_collection_baseline_capture(store, session, install, || {
        store.cloud_bundle(&catalog)?.1.normalized_hash()
    })
}

fn with_collection_baseline_capture(
    store: &Store,
    session: &Session,
    install: impl FnOnce() -> Result<usize>,
    capture_hash: impl Fn() -> io::Result<String>,
) -> Result<usize> {
    let path = store.sync_record(store.profile_state()?.active);
    let Some(mut record) = read_record(&path, &session.fingerprint())? else {
        return install();
    };
    let before = capture_hash()?;
    let installed = install()?;
    if installed > 0 && record.local == before {
        record.local = capture_hash()?;
        preferences::save_json(&path, &record)?;
    }
    Ok(installed)
}

pub(crate) fn last_sync(
    store: &Store,
    profile: Profile,
    session: &Session,
) -> io::Result<Option<i64>> {
    let state = store.profile_state()?;
    if state.active == profile && state.sync_pending {
        return Ok(None);
    }
    Ok(
        read_record(&store.sync_record(profile), &session.fingerprint())?
            .and_then(|record| record.synced_at),
    )
}

#[derive(PartialEq, Eq)]
enum Action {
    Upload,
    Download,
    Same,
    Conflict,
}

fn decide(
    local: &str,
    remote: Option<&str>,
    equivalent: bool,
    record: Option<&Record>,
    choice: Option<Choice>,
    local_empty: bool,
) -> Action {
    // Keep the same precedence as Electron's syncAction, including its empty
    // account bootstrap and unchanged-baseline checks before explicit choices.
    if remote.is_none() {
        return Action::Upload;
    }
    if equivalent
        || record.is_some_and(|record| record.local == local && record.remote.as_deref() == remote)
    {
        return Action::Same;
    }
    if matches!(choice, Some(Choice::Cloud))
        || local_empty
        || record.is_some_and(|record| record.local == local)
    {
        return Action::Download;
    }
    if matches!(choice, Some(Choice::Local))
        || record.is_some_and(|record| record.remote.as_deref() == remote)
    {
        return Action::Upload;
    }
    Action::Conflict
}

pub fn run(
    store: &Store,
    root: &Path,
    client: Client,
    catalog: &[Definition],
    session: &Session,
    resolve: Option<(Conflict, Choice)>,
    automatic: bool,
) -> Result<Outcome> {
    let _lock = store.lock()?;
    let state = store.profile_state()?;
    let active = state.active;
    if automatic && state.sync_pending {
        return Ok(Outcome::ProfilePending);
    }
    let record_path = store.sync_record(active);
    let session_id = session.fingerprint();
    let record = read_record(&record_path, &session_id)?.filter(|_| !state.sync_pending);
    // Only the published Electron allowlist can travel between the apps.
    let portable_catalog = crate::library::catalog();
    let (profile, bundle) = store.cloud_bundle(&portable_catalog)?;
    let encoded = bundle.encode(&portable_catalog)?;
    let service = Service::new();
    let remote = service.current_snapshot(session, client, profile)?;
    let record = record.filter(|record| record.revision <= remote.revision);
    let incoming = if remote.revision == 0 {
        None
    } else {
        let payload = remote
            .payload
            .as_deref()
            .ok_or_else(|| invalid("Astral returned an incomplete cloud backup."))?;
        Some(Bundle::decode(
            payload,
            remote.hash.as_deref(),
            &portable_catalog,
        )?)
    };
    // A legacy Worker revision can lack a hash. Use the normalized digest for its identity.
    let normalized = incoming.as_ref().map(Bundle::normalized_hash).transpose()?;
    let remote_hash = remote.hash.as_deref().or(normalized.as_deref());
    let reviewed_setup = || Conflict {
        profile,
        store: record_path.clone(),
        revision: remote.revision,
        local: encoded.hash.clone(),
        remote: remote_hash.map(str::to_owned),
        session: session_id.clone(),
    };
    if let Some((reviewed, _)) = &resolve
        && !reviewed.matches(&reviewed_setup())
    {
        return Err(invalid(
            "Your local setup or the cloud backup changed. Sync again to review it.",
        )
        .into());
    }
    let equivalent = normalized.as_deref() == Some(encoded.hash.as_str());
    let action = decide(
        &encoded.hash,
        remote_hash,
        equivalent,
        record.as_ref(),
        resolve.map(|(_, choice)| choice),
        bundle.files.is_empty(),
    );
    if action == Action::Conflict {
        return Ok(Outcome::Conflict(reviewed_setup()));
    }
    let (latest_profile, latest) = store.cloud_bundle(&portable_catalog)?;
    if latest_profile != profile || latest.normalized_hash()? != encoded.hash {
        return Err(invalid("Your settings changed during sync. Try again.").into());
    }
    let target = if action == Action::Download {
        incoming.as_ref()
    } else {
        Some(&bundle)
    };
    let collection = shared_library::sync(
        &service,
        session,
        root,
        client,
        catalog,
        Some(store),
        target.map(|bundle| (profile, bundle)),
    )?;
    if !collection.manual.is_empty() {
        return Ok(Outcome::Incomplete(collection));
    }
    if matches!(action, Action::Same | Action::Download) {
        let latest = service.current_snapshot(session, client, profile)?;
        if latest.revision != remote.revision || latest.hash != remote.hash {
            return Err(invalid("The cloud backup changed during sync. Try again.").into());
        }
    }
    let (latest_profile, latest) = store.cloud_bundle(&portable_catalog)?;
    if latest_profile != profile
        || !setup_matches_after_install(&bundle, &latest, collection.installed)?
    {
        return Err(invalid("Your settings changed during sync. Try again.").into());
    }
    let encoded = if collection.installed > 0 {
        latest.encode(&portable_catalog)?
    } else {
        encoded
    };
    let (message, local, remote_hash, revision) = match action {
        Action::Conflict => unreachable!(),
        Action::Same => (
            "Settings are up to date.",
            encoded.hash,
            remote_hash.map(str::to_owned),
            remote.revision,
        ),
        Action::Upload => {
            let saved =
                service.upload_snapshot(session, client, profile, remote.revision, &encoded)?;
            (
                "Settings backed up to Astral.",
                encoded.hash.clone(),
                Some(encoded.hash),
                saved.revision,
            )
        }
        Action::Download => {
            let incoming = incoming.ok_or_else(|| invalid("No cloud backup is available."))?;
            store.apply_cloud_bundle(incoming, &portable_catalog)?;
            let (_, local) = store.cloud_bundle(&portable_catalog)?;
            (
                "Cloud settings restored.",
                local.encode(&portable_catalog)?.hash,
                remote_hash.map(str::to_owned),
                remote.revision,
            )
        }
    };
    preferences::save_json(
        &record_path,
        &Record {
            session: session_id,
            local,
            remote: remote_hash,
            revision,
            synced_at: Some(time::OffsetDateTime::now_utc().unix_timestamp()),
        },
    )?;
    store.clear_sync_pending(profile)?;
    let installed = collection.installed;
    let message = if installed > 0 {
        format!("{message} Installed {installed} shared addons.")
    } else {
        message.into()
    };
    Ok(Outcome::Synced { message, installed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_after_addon_install_allows_disabled_entries_but_rejects_settings_edits() -> Result<()>
    {
        use crate::sync_format::{LibraryAddon, Selection};
        let before = Bundle {
            schema: 2,
            files: vec![settings_backup::Entry::new(
                "bindings-cache.wtf".into(),
                b"before",
            )],
            addon_selection: Some(Selection {
                schema: 1,
                scopes: [("".into(), [("KeyUI".into(), true)].into())].into(),
            }),
            library: Some(vec![]),
            input: None,
        };
        let mut after = before.clone();
        after.library.as_mut().unwrap().push(LibraryAddon {
            folder: "Example".into(),
            name: "Example".into(),
        });
        let choices = &mut after.addon_selection.as_mut().unwrap().scopes;
        choices.insert(
            "".into(),
            [("keyui".into(), true), ("Example".into(), false)].into(),
        );
        assert!(setup_matches_after_install(&before, &after, 1)?);
        assert!(!setup_matches_after_install(&before, &after, 0)?);

        let mut edited = after.clone();
        edited.files[0] = settings_backup::Entry::new("bindings-cache.wtf".into(), b"changed");
        assert!(!setup_matches_after_install(&before, &edited, 1)?);
        after
            .addon_selection
            .as_mut()
            .unwrap()
            .scopes
            .get_mut("")
            .unwrap()
            .insert("Example".into(), true);
        assert!(!setup_matches_after_install(&before, &after, 1)?);
        Ok(())
    }

    #[test]
    fn collection_only_install_advances_a_clean_baseline_without_hiding_local_edits() -> Result<()>
    {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_forever_");
        std::fs::create_dir_all(root.join("WTF/Account/AccountOne"))?;
        let store = Store::open(temp.path(), &root, Client::Forever, "AccountOne")?;
        let session: Session = serde_json::from_value(serde_json::json!({
            "name": "Test",
            "token": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        }))?;
        let path = store.sync_record(Profile::Regular);
        let record = Record {
            session: session.fingerprint(),
            local: "last-synced".into(),
            remote: Some("previous-cloud".into()),
            revision: 3,
            synced_at: Some(1_800_000_000),
        };
        for (before, expected, action) in [
            ("last-synced", "with-shared-addon", Action::Download),
            ("changed-bindings", "last-synced", Action::Conflict),
        ] {
            preferences::save_json(&path, &record)?;
            let hash = std::cell::Cell::new(before);
            let installed = with_collection_baseline_capture(
                &store,
                &session,
                || {
                    hash.set("with-shared-addon");
                    Ok(1)
                },
                || Ok(hash.get().into()),
            )?;
            assert_eq!(installed, 1);
            let saved = read_record(&path, &record.session)?.unwrap();
            assert_eq!(saved.local, expected);
            assert_eq!(saved.remote, record.remote);
            assert_eq!(saved.revision, record.revision);
            assert_eq!(saved.synced_at, record.synced_at);
            assert!(
                decide(
                    hash.get(),
                    Some("new-cloud"),
                    false,
                    Some(&saved),
                    None,
                    false
                ) == action
            );
        }
        Ok(())
    }

    #[test]
    fn last_sync_is_profile_and_session_specific_and_accepts_older_records() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        std::fs::create_dir_all(root.join("WTF/Account/AccountOne"))?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let session: Session = serde_json::from_value(serde_json::json!({
            "name": "Test",
            "token": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        }))?;
        let path = store.sync_record(Profile::Regular);
        assert_eq!(last_sync(&store, Profile::Regular, &session)?, None);
        preferences::save_json(
            &path,
            &serde_json::json!({
                "session": session.fingerprint(), "local": "old", "remote": null, "revision": 1,
            }),
        )?;
        assert_eq!(last_sync(&store, Profile::Regular, &session)?, None);
        let mut record = read_record(&path, &session.fingerprint())?.unwrap();
        record.synced_at = Some(1_800_000_000);
        preferences::save_json(&path, &record)?;
        assert_eq!(
            last_sync(&store, Profile::Regular, &session)?,
            record.synced_at
        );
        assert_eq!(last_sync(&store, Profile::Handheld, &session)?, None);
        record.session = "different session".into();
        preferences::save_json(&path, &record)?;
        assert_eq!(last_sync(&store, Profile::Regular, &session)?, None);
        Ok(())
    }

    #[test]
    fn switching_profiles_pauses_automatic_sync_until_an_explicit_sync_succeeds() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        std::fs::create_dir_all(root.join("WTF/Account/AccountOne"))?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let session: Session = serde_json::from_value(serde_json::json!({
            "name": "Test", "token": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        }))?;
        assert!(!store.profile_state()?.sync_pending);
        store.switch_checked(Profile::Handheld, &[], || Ok(()))?;
        assert!(store.profile_state()?.sync_pending);
        // Pending profiles stop before constructing a service or touching the network.
        assert!(matches!(
            run(&store, &root, Client::Retail, &[], &session, None, true)?,
            Outcome::ProfilePending
        ));
        assert!(store.clear_sync_pending(Profile::Regular).is_err());
        assert!(store.profile_state()?.sync_pending);
        store.clear_sync_pending(Profile::Handheld)?;
        assert!(!store.profile_state()?.sync_pending);
        store.switch_checked(Profile::Regular, &[], || Ok(()))?;
        assert!(store.profile_state()?.sync_pending);
        Ok(())
    }

    #[test]
    fn conflict_choice_matches_only_the_reviewed_setup() {
        let review = Conflict {
            profile: Profile::Regular,
            store: "retail-account/sync.json".into(),
            revision: 4,
            local: "local".into(),
            remote: Some("cloud".into()),
            session: "session".into(),
        };
        assert!(review.matches(&review.clone()));
        for changed in [
            Conflict {
                profile: Profile::Handheld,
                ..review.clone()
            },
            Conflict {
                store: "forever-account/sync.json".into(),
                ..review.clone()
            },
            Conflict {
                revision: 5,
                ..review.clone()
            },
            Conflict {
                local: "edited".into(),
                ..review.clone()
            },
            Conflict {
                remote: Some("recreated".into()),
                ..review.clone()
            },
            Conflict {
                session: "another account".into(),
                ..review.clone()
            },
        ] {
            assert!(!review.matches(&changed));
        }
    }

    #[test]
    fn sync_decisions_match_electron_for_first_sync_conflicts_empty_accounts_and_choices()
    -> Result<()> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") else {
            return Ok(());
        };
        let mut cases = Vec::new();
        let mut actual = Vec::new();
        for local in ["A", "B"] {
            for remote in [None, Some("A"), Some("B"), Some("C")] {
                for empty in [true, false] {
                    for last in [None, Some("A"), Some("B")] {
                        for last_remote in [None, Some("A"), Some("B")] {
                            for choice in [None, Some(Choice::Local), Some(Choice::Cloud)] {
                                let record =
                                    (last.is_some() || last_remote.is_some()).then(|| Record {
                                        session: "session".into(),
                                        local: last.unwrap_or("").into(),
                                        remote: last_remote.map(str::to_owned),
                                        revision: 1,
                                        synced_at: None,
                                    });
                                let choice_name = choice.map(|choice| match choice {
                                    Choice::Local => "local",
                                    Choice::Cloud => "cloud",
                                });
                                cases.push(serde_json::json!({ "local": { "hash": local, "files": if empty { 0 } else { 1 } },
                                    "remote": { "revision": if remote.is_some() { 1 } else { 0 }, "hash": remote },
                                    "last": last, "lastRemote": last_remote, "choice": choice_name }));
                                actual.push(
                                    match decide(
                                        local,
                                        remote,
                                        remote == Some(local),
                                        record.as_ref(),
                                        choice,
                                        empty,
                                    ) {
                                        Action::Upload => "upload",
                                        Action::Download => "download",
                                        Action::Same => "unchanged",
                                        Action::Conflict => "conflict",
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(cases.len(), 432);
        let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../desktop/flavors.cjs");
        let mut process = Command::new(node).arg("-e").arg(
            "const fs=require('node:fs');const {syncAction}=require(process.argv[1]);const cases=JSON.parse(fs.readFileSync(0,'utf8'));process.stdout.write(JSON.stringify(cases.map(c=>syncAction(c.local,c.remote,c.last,c.choice,c.lastRemote))));"
        ).arg(module).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        process
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&cases)?)?;
        let output = process.wait_with_output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected: Vec<String> = serde_json::from_slice(&output.stdout)?;
        for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(actual, expected, "case {}", cases[index]);
        }
        assert_eq!(actual.len(), expected.len());
        Ok(())
    }

    #[test]
    fn sync_requires_a_choice_when_both_sides_changed() {
        let record = Record {
            session: "session".into(),
            local: "A".into(),
            remote: Some("B".into()),
            revision: 3,
            synced_at: None,
        };
        assert!(decide("A", Some("B"), false, Some(&record), None, false) == Action::Same);
        assert!(decide("A", Some("C"), false, Some(&record), None, false) == Action::Download);
        assert!(decide("C", Some("B"), false, Some(&record), None, false) == Action::Upload);
        assert!(decide("C", Some("D"), false, Some(&record), None, false) == Action::Conflict);
        assert!(
            decide("C", Some("D"), false, None, Some(Choice::Cloud), false) == Action::Download
        );
        assert!(decide("C", Some("D"), false, None, Some(Choice::Local), false) == Action::Upload);
        assert!(decide("C", Some("D"), true, None, None, false) == Action::Same);
    }
}
