use crate::sync_addons::{self, ChoiceChange, ProfileAddon, ProfileEdits};
use crate::{Client, Definition, gamepad_settings, installer, preferences};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_FILE: usize = 8 * 1024 * 1024;
const MAX_SNAPSHOT: usize = 24 * 1024 * 1024;
const MAX_FILES: usize = 2_000;
const PROFILE_REVIEW_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    schema: u8,
    files: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    controller: Option<gamepad_settings::Settings>,
}

pub struct ProfileReview {
    pub target: Profile,
    pub name: String,
    pub initialized: bool,
    pub changed: Vec<String>,
    pub blocker: Option<String>,
    pub packages: Vec<String>,
    pub handheld: bool,
    pub addon_changes: Vec<ChoiceChange>,
    pub editing_addons: bool,
    edits: ProfileEdits,
    preset: bool,
    cloud_name: Option<String>,
    fingerprint: String,
    expires_at: Instant,
}

impl ProfileReview {
    pub(crate) fn fixture(client: Client) -> Self {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/profile-review.json"))
                .expect("profile review fixtures");
        let value = &fixtures[client.label().to_lowercase()];
        Self {
            target: Profile::Handheld,
            name: value["name"].as_str().unwrap().into(),
            initialized: value["initialized"].as_bool().unwrap(),
            changed: serde_json::from_value(value["changedSettings"].clone()).unwrap(),
            blocker: None,
            packages: value["packages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["name"].as_str().unwrap().to_owned())
                .collect(),
            handheld: true,
            addon_changes: value["changes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|change| ChoiceChange {
                    name: change["name"].as_str().unwrap().into(),
                    before: change["before"].as_bool(),
                    after: change["enabled"].as_bool(),
                })
                .collect(),
            editing_addons: false,
            edits: ProfileEdits::new(),
            preset: true,
            cloud_name: None,
            fingerprint: "comparison-only".into(),
            expires_at: Instant::now(),
        }
    }
}

struct PreparedSwitch {
    review: ProfileReview,
    state: ProfileState,
    current: Snapshot,
    saved: Option<Snapshot>,
    handheld_preset: bool,
    package_sources: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    #[default]
    Regular,
    Handheld,
    Custom(uuid::Uuid),
}

impl Profile {
    pub const ALL: [Self; 2] = [Self::Regular, Self::Handheld];

    pub fn label(self) -> &'static str {
        match self {
            Self::Regular => "Regular",
            Self::Handheld => "Handheld",
            Self::Custom(_) => "Custom profile",
        }
    }

    pub fn from_cloud_id(id: &str) -> Option<Self> {
        match id {
            "regular" => Some(Self::Regular),
            "handheld" => Some(Self::Handheld),
            _ if id.len() == 32
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
            {
                uuid::Uuid::parse_str(id).ok().map(Self::Custom)
            }
            _ => None,
        }
    }

    pub fn cloud_id(self) -> String {
        match self {
            Self::Regular => "regular".into(),
            Self::Handheld => "handheld".into(),
            Self::Custom(id) => id.simple().to_string(),
        }
    }

    fn file(self) -> String {
        match self {
            Self::Regular => "profile-regular.json".into(),
            Self::Handheld => "profile-handheld.json".into(),
            Self::Custom(id) => format!("profile-{}.json", id.simple()),
        }
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct ProfileState {
    pub active: Profile,
    pub interrupted: bool,
    #[serde(default)]
    pub handheld_preset: bool,
    #[serde(default)]
    pub sync_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    committed_package_switch: Option<uuid::Uuid>,

    #[serde(default)]
    names: Vec<(Profile, String)>,
}

impl ProfileState {
    pub fn label(&self, id: Profile) -> &str {
        self.names
            .iter()
            .find(|(profile, _)| *profile == id)
            .map_or_else(|| id.label(), |(_, name)| name.as_str())
    }

    pub fn profiles(&self) -> impl Iterator<Item = Profile> + '_ {
        Profile::ALL.into_iter().chain(
            self.names.iter().filter_map(|(profile, _)| {
                matches!(profile, Profile::Custom(_)).then_some(*profile)
            }),
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub(crate) path: String,
    sha256: String,
    content: String,
}

impl Entry {
    pub(crate) fn new(path: String, bytes: &[u8]) -> Self {
        Self {
            path,
            sha256: hash(bytes),
            content: STANDARD.encode(bytes),
        }
    }

    pub(crate) fn decode(&self) -> io::Result<Vec<u8>> {
        let bytes = STANDARD
            .decode(&self.content)
            .map_err(|_| invalid("Invalid settings data."))?;
        if bytes.len() > MAX_FILE || hash(&bytes) != self.sha256 {
            return Err(invalid("Settings backup checksum mismatch."));
        }
        Ok(bytes)
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(crate) fn safe_component(part: &str) -> bool {
    let device = part
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !part.is_empty()
        && !part.ends_with(['.', ' '])
        && !part
            .chars()
            .any(|c| c.is_control() || "/\\:<>|\"?*".contains(c))
        && !matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !["COM", "LPT"].iter().any(|prefix| {
            device.strip_prefix(prefix).is_some_and(|tail| {
                matches!(
                    tail,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

pub(crate) fn allowed(relative: &str, catalog: &[Definition]) -> bool {
    let parts: Vec<_> = relative.split('/').collect();
    if !parts.iter().all(|part| safe_component(part)) {
        return false;
    }
    let Some(name) = parts.last() else {
        return false;
    };
    match parts.len() {
        1 | 3 => {
            name.eq_ignore_ascii_case("AddOns.txt")
                || matches!(
                    *name,
                    "bindings-cache.wtf"
                        | "layout-local.txt"
                        | "edit-mode-cache-account.txt"
                        | "edit-mode-cache-character.txt"
                        | "cooldownmanager.txt"
                )
        }
        2 | 4 if parts[parts.len() - 2] == "SavedVariables" => {
            name.strip_suffix(".lua").is_some_and(|addon| {
                addon.starts_with("EllesmereUI")
                    || addon.starts_with("Blizzard_")
                    || catalog.iter().any(|source| source.includes(addon))
            })
        }
        _ => false,
    }
}

pub(crate) fn link(metadata: &fs::Metadata) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & REPARSE_POINT != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(crate) fn child(base: &Path, relative: &str) -> io::Result<PathBuf> {
    let mut path = base.to_path_buf();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        if !safe_component(part) {
            return Err(invalid("Invalid settings path."));
        }
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if link(&metadata) => {
                return Err(invalid("Settings paths cannot contain links."));
            }
            Ok(metadata) if parts.peek().is_some() && !metadata.is_dir() => {
                return Err(invalid("A settings folder is blocked by a file."));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(path)
}

pub(crate) fn read_bounded(path: &Path, max: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if link(&metadata) || !metadata.is_file() || metadata.len() > max as u64 {
        return Err(invalid(
            "A settings file is too large or is not a regular file.",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(invalid("A settings file exceeds the size limit."));
    }
    Ok(bytes)
}

pub fn accounts(root: &Path) -> io::Result<Vec<String>> {
    let base = child(&root.canonicalize()?, "WTF/Account")?;
    let folders = match fs::read_dir(base) {
        Ok(folders) => folders,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut names = Vec::new();
    for entry in folders {
        let entry = entry?;
        let Some(name) = entry.file_name().into_string().ok() else {
            continue;
        };
        if name == "SavedVariables" || !safe_component(&name) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() && !link(&metadata) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

pub struct Store {
    root: PathBuf,
    client: Client,
    lock_file: PathBuf,
    account: PathBuf,
    backups: PathBuf,
}

pub(crate) struct SettingsLock(File);

impl Drop for SettingsLock {
    fn drop(&mut self) {
        // A concurrent child can briefly hold an inherited copy of the descriptor.
        let _ = self.0.unlock();
    }
}

#[derive(Clone)]
pub struct Backup {
    pub id: String,
    pub time: u64,
}

impl Store {
    pub fn open(data: &Path, root: &Path, client: Client, account: &str) -> io::Result<Self> {
        if !safe_component(account) {
            return Err(invalid("Invalid WoW account."));
        }
        let root = root.canonicalize()?;
        let account = child(&root, &format!("WTF/Account/{account}"))?;
        if !account.is_dir() {
            return Err(invalid("The selected WoW account was not found."));
        }
        let identity = hash(account.as_os_str().as_encoded_bytes());
        let storage = data.join("settings-backups");
        let installation = hash(root.as_os_str().as_encoded_bytes());
        Ok(Self {
            root,
            client,
            lock_file: storage.join(format!("{installation}.lock")),
            account,
            backups: storage.join(client.label()).join(identity),
        })
    }

    pub fn list(&self) -> io::Result<Vec<Backup>> {
        let entries = match fs::read_dir(&self.backups) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut backups = Vec::new();
        for entry in entries {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if let Some(time) = backup_time(&id)
                && entry.file_type()?.is_file()
            {
                backups.push(Backup { id, time });
            }
        }
        backups.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(backups)
    }

    pub fn capture(&self, catalog: &[Definition]) -> io::Result<usize> {
        self.capture_checked(catalog, check_game)
    }

    fn capture_checked(
        &self,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<usize> {
        let _lock = self.lock()?;
        check()?;
        self.check_profile_ready()?;
        let snapshot = self.snapshot(catalog)?;
        if snapshot.files.is_empty()
            && snapshot
                .controller
                .as_ref()
                .is_none_or(gamepad_settings::Settings::is_empty)
        {
            return Err(invalid("No supported settings files were found."));
        }
        check()?;
        let count = snapshot.files.len()
            + usize::from(
                snapshot
                    .controller
                    .as_ref()
                    .is_some_and(|controller| !controller.is_empty()),
            );
        self.save(snapshot)?;
        Ok(count)
    }

    pub(crate) fn inspect_settings(&self, catalog: &[Definition]) -> io::Result<(usize, usize)> {
        let _lock = self.lock()?;
        check_game()?;
        let snapshot = self.snapshot(catalog)?;
        let mut bytes = 0;
        for entry in &snapshot.files {
            bytes += entry.decode()?.len();
        }
        check_game()?;
        Ok((snapshot.files.len(), bytes))
    }

    fn snapshot(&self, catalog: &[Definition]) -> io::Result<Snapshot> {
        let mut files = Vec::new();
        let mut size = 0;
        self.collect(&self.account, "", 1, catalog, &mut files, &mut size)?;
        let mut seen = HashSet::new();
        if files
            .iter()
            .any(|entry| !seen.insert(entry.path.to_lowercase()))
        {
            return Err(invalid("Two settings files differ only by letter case."));
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let (_, config) = self.read_config()?;
        let controller = Some(gamepad_settings::Settings::capture(
            config.as_deref().unwrap_or_default(),
        )?);
        Ok(Snapshot {
            schema: 1,
            files,
            controller,
        })
    }

    fn read_config(&self) -> io::Result<(PathBuf, Option<Vec<u8>>)> {
        let config = child(&self.root, "WTF/Config.wtf")?;
        let content = match read_bounded(&config, MAX_FILE) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        Ok((config, content))
    }

    fn collect(
        &self,
        directory: &Path,
        prefix: &str,
        depth: usize,
        catalog: &[Definition],
        files: &mut Vec<Entry>,
        size: &mut usize,
    ) -> io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let Some(name) = entry.file_name().into_string().ok() else {
                continue;
            };
            if !safe_component(&name) {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if link(&metadata) {
                continue;
            }
            let relative = format!("{prefix}{name}");
            if metadata.is_dir() && depth < 4 {
                self.collect(
                    &entry.path(),
                    &format!("{relative}/"),
                    depth + 1,
                    catalog,
                    files,
                    size,
                )?;
            } else if metadata.is_file() && allowed(&relative, catalog) {
                let bytes = read_bounded(&entry.path(), MAX_FILE)?;
                *size += bytes.len().div_ceil(3) * 4 + relative.len() + 120;
                if files.len() == MAX_FILES || *size > MAX_SNAPSHOT {
                    return Err(invalid("This setup exceeds the settings backup limit."));
                }
                files.push(Entry::new(relative, &bytes));
            }
        }
        Ok(())
    }

    fn save(&self, snapshot: Snapshot) -> io::Result<()> {
        let bytes = serde_json::to_vec(&snapshot)?;
        if bytes.len() > MAX_SNAPSHOT {
            return Err(invalid("This setup exceeds the settings backup limit."));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        fs::create_dir_all(&self.backups)?;
        let mut temp = tempfile::Builder::new()
            .prefix(".pending-")
            .tempfile_in(&self.backups)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        let suffix = temp
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let id = format!("{now:013}-{}.json", suffix.trim_start_matches(".pending-"));
        temp.persist_noclobber(self.backups.join(id))?;
        Ok(())
    }

    pub(crate) fn lock(&self) -> io::Result<SettingsLock> {
        let parent = self.lock_file.parent().ok_or(io::ErrorKind::InvalidInput)?;
        fs::create_dir_all(parent)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock_file)?;
        lock.try_lock().map_err(|error| {
            let error = io::Error::from(error);
            if error.kind() == io::ErrorKind::WouldBlock {
                io::Error::other("Another settings backup or restore is in progress.")
            } else {
                error
            }
        })?;
        Ok(SettingsLock(lock))
    }

    pub fn restore(&self, id: &str, catalog: &[Definition]) -> io::Result<usize> {
        self.restore_checked(id, catalog, check_game)
    }

    fn restore_checked(
        &self,
        id: &str,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<usize> {
        let _lock = self.lock()?;
        check()?;
        self.check_profile_ready()?;
        if backup_time(id).is_none() {
            return Err(invalid("Invalid settings backup."));
        }
        let snapshot = Self::read_snapshot(&self.backups.join(id))?;
        self.apply(snapshot, &[], catalog, check, || Ok(()), || Ok(()))
    }

    fn read_snapshot(path: &Path) -> io::Result<Snapshot> {
        let snapshot: Snapshot = serde_json::from_slice(&read_bounded(path, MAX_SNAPSHOT)?)?;
        if snapshot.schema != 1 || snapshot.files.len() > MAX_FILES {
            return Err(invalid("Unsupported settings backup."));
        }
        Ok(snapshot)
    }

    fn profile_state_path(&self) -> PathBuf {
        self.backups.join("active-profile.json")
    }

    pub fn profile_state(&self) -> io::Result<ProfileState> {
        match read_bounded(&self.profile_state_path(), 4096) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ProfileState::default()),
            Err(error) => Err(error),
        }
    }

    fn check_profile_ready(&self) -> io::Result<ProfileState> {
        let state = self.profile_state()?;
        if state.interrupted {
            return Err(invalid("Recover the interrupted profile switch first."));
        }
        Ok(state)
    }

    pub(crate) fn cloud_bundle(
        &self,
        catalog: &[Definition],
    ) -> io::Result<(Profile, crate::sync_format::Bundle)> {
        self.cloud_bundle_checked(catalog, check_game)
    }

    fn cloud_bundle_checked(
        &self,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<(Profile, crate::sync_format::Bundle)> {
        use crate::sync_format::{Bundle, LibraryAddon, portable_file, valid_addon};
        check()?;
        installer::ensure_no_pending_profile_packages(&self.root).map_err(io::Error::other)?;
        let active = self.check_profile_ready()?.active;
        let snapshot = self.snapshot(catalog)?;
        let (_, config) = self.read_config()?;
        let choices = crate::sync_addons::Inventory::open(
            &self.root,
            &self.account,
            self.client,
            config.as_deref().unwrap_or_default(),
        )?;
        let mut addons = crate::library::scan_addons(&self.root, self.client, catalog, false)?;
        addons.sort_by(|a, b| a.folder.cmp(&b.folder));
        let library = addons
            .into_iter()
            .filter(|addon| valid_addon(&addon.folder, 100))
            .map(|addon| LibraryAddon {
                folder: addon.folder,
                name: addon
                    .name
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(60)
                    .collect(),
            })
            .collect();
        let files = snapshot
            .files
            .into_iter()
            .filter(|entry| portable_file(&entry.path, catalog))
            .collect();
        let addon_selection = choices.selection()?;
        check()?;
        Ok((
            active,
            Bundle {
                schema: 2,
                files,
                addon_selection: Some(addon_selection),
                library: Some(library),
                input: snapshot.controller,
            },
        ))
    }

    pub(crate) fn apply_cloud_bundle(
        &self,
        bundle: crate::sync_format::Bundle,
        catalog: &[Definition],
    ) -> io::Result<usize> {
        self.apply_cloud_bundle_with(bundle, catalog, || Ok(()))
    }

    pub(crate) fn apply_cloud_bundle_with(
        &self,
        bundle: crate::sync_format::Bundle,
        catalog: &[Definition],
        commit: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<usize> {
        self.apply_cloud_bundle_checked(bundle, catalog, check_game, commit)
    }

    fn apply_cloud_bundle_checked(
        &self,
        bundle: crate::sync_format::Bundle,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
        commit: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<usize> {
        check()?;
        bundle.validate(catalog)?;
        let state = self.check_profile_ready()?;
        let mut target = Snapshot {
            schema: 1,
            files: bundle.files,
            controller: bundle.input,
        };
        if let Some(selection) = bundle.addon_selection {
            let (_, config) = self.read_config()?;
            let choices = crate::sync_addons::Inventory::open(
                &self.root,
                &self.account,
                self.client,
                config.as_deref().unwrap_or_default(),
            )?;
            target
                .files
                .extend(choices.plan(&self.account, &selection)?);
        }
        if state.handheld_preset && !matches!(state.active, Profile::Custom(_)) {
            let current = self.snapshot(catalog)?;
            self.apply_profile_policy(state.active, &mut target, &current, catalog)?;
        }
        self.apply(target, &[], catalog, check, || Ok(()), commit)
    }

    /// Called with the settings lock held after an explicit sync or restore succeeds.
    pub(crate) fn clear_sync_pending(&self, profile: Profile) -> io::Result<()> {
        let mut state = self.profile_state()?;
        if state.active != profile || state.interrupted {
            return Err(invalid("The active profile changed during sync."));
        }
        if state.sync_pending {
            state.sync_pending = false;
            preferences::save_json(&self.profile_state_path(), &state)?;
        }
        Ok(())
    }

    pub(crate) fn sync_record(&self, profile: Profile) -> PathBuf {
        self.backups.join(format!("sync-{}", profile.file()))
    }

    pub(crate) fn addon_choices(&self) -> io::Result<crate::sync_format::Selection> {
        check_game()?;
        self.check_profile_ready()?;
        let (_, config) = self.read_config()?;
        crate::sync_addons::Inventory::open(
            &self.root,
            &self.account,
            self.client,
            config.as_deref().unwrap_or_default(),
        )?
        .selection()
    }

    pub(crate) fn preserve_addon_choices(
        &self,
        selection: &crate::sync_format::Selection,
        catalog: &[Definition],
    ) -> io::Result<()> {
        self.preserve_addon_choices_checked(selection, catalog, check_game)
    }

    fn preserve_addon_choices_checked(
        &self,
        selection: &crate::sync_format::Selection,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        let (_, config) = self.read_config()?;
        let inventory = crate::sync_addons::Inventory::open(
            &self.root,
            &self.account,
            self.client,
            config.as_deref().unwrap_or_default(),
        )?;
        self.apply(
            Snapshot {
                schema: 1,
                files: inventory.plan(&self.account, selection)?,
                controller: None,
            },
            &[],
            catalog,
            check,
            || Ok(()),
            || Ok(()),
        )?;
        Ok(())
    }

    pub fn save_profile_name(&self, profile: Option<Profile>, name: &str) -> io::Result<Profile> {
        let name = name.trim();
        if name.is_empty() || name.encode_utf16().count() > 40 || name.chars().any(char::is_control)
        {
            return Err(invalid("Enter a profile name between 1 and 40 characters."));
        }
        let _lock = self.lock()?;
        let mut state = self.check_profile_ready()?;
        let id = match profile {
            Some(id) if state.profiles().any(|profile| profile == id) => id,
            Some(_) => return Err(invalid("Profile not found.")),
            None if state.profiles().count() < 8 => Profile::Custom(uuid::Uuid::new_v4()),
            None => return Err(invalid("You can add up to six custom profiles.")),
        };
        if state
            .profiles()
            .any(|other| other != id && state.label(other).to_lowercase() == name.to_lowercase())
        {
            return Err(invalid("A profile already uses that name."));
        }
        if let Some((_, current)) = state.names.iter_mut().find(|(profile, _)| *profile == id) {
            *current = name.into();
        } else {
            state.names.push((id, name.into()));
        }
        preferences::save_json(&self.profile_state_path(), &state)?;
        Ok(id)
    }

    pub fn import_cloud_profile(&self, id: Profile, name: &str) -> io::Result<()> {
        let name = name.trim();
        if name.is_empty() || name.encode_utf16().count() > 40 || name.chars().any(char::is_control)
        {
            return Err(invalid("Invalid cloud profile name."));
        }
        let _lock = self.lock()?;
        let mut state = self.check_profile_ready()?;
        if let Some((_, previous)) = state.names.iter_mut().find(|(profile, _)| *profile == id) {
            *previous = name.into();
        } else {
            // A cloud collection may contain six profiles in addition to the
            // six originally created on this computer.
            if state.names.len() >= 14 {
                return Err(invalid("Remove a local profile before adding another."));
            }
            state.names.push((id, name.into()));
        }
        preferences::save_json(&self.profile_state_path(), &state)
    }

    pub fn delete_profile(&self, profile: Profile) -> io::Result<()> {
        let _lock = self.lock()?;
        let mut state = self.check_profile_ready()?;
        if !matches!(profile, Profile::Custom(_)) || state.active == profile {
            return Err(invalid("Only inactive custom profiles can be deleted."));
        }
        if !state.profiles().any(|id| id == profile) {
            return Err(invalid("Profile not found."));
        }
        let path = self.backups.join(profile.file());
        let archived = match Self::read_snapshot(&path) {
            Ok(snapshot) => {
                self.save(snapshot)?;
                true
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        state.names.retain(|(id, _)| *id != profile);
        preferences::save_json(&self.profile_state_path(), &state)?;
        if archived {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn switch_checked(
        &self,
        target: Profile,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        self.switch_with_policy(target, false, catalog, check)
    }

    #[cfg(test)]
    fn switch_with_policy(
        &self,
        target: Profile,
        enable_preset: bool,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        check()?;
        let prepared = self.prepare_switch(target, enable_preset, catalog)?;
        self.commit_switch(prepared, catalog, check, None)
    }

    pub fn review_profile(
        &self,
        target: Profile,
        preset: bool,
        catalog: &[Definition],
        edits: ProfileEdits,
    ) -> io::Result<ProfileReview> {
        let _lock = self.lock()?;
        check_game()?;
        installer::ensure_no_pending_profile_packages(&self.root).map_err(io::Error::other)?;
        let review = self.prepare_profile(target, preset, catalog, edits)?.review;
        check_game()?;
        Ok(review)
    }

    pub fn review_cloud_profile(
        &self,
        target: Profile,
        name: &str,
        catalog: &[Definition],
    ) -> io::Result<ProfileReview> {
        let _lock = self.lock()?;
        check_game()?;
        installer::ensure_no_pending_profile_packages(&self.root).map_err(io::Error::other)?;
        let review = self
            .prepare_named_profile(
                target,
                target == Profile::Handheld,
                catalog,
                ProfileEdits::new(),
                Some(name),
            )?
            .review;
        check_game()?;
        Ok(review)
    }

    pub fn apply_review(&self, reviewed: &ProfileReview, catalog: &[Definition]) -> io::Result<()> {
        self.apply_review_with(
            reviewed,
            catalog,
            check_game,
            |sources, operation, finish| {
                let packages = sources
                    .iter()
                    .map(|&source| {
                        crate::batch::inspect_missing(source, &self.root, self.client, catalog)
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(io::Error::other)?;
                installer::install_profile_packages(
                    &self.root,
                    self.client,
                    &packages,
                    operation,
                    check_game,
                    finish,
                )
                .map(|_| ())
                .map_err(io::Error::other)
            },
        )
    }

    #[cfg(test)]
    fn apply_review_checked(
        &self,
        reviewed: &ProfileReview,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        self.apply_review_with(reviewed, catalog, check, |_, _, _| {
            Err(invalid("No package installer was supplied."))
        })
    }

    fn apply_review_with(
        &self,
        reviewed: &ProfileReview,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
        install: impl FnOnce(
            &[usize],
            installer::ProfileSwitch,
            &mut dyn FnMut() -> io::Result<()>,
        ) -> io::Result<()>,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        if Instant::now() >= reviewed.expires_at {
            return Err(invalid("This review expired. Review the profile again."));
        }
        check()?;
        installer::ensure_no_pending_profile_packages(&self.root).map_err(io::Error::other)?;
        let prepare = || {
            self.prepare_named_profile(
                reviewed.target,
                reviewed.preset,
                catalog,
                reviewed.edits.clone(),
                reviewed.cloud_name.as_deref(),
            )
        };
        let prepared = prepare()?;
        if prepared.review.fingerprint != reviewed.fingerprint {
            return Err(invalid("Your setup changed. Review the profile again."));
        }
        if prepared.package_sources.is_empty() || prepared.review.blocker.is_some() {
            return self.commit_switch(prepared, catalog, check, None);
        }

        let id = uuid::Uuid::new_v4();
        let account = self
            .account
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("Invalid WoW account."))?
            .to_owned();
        install(
            &prepared.package_sources,
            installer::ProfileSwitch { account, id },
            &mut || {
                check()?;
                let next = prepare()?;
                if serde_json::to_vec(&next.state)? != serde_json::to_vec(&prepared.state)?
                    || serde_json::to_vec(&next.current)? != serde_json::to_vec(&prepared.current)?
                {
                    return Err(invalid(
                        "Your setup changed while preparing addons. Review the profile again.",
                    ));
                }
                self.commit_switch(next, catalog, &check, Some(id))
            },
        )
    }

    #[cfg(test)]
    fn prepare_switch(
        &self,
        target: Profile,
        enable_preset: bool,
        catalog: &[Definition],
    ) -> io::Result<PreparedSwitch> {
        self.prepare_profile(target, enable_preset, catalog, ProfileEdits::new())
    }

    fn addon_inventory(&self) -> io::Result<sync_addons::Inventory> {
        let (_, config) = self.read_config()?;
        sync_addons::Inventory::open(
            &self.root,
            &self.account,
            self.client,
            config.as_deref().unwrap_or_default(),
        )
    }

    fn managed_addon(&self, profile: Profile, folder: &str) -> bool {
        self.client == Client::Retail
            && !matches!(profile, Profile::Custom(_))
            && folder == "ConsolePort"
    }

    pub fn profile_addons(&self, catalog: &[Definition]) -> io::Result<Vec<ProfileAddon>> {
        let state = self.profile_state()?;
        let selection = self.addon_inventory()?.selection()?;
        let mut rows: Vec<_> =
            crate::library::scan_addons(&self.root, self.client, catalog, false)?
                .iter()
                .map(|addon| ProfileAddon {
                    folder: addon.folder.clone(),
                    name: addon.name.clone(),
                    enabled: sync_addons::group_enabled(&selection, addon),
                    managed: self.managed_addon(state.active, &addon.folder),
                })
                .collect();
        rows.sort_by_key(|row| row.name.to_lowercase());
        Ok(rows)
    }

    fn prepare_profile(
        &self,
        target: Profile,
        enable_preset: bool,
        catalog: &[Definition],
        edits: ProfileEdits,
    ) -> io::Result<PreparedSwitch> {
        self.prepare_named_profile(target, enable_preset, catalog, edits, None)
    }

    fn prepare_named_profile(
        &self,
        target: Profile,
        enable_preset: bool,
        catalog: &[Definition],
        edits: ProfileEdits,
        cloud_name: Option<&str>,
    ) -> io::Result<PreparedSwitch> {
        let state = self.check_profile_ready()?;
        let known = state.profiles().any(|profile| profile == target);
        if let Some(name) = cloud_name {
            if name.trim().is_empty()
                || name.encode_utf16().count() > 40
                || name.chars().any(char::is_control)
            {
                return Err(invalid("Invalid cloud profile name."));
            }
            if !state.names.iter().any(|(profile, _)| *profile == target) && state.names.len() >= 14
            {
                return Err(invalid("Remove a local profile before adding another."));
            }
        } else if !known {
            return Err(invalid("Profile not found."));
        }
        let current = self.snapshot(catalog)?;
        let mut saved = if target == state.active {
            Some(current.clone())
        } else {
            match Self::read_snapshot(&self.backups.join(target.file())) {
                Ok(snapshot) => Some(snapshot),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            }
        };
        let initialized = saved.is_none();
        let handheld_preset = state.handheld_preset || enable_preset;
        let mut blocker = None;
        let mut package_sources = Vec::new();
        if (target != state.active || enable_preset)
            && handheld_preset
            && !matches!(target, Profile::Custom(_))
        {
            let snapshot = saved.get_or_insert_with(|| current.clone());
            if target == Profile::Handheld {
                let (sources, missing) = self.handheld_requirements(catalog)?;
                package_sources = sources;
                if !missing.is_empty() {
                    blocker = Some(format!("Connect or install first: {}.", missing.join(", ")));
                }
            }
            if blocker.is_none() && package_sources.is_empty() {
                blocker = self
                    .apply_profile_policy(target, snapshot, &current, catalog)
                    .err()
                    .map(|error| error.to_string());
            }
        }
        let mut addon_changes = Vec::new();
        if !edits.is_empty() && !self.root.join("Interface/AddOns").is_dir() {
            return Err(invalid("The WoW addon folder has moved."));
        }
        if let Some(snapshot) = &mut saved
            && self.root.join("Interface/AddOns").is_dir()
        {
            let addons = crate::library::scan_addons(&self.root, self.client, catalog, false)?;
            let before = self.addon_inventory()?.selection()?;
            let inventory = self.addon_inventory()?.with_snapshot(&snapshot.files)?;
            if !edits.is_empty() && blocker.is_none() {
                if edits
                    .keys()
                    .any(|folder| self.managed_addon(target, folder))
                {
                    return Err(invalid(
                        "ConsolePort is managed by the Regular and Handheld profiles.",
                    ));
                }
                match inventory.plan_profile_edits(&self.account, &addons, &edits) {
                    Ok(entries) => {
                        for entry in entries {
                            snapshot
                                .files
                                .retain(|old| !old.path.eq_ignore_ascii_case(&entry.path));
                            snapshot.files.push(entry);
                        }
                    }
                    Err(error) => blocker = Some(error.to_string()),
                }
            }
            let after = self
                .addon_inventory()?
                .with_snapshot(&snapshot.files)?
                .selection()?;
            for addon in &addons {
                let previous = sync_addons::group_enabled(&before, addon);
                let next = sync_addons::group_enabled(&after, addon);
                if previous != next {
                    addon_changes.push(ChoiceChange {
                        name: addon.name.clone(),
                        before: previous,
                        after: next,
                    });
                }
            }
        }
        let mut changed = Vec::new();
        if let Some(saved) = &saved {
            let file_hashes = |snapshot: &Snapshot| -> BTreeMap<String, String> {
                snapshot
                    .files
                    .iter()
                    .map(|entry| (entry.path.clone(), entry.sha256.clone()))
                    .collect()
            };
            let before = file_hashes(&current);
            let after = file_hashes(saved);
            let paths: BTreeSet<_> = before.keys().chain(after.keys()).collect();
            changed.extend(
                paths
                    .into_iter()
                    .filter(|path| before.get(*path) != after.get(*path))
                    .filter(|path| {
                        !path
                            .rsplit('/')
                            .next()
                            .is_some_and(|name| name.eq_ignore_ascii_case("AddOns.txt"))
                    })
                    .cloned(),
            );
            if saved.controller.is_some() && saved.controller != current.controller {
                changed.push("Controller settings".into());
            }
        }
        let fingerprint = hash(&serde_json::to_vec(&(
            &self.root,
            &self.account,
            &state,
            &current,
            &saved,
            &blocker,
            package_sources
                .iter()
                .map(|&index| &catalog[index])
                .collect::<Vec<_>>(),
            cloud_name,
        ))?);
        let review = ProfileReview {
            target,
            name: cloud_name.unwrap_or_else(|| state.label(target)).into(),
            initialized,
            changed,
            blocker,
            packages: package_sources
                .iter()
                .map(|&index| catalog[index].name.clone())
                .collect(),
            handheld: target == Profile::Handheld && handheld_preset,
            addon_changes,
            editing_addons: !edits.is_empty(),
            edits,
            preset: enable_preset,
            cloud_name: cloud_name.map(str::to_owned),
            fingerprint,
            expires_at: Instant::now() + PROFILE_REVIEW_TTL,
        };
        Ok(PreparedSwitch {
            review,
            state,
            current,
            saved,
            handheld_preset,
            package_sources,
        })
    }

    fn commit_switch(
        &self,
        prepared: PreparedSwitch,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
        package_switch: Option<uuid::Uuid>,
    ) -> io::Result<()> {
        let PreparedSwitch {
            review,
            state,
            current,
            saved,
            handheld_preset,
            package_sources,
        } = prepared;
        if let Some(error) = review.blocker {
            return Err(invalid(&error));
        }
        if !package_sources.is_empty() {
            return Err(invalid(
                "Install the required addons and review the profile again.",
            ));
        }
        let target = review.target;
        if target == state.active && !review.preset && !review.editing_addons {
            return Ok(());
        }
        check()?;
        let bytes = serde_json::to_vec(&current)?;
        if bytes.len() > MAX_SNAPSHOT {
            return Err(invalid("This setup exceeds the settings backup limit."));
        }
        replace(&self.backups.join(state.active.file()), &bytes)?;
        let finish = || {
            let mut names = state.names.clone();
            if let Some(name) = review.cloud_name {
                if let Some((_, previous)) =
                    names.iter_mut().find(|(profile, _)| *profile == target)
                {
                    *previous = name;
                } else {
                    names.push((target, name));
                }
            }
            preferences::save_json(
                &self.profile_state_path(),
                &ProfileState {
                    names,
                    active: target,
                    interrupted: false,
                    sync_pending: state.sync_pending || target != state.active,
                    handheld_preset,
                    committed_package_switch: package_switch.or(state.committed_package_switch),
                    ..state.clone()
                },
            )
        };
        if let Some(saved) = saved {
            self.apply(
                saved,
                &current.files,
                catalog,
                check,
                || {
                    preferences::save_json(
                        &self.profile_state_path(),
                        &ProfileState {
                            interrupted: true,
                            ..state.clone()
                        },
                    )
                },
                finish,
            )?;
        } else {
            // A new slot starts from the live setup. Its first save happens when leaving it.
            finish()?;
        }
        Ok(())
    }

    fn addon_toc(&self, folder: &str) -> io::Result<Option<PathBuf>> {
        for toc in crate::library::toc_names(folder, self.client) {
            let path = child(&self.root, &format!("Interface/AddOns/{folder}/{toc}"))?;
            if path.is_file() {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn handheld_requirements(
        &self,
        catalog: &[Definition],
    ) -> io::Result<(Vec<usize>, Vec<&'static str>)> {
        let mut packages = Vec::new();
        let mut manual = Vec::new();
        let groups: &[(&str, &str, &[&str])] = if self.client == Client::Retail {
            &[
                ("ellesmere", "EllesmereUI", &["EllesmereUI"]),
                ("profiles", "Astral Profiles", &["AstralProfiles"]),
                (
                    "consoleport",
                    "ConsolePort (including its Action Bar module)",
                    &["ConsolePort", "ConsolePort_Bar"],
                ),
            ]
        } else {
            &[
                ("ellesmere", "EllesmereUI", &["EllesmereUI"]),
                ("profiles", "Astral Profiles", &["AstralProfiles"]),
            ]
        };
        let addons = child(&self.root, "Interface/AddOns")?;
        for &(id, name, folders) in groups {
            let present = folders
                .iter()
                .map(|folder| self.addon_toc(folder).map(|path| path.is_some()))
                .collect::<io::Result<Vec<_>>>()?;
            if present.iter().all(|found| *found) {
                continue;
            }
            let source = catalog.iter().enumerate().find(|(_, addon)| {
                addon.id == id
                    && addon.supports(self.client)
                    && (addon.bundled == Some(true)
                        || (addon.repo.is_some() && addon.asset.is_some()))
            });
            if let Some((index, source)) = source
                && addons.is_dir()
                && !addons.join(&source.folder).exists()
                && !present.contains(&true)
            {
                packages.push(index);
            } else {
                manual.push(name);
            }
        }
        Ok((packages, manual))
    }

    fn apply_profile_policy(
        &self,
        target: Profile,
        snapshot: &mut Snapshot,
        current: &Snapshot,
        catalog: &[Definition],
    ) -> io::Result<()> {
        let handheld = target == Profile::Handheld;
        let retail = self.client == Client::Retail;
        let mut missing = Vec::new();
        let bridge = self.addon_toc("AstralProfiles")?;
        if handheld {
            if self.addon_toc("EllesmereUI")?.is_none() {
                missing.push("EllesmereUI");
            }
            if bridge.is_none() {
                missing.push("Astral Profiles");
            }
            if retail
                && (self.addon_toc("ConsolePort")?.is_none()
                    || self.addon_toc("ConsolePort_Bar")?.is_none())
            {
                missing.push("ConsolePort (including its Action Bar module)");
            }
        }
        if !missing.is_empty() {
            return Err(invalid(&format!(
                "Install {} from Discover, then try again.",
                missing.join(", ")
            )));
        }

        if handheld && let Some(path) = &bridge {
            let text = String::from_utf8_lossy(&read_bounded(path, 128 * 1024)?).into_owned();
            let toc = crate::library::toc_metadata(&text);
            let version = toc.get("Version").map(String::as_str).unwrap_or_default();
            let compatible = catalog
                .iter()
                .find(|definition| definition.id == "profiles")
                .and_then(|definition| crate::releases::compare(definition, version, "1.2.0"))
                .is_some_and(|ordering| !ordering.is_gt());
            if !compatible {
                return Err(invalid(
                    "Update Astral Profiles to 1.2.0 or later from the Library, then try again.",
                ));
            }
        }
        let mut choices = std::collections::BTreeMap::new();
        if retail {
            let consoleport = catalog
                .iter()
                .find(|definition| definition.id == "consoleport")
                .ok_or_else(|| invalid("ConsolePort is missing from the addon catalog."))?;
            for module in consoleport.module_names() {
                choices.insert(module, handheld);
            }
            if handheld {
                choices.insert("AstralHandheldHUD", false);
            }
        }
        if handheld {
            choices.insert("EllesmereUI", true);
            snapshot
                .controller
                .get_or_insert_with(|| current.controller.clone().unwrap_or_default())
                .enable_controller();
        }
        if bridge.is_some() {
            choices.insert("AstralProfiles", true);
            let mode = if handheld { "handheld" } else { "regular" };
            edit_snapshot_file(snapshot, "SavedVariables/AstralProfiles.lua", |old| {
                let mode_line = regex::bytes::Regex::new(
                    r#"^[ \t]*AstralProfileMode[ \t]*=[ \t]*(?:"(?:regular|handheld)"|'(?:regular|handheld)'|nil)[ \t\r]*$"#,
                ).expect("valid profile mode pattern");
                let mut text = retained_lines(old, |line| !mode_line.is_match(line));
                append_line(&mut text, &format!("AstralProfileMode = \"{mode}\""));
                text
            })?;
        }
        let mut scopes = std::collections::BTreeMap::from([(
            "addons.txt".to_string(),
            "AddOns.txt".to_string(),
        )]);
        for entry in current.files.iter().chain(&snapshot.files) {
            if entry
                .path
                .rsplit('/')
                .next()
                .is_some_and(|file| file.eq_ignore_ascii_case("AddOns.txt"))
            {
                scopes.insert(entry.path.to_lowercase(), entry.path.clone());
            }
        }
        for path in scopes.into_values() {
            edit_snapshot_file(snapshot, &path, |old| {
                let mut text = retained_lines(old, |line| {
                    let Some((name, _)) = std::str::from_utf8(line)
                        .ok()
                        .and_then(|line| line.split_once(':'))
                    else {
                        return true;
                    };
                    !choices
                        .keys()
                        .any(|folder| folder.eq_ignore_ascii_case(name.trim()))
                });
                for (folder, enabled) in &choices {
                    let state = if *enabled { "enabled" } else { "disabled" };
                    append_line(&mut text, &format!("{folder}: {state}"));
                }
                text
            })?;
        }
        Ok(())
    }

    pub fn recover_profile(&self, catalog: &[Definition]) -> io::Result<()> {
        self.recover_checked(catalog, check_game)
    }

    pub(crate) fn recover_package_switch(
        &self,
        backup: &Path,
        operation: &installer::ProfileSwitch,
        catalog: &[Definition],
    ) -> io::Result<()> {
        self.recover_package_switch_checked(backup, operation, catalog, check_game)
    }

    fn recover_package_switch_checked(
        &self,
        backup: &Path,
        operation: &installer::ProfileSwitch,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        installer::recover_with(
            &self.root,
            self.client,
            backup,
            || check().map_err(|error| error.to_string()),
            |record| {
                if record.id != operation.id
                    || record.account != operation.account
                    || self.account.file_name().and_then(|name| name.to_str())
                        != Some(record.account.as_str())
                {
                    return Err("This recovery belongs to a different profile operation.".into());
                }
                let state = self.profile_state().map_err(|error| error.to_string())?;
                if state.committed_package_switch == Some(record.id) {
                    if state.interrupted {
                        return Err("Recover the interrupted profile in Settings first.".into());
                    }
                    return Ok(true);
                }
                self.recover_settings_locked(catalog, &check)
                    .map_err(|error| error.to_string())?;
                Ok(false)
            },
        )
        .map_err(io::Error::other)
    }

    fn recover_checked(
        &self,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        check()?;
        self.recover_settings_locked(catalog, check)
    }

    fn recover_settings_locked(
        &self,
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        let state = self.profile_state()?;
        if !state.interrupted {
            return Ok(());
        }
        let saved = Self::read_snapshot(&self.backups.join(state.active.file()))?;
        let current = self.snapshot(catalog)?;
        self.apply(
            saved,
            &current.files,
            catalog,
            check,
            || Ok(()),
            || {
                preferences::save_json(
                    &self.profile_state_path(),
                    &ProfileState {
                        interrupted: false,
                        ..state
                    },
                )
            },
        )?;
        Ok(())
    }

    fn apply(
        &self,
        snapshot: Snapshot,
        current: &[Entry],
        catalog: &[Definition],
        check: impl Fn() -> io::Result<()>,
        begin: impl FnOnce() -> io::Result<()>,
        commit: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<usize> {
        let mut seen = HashSet::new();
        let mut changes = Vec::new();
        let mut undo = Vec::new();
        for entry in snapshot.files {
            if !allowed(&entry.path, catalog) || !seen.insert(entry.path.to_lowercase()) {
                return Err(invalid("Invalid or duplicate settings path."));
            }
            let content = entry.decode()?;
            let dest = child(&self.account, &entry.path)?;
            let previous = match read_bounded(&dest, MAX_FILE) {
                Ok(bytes) if bytes == content => continue,
                Ok(bytes) => {
                    undo.push(Entry::new(entry.path, &bytes));
                    Some(bytes)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            changes.push((dest, Some(content), previous));
        }
        for entry in current {
            if !seen.contains(&entry.path.to_lowercase()) {
                let dest = child(&self.account, &entry.path)?;
                let previous = read_bounded(&dest, MAX_FILE)?;
                undo.push(Entry::new(entry.path.clone(), &previous));
                changes.push((dest, None, Some(previous)));
            }
        }
        let mut undo_controller = None;
        if let Some(controller) = snapshot.controller {
            let (config, previous) = self.read_config()?;
            let current =
                gamepad_settings::Settings::capture(previous.as_deref().unwrap_or_default())?;
            if let Some(content) =
                controller.render(previous.as_deref().unwrap_or_default(), &current)?
            {
                undo_controller = Some(current);
                changes.push((config, Some(content), previous));
            }
        }
        if !undo.is_empty() || undo_controller.is_some() {
            self.save(Snapshot {
                schema: 1,
                files: undo,
                controller: undo_controller,
            })?;
        }
        check()?;
        begin()?;
        let mut applied = 0;
        let result = (|| {
            for (dest, content, _) in &changes {
                check()?;
                match content {
                    Some(bytes) => replace(dest, bytes)?,
                    None => fs::remove_file(dest)?,
                }
                applied += 1;
            }
            check()?;
            commit()
        })();
        if let Err(error) = result {
            let mut rollback_failed = false;
            for (dest, _, previous) in changes[..applied].iter().rev() {
                let reverted = match previous {
                    Some(bytes) => replace(dest, bytes),
                    None => fs::remove_file(dest),
                };
                rollback_failed |= reverted.is_err();
            }
            return Err(if rollback_failed {
                io::Error::other(format!(
                    "{error} Some files could not be rolled back. Check the newest settings backup."
                ))
            } else {
                error
            });
        }
        Ok(applied)
    }
}

fn edit_snapshot_file(
    snapshot: &mut Snapshot,
    path: &str,
    edit: impl FnOnce(&[u8]) -> Vec<u8>,
) -> io::Result<()> {
    if let Some(entry) = snapshot.files.iter_mut().find(|entry| entry.path == path) {
        let data = edit(&entry.decode()?);
        *entry = Entry::new(path.to_owned(), &data);
    } else {
        snapshot.files.push(Entry::new(path.to_owned(), &edit(&[])));
    }
    Ok(())
}

fn retained_lines(text: &[u8], keep: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let mut output = Vec::with_capacity(text.len());
    for line in text.split_inclusive(|&byte| byte == b'\n') {
        if keep(line.strip_suffix(b"\n").unwrap_or(line)) {
            output.extend_from_slice(line);
        }
    }
    output
}

fn append_line(text: &mut Vec<u8>, line: &str) {
    if !text.is_empty() && !text.ends_with(b"\n") {
        text.push(b'\n');
    }
    text.extend_from_slice(line.as_bytes());
    text.push(b'\n');
}

fn check_game() -> io::Result<()> {
    installer::game_closed().map_err(|error| {
        io::Error::other(error.replace("changing addon files", "backing up or restoring settings"))
    })
}

fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Invalid settings path."))?;
    fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if link(&metadata) || !metadata.is_file() {
            return Err(invalid("The settings file is blocked."));
        }
        staged.as_file().set_permissions(metadata.permissions())?;
    }
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged.persist(path)?;
    Ok(())
}

fn backup_time(id: &str) -> Option<u64> {
    let (time, suffix) = id.strip_suffix(".json")?.split_once('-')?;
    if time.len() != 13 || suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    time.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn electron_and_native_exchange_settings_on_both_clients() -> io::Result<()> {
        use std::process::{Command, Stdio};
        let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") else {
            return Ok(());
        };
        let temp = tempfile::tempdir()?;
        let catalog = crate::library::catalog();
        for client in Client::ALL {
            let client_dir = temp.path().join(client.folder());
            let setup =
                |device: &str, account: &str, binding: &str, graphics: u8| -> io::Result<_> {
                    let data = client_dir.join(device);
                    let root = data.join(client.folder());
                    let account_path = root.join("WTF/Account").join(account);
                    let addons = root.join("Interface/AddOns/EllesmereUI");
                    fs::create_dir_all(account_path.join("Realm/Character"))?;
                    fs::create_dir_all(&addons)?;
                    fs::write(addons.join("EllesmereUI.toc"), "## Version: 1\n")?;
                    fs::write(account_path.join("bindings-cache.wtf"), binding)?;
                    fs::write(account_path.join("AddOns.txt"), "EllesmereUI: enabled\n")?;
                    fs::write(
                        account_path.join("Realm/Character/AddOns.txt"),
                        "EllesmereUI: disabled\n",
                    )?;
                    fs::write(
                        root.join("WTF/Config.wtf"),
                        format!(
                            "SET graphicsQuality \"{graphics}\"\nSET GamePadEnable \"{}\"\n",
                            u8::from(device == "rust")
                        ),
                    )?;
                    Ok((data, root, account_path))
                };
            let (rust_data, rust_root, rust_account) =
                setup("rust", "RustAccount", "BIND 1 ACTIONBUTTON1\n", 9)?;
            let (electron_data, electron_root, electron_account) =
                setup("electron", "ElectronAccount", "BIND 2 ACTIONBUTTON2\n", 5)?;
            let store = Store::open(&rust_data, &rust_root, client, "RustAccount")?;
            let (_, snapshot) = store.cloud_bundle_checked(&catalog, || Ok(()))?;
            let encoded = snapshot.encode(&catalog)?;
            let mut process = Command::new(&node)
                .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
                .args(["-e", r#"
                    const assert = require('node:assert/strict');
                    const fs = require('node:fs/promises');
                    const path = require('node:path');
                    const settings = require('./desktop/settings.cjs');
                    (async () => {
                      const [root, data] = process.argv.slice(1);
                      const chunks = [];
                      for await (const chunk of process.stdin) chunks.push(chunk);
                      const {payload, hash} = JSON.parse(Buffer.concat(chunks).toString('utf8'));
                      assert.equal(settings.inspect(payload).hash, hash);
                      const account = 'ElectronAccount';
                      const base = await settings.accountRoot(root, account);
                      await settings.apply(root, account, payload, data, async () => false);
                      assert.equal(await fs.readFile(path.join(base, 'bindings-cache.wtf'), 'utf8'), 'BIND 1 ACTIONBUTTON1\n');
                      assert.match(await fs.readFile(path.join(base, 'Realm/Character/AddOns.txt'), 'utf8'), /EllesmereUI: disabled/);
                      const config = path.join(root, 'WTF/Config.wtf');
                      const applied = await fs.readFile(config, 'utf8');
                      assert.match(applied, /graphicsQuality "5"/);
                      assert.match(applied, /GamePadEnable "1"/);
                      await fs.writeFile(config, applied.replace('GamePadEnable "1"', 'GamePadEnable "0"'));
                      await fs.writeFile(path.join(base, 'bindings-cache.wtf'), 'BIND 3 ACTIONBUTTON3\n');
                      await fs.writeFile(path.join(base, 'Realm/Character/AddOns.txt'), 'EllesmereUI: enabled\n');
                      process.stdout.write(JSON.stringify(await settings.capture(root, account, data)));
                    })().catch(error => { console.error(error); process.exitCode = 1; });
                "#])
                .arg(&electron_root)
                .arg(&electron_data)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?;
            serde_json::to_writer(process.stdin.take().unwrap(), &encoded)?;
            let output = process.wait_with_output()?;
            assert!(
                output.status.success(),
                "{} Electron handoff failed",
                client.label()
            );
            let incoming: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            let bundle = crate::sync_format::Bundle::decode(
                incoming["payload"].as_str().unwrap(),
                Some(incoming["hash"].as_str().unwrap()),
                &catalog,
            )?;
            store.apply_cloud_bundle_checked(bundle, &catalog, || Ok(()), || Ok(()))?;
            assert_eq!(
                fs::read_to_string(rust_account.join("bindings-cache.wtf"))?,
                "BIND 3 ACTIONBUTTON3\n"
            );
            assert!(
                fs::read_to_string(rust_account.join("Realm/Character/AddOns.txt"))?
                    .contains("EllesmereUI: enabled")
            );
            assert!(
                fs::read_to_string(rust_root.join("WTF/Config.wtf"))?
                    .contains("graphicsQuality \"9\"")
            );
            assert!(
                fs::read_to_string(rust_root.join("WTF/Config.wtf"))?
                    .contains("GamePadEnable \"0\"")
            );
            assert!(
                fs::read_to_string(electron_account.join("bindings-cache.wtf"))?
                    .contains("ACTIONBUTTON3")
            );
            assert_eq!(store.list()?.len(), 1);
            assert_eq!(
                fs::read_dir(electron_data.join("settings-backups"))?.count(),
                1
            );
        }
        Ok(())
    }

    #[test]
    fn newly_shared_addons_are_disabled_without_changing_existing_profile_choices() -> io::Result<()>
    {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        let character = account.join("Realm/Character");
        fs::create_dir_all(&character)?;
        let addons = root.join("Interface/AddOns");
        fs::create_dir_all(addons.join("EllesmereUI"))?;
        fs::write(
            addons.join("EllesmereUI/EllesmereUI.toc"),
            "## Version: 1\n",
        )?;
        fs::write(account.join("AddOns.txt"), "EllesmereUI: enabled\n")?;
        fs::write(character.join("AddOns.txt"), "EllesmereUI: disabled\n")?;
        let inventory = crate::sync_addons::Inventory::open(&root, &account, Client::Retail, b"")?;
        let original = inventory.selection()?;
        fs::create_dir_all(addons.join("AstralProfiles"))?;
        fs::write(
            addons.join("AstralProfiles/AstralProfiles.toc"),
            "## Dependencies: EllesmereUI\n",
        )?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        store.preserve_addon_choices_checked(&original, &crate::library::catalog(), || Ok(()))?;
        let account_choices = fs::read_to_string(account.join("AddOns.txt"))?;
        let character_choices = fs::read_to_string(character.join("AddOns.txt"))?;
        assert!(account_choices.contains("EllesmereUI: enabled"));
        assert!(character_choices.contains("EllesmereUI: disabled"));
        assert!(account_choices.contains("AstralProfiles: disabled"));
        assert!(character_choices.contains("AstralProfiles: disabled"));
        Ok(())
    }

    #[test]
    fn electron_snapshot_restores_character_addons_and_controller_without_touching_graphics()
    -> io::Result<()> {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/electron-snapshot.json"))?;
        let payload = fixture["payload"].as_str().unwrap();
        let hash = fixture["hash"].as_str().unwrap();
        let catalog = crate::library::catalog();
        assert!(crate::sync_format::Bundle::decode(payload, Some("wrong"), &catalog).is_err());
        let bundle = crate::sync_format::Bundle::decode(payload, Some(hash), &catalog)?;
        let encoded = bundle.encode(&catalog)?;
        if let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") {
            use std::process::{Command, Stdio};
            let mut process = Command::new(node)
                .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
                .args(["-e", r#"
                    const assert = require('node:assert/strict');
                    const fs = require('node:fs');
                    const settings = require('./desktop/settings.cjs');
                    const snapshot = JSON.parse(fs.readFileSync(0, 'utf8'));
                    assert.equal(settings.inspect(snapshot.payload).hash, snapshot.hash);
                    const bundle = settings.decode(snapshot.payload);
                    assert.equal(bundle.input.GamePadEnable, '1');
                    assert.equal(bundle.addonSelection.scopes['Test Realm/Brânch'].ConsolePort, true);
                "#])
                .stdin(Stdio::piped())
                .spawn()?;
            serde_json::to_writer(process.stdin.take().unwrap(), &encoded)?;
            assert!(
                process.wait()?.success(),
                "Electron must decode the Rust snapshot"
            );
        }
        let restored =
            crate::sync_format::Bundle::decode(&encoded.payload, Some(&encoded.hash), &catalog)?;
        assert_eq!(restored.normalized_hash()?, bundle.normalized_hash()?);

        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        let character = account.join("Test Realm/Brânch");
        fs::create_dir_all(&character)?;
        for folder in ["ConsolePort", "EllesmereUI"] {
            let path = root.join("Interface/AddOns").join(folder);
            fs::create_dir_all(&path)?;
            fs::write(path.join(format!("{folder}.toc")), "## Version: 1.0\n")?;
        }
        let config = root.join("WTF/Config.wtf");
        fs::write(
            &config,
            "SET graphicsQuality \"9\"\nSET GamePadEnable \"0\"\n",
        )?;
        fs::write(character.join("bindings-cache.wtf"), "local bindings")?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let rejected = store.apply_cloud_bundle_checked(
            restored.clone(),
            &catalog,
            || Ok(()),
            || Err(io::Error::other("The cloud backup changed.")),
        );
        assert_eq!(
            rejected.unwrap_err().to_string(),
            "The cloud backup changed."
        );
        assert_eq!(
            fs::read_to_string(character.join("bindings-cache.wtf"))?,
            "local bindings"
        );
        assert_eq!(
            fs::read_to_string(&config)?,
            "SET graphicsQuality \"9\"\nSET GamePadEnable \"0\"\n"
        );
        assert!(!account.join("AddOns.txt").exists());
        assert!(!character.join("AddOns.txt").exists());
        store.apply_cloud_bundle_checked(restored, &catalog, || Ok(()), || Ok(()))?;
        assert_eq!(
            fs::read_to_string(character.join("bindings-cache.wtf"))?,
            "BIND 1 ACTIONBUTTON1\n"
        );
        assert!(fs::read_to_string(account.join("AddOns.txt"))?.contains("ConsolePort: disabled"));
        assert!(fs::read_to_string(character.join("AddOns.txt"))?.contains("ConsolePort: enabled"));
        assert_eq!(
            fs::read_to_string(&config)?,
            "SET graphicsQuality \"9\"\nSET GamePadEnable \"1\"\nSET SoftTargetEnemy \"3\"\n"
        );
        assert_eq!(store.list()?.len(), 2);
        Ok(())
    }

    #[test]
    fn custom_profile_keeps_its_setup_when_renamed_and_is_archived_on_delete() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        fs::create_dir_all(&account)?;
        let bindings = account.join("bindings-cache.wtf");
        fs::write(&bindings, "desktop")?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let catalog = crate::library::catalog();
        let custom = store.save_profile_name(None, " Raid ")?;
        assert!(store.save_profile_name(None, "raid").is_err());
        assert_eq!(store.profile_state()?.label(custom), "Raid");
        store.switch_checked(custom, &catalog, || Ok(()))?;
        fs::write(&bindings, "raid bindings")?;
        assert!(store.delete_profile(custom).is_err());
        store.save_profile_name(Some(custom), "Mythic Raid")?;
        store.save_profile_name(Some(Profile::Regular), "Keyboard")?;
        store.switch_checked(Profile::Regular, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&bindings)?, "desktop");
        let reopened = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        assert_eq!(reopened.profile_state()?.label(custom), "Mythic Raid");
        assert_eq!(
            reopened.profile_state()?.label(Profile::Regular),
            "Keyboard"
        );
        let before: HashSet<_> = reopened
            .list()?
            .into_iter()
            .map(|backup| backup.id)
            .collect();
        reopened.delete_profile(custom)?;
        assert!(!reopened.profile_state()?.profiles().any(|id| id == custom));
        assert!(
            reopened
                .switch_checked(custom, &catalog, || Ok(()))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&bindings)?, "desktop");
        let archived = reopened
            .list()?
            .into_iter()
            .find(|backup| !before.contains(&backup.id))
            .unwrap();
        reopened.restore_checked(&archived.id, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&bindings)?, "raid bindings");
        Ok(())
    }

    #[test]
    fn addon_choices_review_keeps_dependencies_and_other_profiles_intact() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        let character = account.join("Realm/Druid");
        fs::create_dir_all(&character)?;
        for (addon, toc) in [
            ("ExampleCore", "## Title: Example Core\n"),
            ("ExampleChild", "## Dependencies: ExampleCore\n"),
        ] {
            let folder = root.join("Interface/AddOns").join(addon);
            fs::create_dir_all(&folder)?;
            fs::write(folder.join(format!("{addon}.toc")), toc)?;
        }
        let choices = character.join("AddOns.txt");
        let original = "ExampleCore: enabled\nExampleChild: enabled\nBlizzard_Test: disabled\n";
        fs::write(&choices, original)?;
        let catalog = crate::library::catalog();
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let custom = store.save_profile_name(None, "Raid")?;
        store.switch_checked(custom, &catalog, || Ok(()))?;
        let blocked = store
            .prepare_profile(
                custom,
                false,
                &catalog,
                ProfileEdits::from([("ExampleCore".into(), false)]),
            )?
            .review;
        assert!(
            blocked
                .blocker
                .as_ref()
                .is_some_and(|error| error.contains("requires"))
        );
        let edits = ProfileEdits::from([
            ("ExampleCore".into(), false),
            ("ExampleChild".into(), false),
        ]);
        let review = store
            .prepare_profile(custom, false, &catalog, edits)?
            .review;
        assert_eq!(review.addon_changes.len(), 2);
        assert!(review.changed.is_empty());
        assert_eq!(fs::read_to_string(&choices)?, original);
        store.apply_review_checked(&review, &catalog, || Ok(()))?;
        let updated = fs::read_to_string(&choices)?;
        assert!(
            updated.contains("ExampleCore: disabled")
                && updated.contains("Blizzard_Test: disabled")
        );
        store.switch_checked(Profile::Regular, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&choices)?, original);
        store.switch_checked(custom, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&choices)?, updated);
        Ok(())
    }

    #[test]
    fn profile_review_shows_file_changes_and_rejects_a_stale_confirmation() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        fs::create_dir_all(&account)?;
        let bindings = account.join("bindings-cache.wtf");
        fs::write(&bindings, "desktop")?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let catalog = crate::library::catalog();
        let first = store
            .prepare_switch(Profile::Handheld, false, &catalog)?
            .review;
        assert!(first.initialized);
        assert!(first.changed.is_empty());
        store.apply_review_checked(&first, &catalog, || Ok(()))?;
        fs::write(&bindings, "handheld")?;
        let review = store
            .prepare_switch(Profile::Regular, false, &catalog)?
            .review;
        assert!(!review.initialized);
        assert_eq!(review.changed, ["bindings-cache.wtf"]);
        fs::write(&bindings, "revised handheld")?;
        assert!(
            store
                .apply_review_checked(&review, &catalog, || Ok(()))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&bindings)?, "revised handheld");
        assert_eq!(store.profile_state()?.active, Profile::Handheld);
        let review = store
            .prepare_switch(Profile::Regular, false, &catalog)?
            .review;
        store.apply_review_checked(&review, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&bindings)?, "desktop");
        let blocked = store
            .prepare_switch(Profile::Handheld, true, &catalog)?
            .review;
        assert!(
            blocked
                .blocker
                .as_ref()
                .is_some_and(|error| error.contains("ConsolePort"))
        );
        assert!(
            store
                .apply_review_checked(&blocked, &catalog, || Ok(()))
                .is_err()
        );
        assert_eq!(store.profile_state()?.active, Profile::Regular);
        Ok(())
    }

    #[test]
    fn profile_review_rejects_another_account_or_an_expired_preview() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        for name in ["AccountOne", "AccountTwo"] {
            let account = root.join("WTF/Account").join(name);
            fs::create_dir_all(&account)?;
            fs::write(account.join("bindings-cache.wtf"), "same bindings")?;
        }
        let first = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let second = Store::open(temp.path(), &root, Client::Retail, "AccountTwo")?;
        let mut review = first.prepare_switch(Profile::Handheld, false, &[])?.review;
        let error = second
            .apply_review_checked(&review, &[], || Ok(()))
            .unwrap_err();
        assert!(error.to_string().contains("Your setup changed"));
        review.expires_at = Instant::now();
        let error = first
            .apply_review_checked(&review, &[], || Ok(()))
            .unwrap_err();
        assert!(error.to_string().contains("review expired"));
        for store in [first, second] {
            assert_eq!(store.profile_state()?.active, Profile::Regular);
            assert!(store.list()?.is_empty());
        }
        Ok(())
    }

    #[test]
    fn cloud_profile_is_created_locally_only_after_the_review_is_applied() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_forever_");
        let account = root.join("WTF/Account/AccountOne");
        fs::create_dir_all(&account)?;
        let bindings = account.join("bindings-cache.wtf");
        fs::write(&bindings, "regular")?;
        let store = Store::open(temp.path(), &root, Client::Forever, "AccountOne")?;
        let catalog = crate::library::catalog();
        let target = Profile::from_cloud_id("1234567890ab4cde8f0123456789abcd").unwrap();
        let review = || {
            store
                .prepare_named_profile(target, false, &catalog, ProfileEdits::new(), Some("Raid"))
                .map(|plan| plan.review)
        };
        let first = review()?;
        assert!(first.initialized);
        assert!(!store.profile_state()?.profiles().any(|id| id == target));
        fs::write(&bindings, "updated regular")?;
        assert!(
            store
                .apply_review_checked(&first, &catalog, || Ok(()))
                .is_err()
        );
        assert!(!store.profile_state()?.profiles().any(|id| id == target));
        store.apply_review_checked(&review()?, &catalog, || Ok(()))?;
        let state = store.profile_state()?;
        assert_eq!(state.active, target);
        assert_eq!(state.label(target), "Raid");
        assert_eq!(fs::read_to_string(bindings)?, "updated regular");
        Ok(())
    }

    #[test]
    fn cloud_profile_reuses_its_local_settings_after_rename() -> io::Result<()> {
        let id = "1234567890ab4cde8f0123456789abcd";
        let profile = Profile::from_cloud_id(id).unwrap();
        assert_eq!(profile.cloud_id(), id);
        assert!(Profile::from_cloud_id("../retail").is_none());
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        fs::create_dir_all(&account)?;
        let bindings = account.join("bindings-cache.wtf");
        fs::write(&bindings, "regular")?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let catalog = crate::library::catalog();
        store.import_cloud_profile(profile, "Ally Raid")?;
        store.switch_checked(profile, &catalog, || Ok(()))?;
        fs::write(&bindings, "ally raid")?;
        store.switch_checked(Profile::Regular, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&bindings)?, "regular");
        store.import_cloud_profile(profile, "Raid")?;
        store.switch_checked(profile, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&bindings)?, "ally raid");
        assert_eq!(store.profile_state()?.label(profile), "Raid");
        assert_eq!(store.profile_state()?.profiles().count(), 3);
        Ok(())
    }

    #[test]
    fn reviewed_handheld_requirements_install_and_switch_in_one_operation() -> io::Result<()> {
        for client in Client::ALL {
            let temp = tempfile::tempdir()?;
            let root = temp.path().join(if client == Client::Retail {
                "_retail_"
            } else {
                "_forever_"
            });
            let account = root.join("WTF/Account/AccountOne");
            let addons = root.join("Interface/AddOns");
            fs::create_dir_all(&account)?;
            fs::create_dir_all(&addons)?;
            let bindings = account.join("bindings-cache.wtf");
            fs::write(&bindings, "desktop")?;
            let store = Store::open(temp.path(), &root, client, "AccountOne")?;
            let catalog = crate::library::catalog();
            let prepare = || {
                store
                    .prepare_switch(Profile::Handheld, true, &catalog)
                    .map(|plan| plan.review)
            };
            let initial = prepare()?;
            let expected = if client == Client::Retail {
                vec!["EllesmereUI", "Astral Profiles", "ConsolePort"]
            } else {
                vec!["EllesmereUI", "Astral Profiles"]
            };
            assert_eq!(initial.packages, expected);
            assert!(initial.blocker.is_none());
            assert!(
                store
                    .apply_review_checked(&initial, &catalog, || Ok(()))
                    .is_err()
            );
            fs::write(&bindings, "new desktop bindings")?;
            assert!(
                store
                    .apply_review_with(
                        &initial,
                        &catalog,
                        || Ok(()),
                        |_, _, _| panic!("stale review must not install")
                    )
                    .is_err()
            );
            let mut installed = Vec::new();
            store.apply_review_with(
                &prepare()?,
                &catalog,
                || Ok(()),
                |sources, _, finish| {
                    for &index in sources {
                        let source = &catalog[index];
                        installed.push(source.name.as_str());
                        for folder in source.module_names() {
                            let destination = addons.join(folder);
                            fs::create_dir_all(&destination)?;
                            fs::write(
                                destination.join(format!("{folder}.toc")),
                                "## Version: 1.2.0\n",
                            )?;
                        }
                    }
                    assert_eq!(store.profile_state()?.active, Profile::Regular);
                    assert_eq!(fs::read_to_string(&bindings)?, "new desktop bindings");
                    finish()
                },
            )?;
            assert_eq!(installed, expected);
            assert_eq!(store.profile_state()?.active, Profile::Handheld);
        }
        Ok(())
    }

    #[test]
    fn handheld_preset_uses_consoleport_only_for_retail_and_keeps_regular_settings()
    -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let catalog = crate::library::catalog();
        for client in Client::ALL {
            let root = temp.path().join(if client == Client::Retail {
                "_retail_"
            } else {
                "_forever_"
            });
            let account = root.join("WTF/Account/AccountOne");
            let character = account.join("Realm/Character");
            fs::create_dir_all(&character)?;
            let addons = character.join("AddOns.txt");
            fs::write(&addons, "Example: enabled\nConsolePort: disabled\n")?;
            let config = root.join("WTF/Config.wtf");
            fs::write(
                &config,
                "SET GamePadEnable \"0\"\nSET graphicsQuality \"9\"\n",
            )?;
            let store = Store::open(temp.path(), &root, client, "AccountOne")?;
            assert!(
                store
                    .switch_with_policy(Profile::Handheld, true, &catalog, || Ok(()))
                    .is_err()
            );
            assert_eq!(store.profile_state()?.active, Profile::Regular);
            assert!(!store.profile_state()?.handheld_preset);

            let mut required = vec![("EllesmereUI", "9.8.4"), ("AstralProfiles", "1.2.0")];
            if client == Client::Retail {
                required.extend([("ConsolePort", "3.0.0"), ("ConsolePort_Bar", "3.0.0")]);
            }
            for (name, version) in required {
                let path = root.join("Interface/AddOns").join(name);
                fs::create_dir_all(&path)?;
                fs::write(
                    path.join(format!("{name}.toc")),
                    format!("## Version: {version}\n"),
                )?;
            }
            store.switch_with_policy(Profile::Handheld, true, &catalog, || Ok(()))?;
            let handheld_addons = fs::read_to_string(&addons)?;
            assert!(handheld_addons.contains("Example: enabled"));
            assert!(handheld_addons.contains("AstralProfiles: enabled"));
            assert_eq!(
                handheld_addons.contains("ConsolePort: enabled"),
                client == Client::Retail
            );
            assert!(fs::read_to_string(&config)?.contains("SET GamePadEnable \"1\""));
            let marker = account.join("SavedVariables/AstralProfiles.lua");
            assert_eq!(
                fs::read_to_string(&marker)?,
                "AstralProfileMode = \"handheld\"\n"
            );

            store.switch_checked(Profile::Regular, &catalog, || Ok(()))?;
            assert!(fs::read_to_string(&addons)?.contains("ConsolePort: disabled"));
            assert!(fs::read_to_string(&config)?.contains("SET GamePadEnable \"0\""));
            assert_eq!(
                fs::read_to_string(&marker)?,
                "AstralProfileMode = \"regular\"\n"
            );
            store.switch_checked(Profile::Handheld, &catalog, || Ok(()))?;
            assert!(store.profile_state()?.handheld_preset);
            assert_eq!(
                fs::read_to_string(&marker)?,
                "AstralProfileMode = \"handheld\"\n"
            );
            assert!(fs::read_to_string(&config)?.contains("SET graphicsQuality \"9\""));
        }
        Ok(())
    }

    #[test]
    fn switching_profiles_preserves_edits_and_separates_files_created_in_one_profile()
    -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        let character = account.join("Realm/Character");
        fs::create_dir_all(&character)?;
        let bindings = account.join("bindings-cache.wtf");
        let selection = character.join("AddOns.txt");
        let config = root.join("WTF/Config.wtf");
        fs::write(&bindings, "desktop bindings")?;
        fs::write(
            &config,
            b"\xef\xbb\xbfSET GamePadEnable \"0\"\r\nSET graphicsQuality \"8\"\r\n",
        )?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let catalog = crate::library::catalog();

        store.switch_checked(Profile::Handheld, &catalog, || Ok(()))?;
        assert_eq!(store.profile_state()?.active, Profile::Handheld);
        assert_eq!(fs::read_to_string(&bindings)?, "desktop bindings");
        fs::write(&bindings, "handheld bindings")?;
        fs::write(&selection, "ConsolePort: enabled")?;
        fs::write(&config, b"\xef\xbb\xbfSET GamePadEnable \"1\"\r\nSET SoftTargetEnemy \"3\"\r\nSET graphicsQuality \"9\"\r\n")?;

        store.switch_checked(Profile::Regular, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&bindings)?, "desktop bindings");
        assert!(!selection.exists());
        assert_eq!(
            fs::read(&config)?,
            b"\xef\xbb\xbfSET graphicsQuality \"9\"\r\nSET GamePadEnable \"0\"\r\n"
        );
        fs::write(&bindings, "updated desktop bindings")?;

        store.switch_checked(Profile::Handheld, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&bindings)?, "handheld bindings");
        assert_eq!(fs::read_to_string(&selection)?, "ConsolePort: enabled");
        assert_eq!(fs::read(&config)?, b"\xef\xbb\xbfSET graphicsQuality \"9\"\r\nSET GamePadEnable \"1\"\r\nSET SoftTargetEnemy \"3\"\r\n");

        store.switch_checked(Profile::Regular, &catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&bindings)?, "updated desktop bindings");
        assert!(!selection.exists());
        assert_eq!(
            fs::read(&config)?,
            b"\xef\xbb\xbfSET graphicsQuality \"9\"\r\nSET GamePadEnable \"0\"\r\n"
        );
        assert_eq!(
            Store::open(temp.path(), &root, Client::Forever, "AccountOne")?
                .profile_state()?
                .active,
            Profile::Regular
        );
        Ok(())
    }

    #[test]
    fn package_switch_recovery_rolls_back_before_commit_and_keeps_addons_after_commit()
    -> io::Result<()> {
        for committed in [false, true] {
            let temp = tempfile::tempdir()?;
            let root = temp.path().join("_retail_");
            let account = root.join("WTF/Account/AccountOne");
            let addon = root.join("Interface/AddOns/ConsolePort");
            fs::create_dir_all(&account)?;
            fs::create_dir_all(&addon)?;
            let bindings = account.join("bindings-cache.wtf");
            fs::write(&bindings, "handheld")?;
            let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
            let catalog = crate::library::catalog();
            let operation = installer::ProfileSwitch {
                account: "AccountOne".into(),
                id: uuid::Uuid::new_v4(),
            };
            let backup = store
                .root
                .join("Interface/.astral-native-backups/release-interrupted");
            fs::create_dir_all(backup.join("previous"))?;
            preferences::save_json(
                &backup.join("pending.json"),
                &installer::Transaction {
                    addon: "Handheld profile addons".into(),
                    release: "Profile setup".into(),
                    previous_version: None,
                    notes: String::new(),
                    created: 1,
                    created_ms: None,
                    folders: vec!["ConsolePort".into()],
                    previous: Vec::new(),
                    restored: false,
                    removed: false,
                    profile_switch: Some(operation.clone()),
                },
            )?;
            preferences::save_json(
                &store.backups.join(Profile::Regular.file()),
                &Snapshot {
                    schema: 1,
                    files: vec![Entry::new("bindings-cache.wtf".into(), b"desktop")],
                    controller: None,
                },
            )?;
            preferences::save_json(
                &store.profile_state_path(),
                &ProfileState {
                    active: if committed {
                        Profile::Handheld
                    } else {
                        Profile::Regular
                    },
                    interrupted: !committed,
                    committed_package_switch: committed.then_some(operation.id),
                    ..ProfileState::default()
                },
            )?;
            assert!(installer::ensure_no_pending_profile_packages(&root).is_err());
            store.recover_package_switch_checked(&backup, &operation, &catalog, || Ok(()))?;
            assert!(!store.profile_state()?.interrupted);
            assert_eq!(addon.exists(), committed);
            assert_eq!(backup.join("manifest.json").exists(), committed);
            assert_eq!(
                fs::read_to_string(&bindings)?,
                if committed { "handheld" } else { "desktop" }
            );
            assert!(!backup.join("pending.json").exists());
        }
        Ok(())
    }

    #[test]
    fn recovers_the_outgoing_profile_after_an_interruption() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        fs::create_dir_all(&account)?;
        let bindings = account.join("bindings-cache.wtf");
        fs::write(&bindings, "desktop")?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let catalog = crate::library::catalog();
        store.switch_checked(Profile::Handheld, &catalog, || Ok(()))?;
        fs::write(&bindings, "handheld")?;

        let checks = Cell::new(0);
        let interrupted = store.switch_checked(Profile::Regular, &catalog, || {
            checks.set(checks.get() + 1);
            if checks.get() == 5 {
                Err(io::Error::other("WoW started."))
            } else {
                Ok(())
            }
        });
        assert!(interrupted.is_err());
        assert_eq!(fs::read_to_string(&bindings)?, "handheld");
        assert!(store.profile_state()?.interrupted);

        // Simulate reopening Astral after a crash left one target file in place.
        fs::write(&bindings, "desktop")?;
        let reopened = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        reopened.recover_checked(&catalog, || Ok(()))?;
        assert_eq!(fs::read_to_string(&bindings)?, "handheld");
        assert_eq!(reopened.profile_state()?.active, Profile::Handheld);
        assert!(!reopened.profile_state()?.interrupted);
        Ok(())
    }

    #[test]
    fn restores_bindings_and_cooldowns_for_one_client_and_preserves_the_current_files()
    -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        let character = account.join("Test Realm/Brânch");
        fs::create_dir_all(&character)?;
        let bindings = account.join("bindings-cache.wtf");
        let cooldowns = character.join("cooldownmanager.txt");
        let config = root.join("WTF/Config.wtf");
        fs::write(&bindings, "old bindings")?;
        fs::write(&cooldowns, "old layout")?;
        fs::write(&config, "local graphics")?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let catalog = crate::library::catalog();
        assert_eq!(store.capture_checked(&catalog, || Ok(()))?, 2);
        let backup = store.list()?.remove(0);
        fs::write(&bindings, "new bindings")?;
        fs::write(&cooldowns, "new layout")?;

        let checks = Cell::new(0);
        let interrupted = store.restore_checked(&backup.id, &catalog, || {
            checks.set(checks.get() + 1);
            if checks.get() == 4 {
                Err(io::Error::other("WoW started."))
            } else {
                Ok(())
            }
        });
        assert!(interrupted.is_err());
        assert_eq!(fs::read_to_string(&bindings)?, "new bindings");
        assert_eq!(fs::read_to_string(&cooldowns)?, "new layout");

        assert_eq!(store.restore_checked(&backup.id, &catalog, || Ok(()))?, 2);
        assert_eq!(fs::read_to_string(&bindings)?, "old bindings");
        assert_eq!(fs::read_to_string(&cooldowns)?, "old layout");
        assert_eq!(fs::read_to_string(&config)?, "local graphics");
        let undo = store
            .list()?
            .into_iter()
            .find(|entry| entry.id != backup.id)
            .unwrap();
        assert_eq!(store.restore_checked(&undo.id, &catalog, || Ok(()))?, 2);
        assert_eq!(fs::read_to_string(&bindings)?, "new bindings");
        assert_eq!(fs::read_to_string(&cooldowns)?, "new layout");
        let forever = Store::open(temp.path(), &root, Client::Forever, "AccountOne")?;
        assert!(forever.list()?.is_empty());
        Ok(())
    }

    #[test]
    fn validates_the_whole_backup_before_replacing_any_files() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("_retail_");
        let account = root.join("WTF/Account/AccountOne");
        fs::create_dir_all(&account)?;
        let bindings = account.join("bindings-cache.wtf");
        fs::write(&bindings, "current")?;
        let store = Store::open(temp.path(), &root, Client::Retail, "AccountOne")?;
        let catalog = crate::library::catalog();
        store.save(Snapshot {
            schema: 1,
            files: vec![
                Entry::new("bindings-cache.wtf".into(), b"changed"),
                Entry::new("../Elsewhere/bindings-cache.wtf".into(), b"invalid"),
            ],
            controller: None,
        })?;
        let backup = store.list()?.remove(0);
        assert!(
            store
                .restore_checked(&backup.id, &catalog, || Ok(()))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&bindings)?, "current");
        assert_eq!(store.list()?.len(), 1);
        Ok(())
    }
}
