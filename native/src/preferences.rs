use crate::library::Client;
use serde::{Deserialize, Serialize};
use std::{
    env, fs, io,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ProfileNamesScope {
    pub root: Option<PathBuf>,
    pub account: Option<String>,
}

#[derive(Serialize)]
pub struct ProfileNames {
    pub scope: ProfileNamesScope,
    pub rows: Vec<crate::cloud::CloudProfile>,
}

// localStorage's cached names are optional UI data. A damaged entry must not
// discard unrelated paths, update preferences, or otherwise valid cached rows.
fn deserialize_profile_names<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ProfileNames>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let scope = serde_json::from_value(entry.get("scope")?.clone()).ok()?;
            let rows = entry
                .get("rows")?
                .as_array()?
                .iter()
                .filter_map(|value| {
                    serde_json::from_value::<crate::cloud::CloudProfile>(value.clone()).ok()
                })
                .filter(valid_profile_name)
                .take(8)
                .collect();
            Some(ProfileNames { scope, rows })
        })
        .collect())
}

fn valid_profile_name(row: &crate::cloud::CloudProfile) -> bool {
    row.profile().is_ok() && row.name.encode_utf16().count() <= 40
}

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub client: Client,
    pub light_theme: bool,
    pub folders: [Option<PathBuf>; 2],
    pub local_accounts: [Option<(PathBuf, String)>; 2],
    pub batch_exclusions: [Vec<String>; 2],
    pub daily_updates: bool,
    pub background_updates: bool,
    pub app_update_checks: bool,
    pub start_at_login: bool,
    pub daily_runs: [Option<crate::daily::Run>; 2],
    pub projects: Vec<crate::library::Definition>,
    #[serde(deserialize_with = "deserialize_profile_names")]
    pub profile_names: Vec<ProfileNames>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            client: Client::default(),
            light_theme: false,
            folders: Default::default(),
            local_accounts: Default::default(),
            batch_exclusions: Default::default(),
            daily_updates: false,
            background_updates: false,
            app_update_checks: true,
            start_at_login: false,
            daily_runs: Default::default(),
            projects: Vec::new(),
            profile_names: Vec::new(),
        }
    }
}

pub fn path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    base.map(|base| base.join("Astral Native").join("preferences.json"))
}

impl Preferences {
    pub fn detected_folders(&self, defaults: &[Option<PathBuf>; 2]) -> [Option<PathBuf>; 2] {
        Client::ALL.map(|client| {
            let sibling = Client::ALL
                .into_iter()
                .find(|other| *other != client)
                .and_then(|other| self.folders[other.index()].as_deref())
                .and_then(Path::parent)
                .map(|parent| parent.join(client.folder()))
                .filter(|root| crate::library::validate_root(root, client).is_ok());
            sibling.or_else(|| defaults[client.index()].clone())
        })
    }

    pub fn local_account(&self, client: Client, root: &Path) -> Option<&str> {
        self.local_accounts[client.index()]
            .as_ref()
            .filter(|(folder, _)| folder == root)
            .map(|(_, account)| account.as_str())
    }

    pub fn cached_profile_names(
        &self,
        scope: &ProfileNamesScope,
    ) -> Option<&[crate::cloud::CloudProfile]> {
        self.profile_names
            .iter()
            .rev()
            .find(|entry| &entry.scope == scope)
            .map(|entry| entry.rows.as_slice())
    }

    pub fn remember_profile_names(
        &mut self,
        scope: ProfileNamesScope,
        rows: &[crate::cloud::CloudProfile],
    ) {
        self.profile_names.retain(|entry| entry.scope != scope);
        self.profile_names.push(ProfileNames {
            scope,
            rows: rows
                .iter()
                .filter(|row| valid_profile_name(row))
                .take(8)
                .cloned()
                .collect(),
        });
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        save_json(path, self)
    }
}

pub fn save_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_persist_by_exact_folder_and_account_without_session_data() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("preferences.json");
        let scope = |root: &str, account: &str| ProfileNamesScope {
            root: Some(root.into()),
            account: Some(account.into()),
        };
        let retail = scope("/fixture/_retail_", "One");
        let other_account = scope("/fixture/_retail_", "Two");
        let forever = scope("/fixture/_forever_", "One");
        let mut preferences = Preferences::default();
        for (key, name) in [
            (retail.clone(), "Raid"),
            (other_account.clone(), "Second account"),
            (forever.clone(), "Forever"),
        ] {
            preferences.remember_profile_names(
                key,
                &[crate::cloud::CloudProfile {
                    id: "regular".into(),
                    name: name.into(),
                }],
            );
        }
        preferences.save(&path)?;
        let restored = Preferences::load(&path)?;
        assert_eq!(
            restored.cached_profile_names(&retail).unwrap()[0].name,
            "Raid"
        );
        assert_eq!(
            restored.cached_profile_names(&other_account).unwrap()[0].name,
            "Second account"
        );
        assert_eq!(
            restored.cached_profile_names(&forever).unwrap()[0].name,
            "Forever"
        );
        assert!(
            restored
                .cached_profile_names(&scope("/elsewhere/_retail_", "One"))
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn damaged_profile_name_cache_preserves_preferences_and_uses_browser_validation()
    -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("preferences.json");
        let good = |name: String| serde_json::json!({"id": "regular", "name": name});
        let mut rows = vec![
            serde_json::json!({"id": "invalid", "name": "No"}),
            good("🌙".repeat(21)),
            serde_json::json!({"id": "regular", "name": 123}),
        ];
        rows.extend((0..10).map(|i| good(format!("Name {i}"))));
        let scope = ProfileNamesScope {
            root: None,
            account: None,
        };
        let value = serde_json::json!({"light_theme": true, "daily_updates": true, "profile_names": [
            {"scope": {"root": 4}, "rows": []},
            {"scope": scope, "rows": rows}
        ]});
        fs::write(&path, serde_json::to_vec(&value)?)?;
        let preferences = Preferences::load(&path)?;
        assert!(preferences.light_theme && preferences.daily_updates);
        let names = preferences.cached_profile_names(&scope).unwrap();
        assert_eq!(names.len(), 8);
        assert_eq!(names[0].name, "Name 0");
        assert_eq!(names[7].name, "Name 7");
        fs::write(&path, br#"{"light_theme":true,"profile_names":"damaged"}"#)?;
        let preferences = Preferences::load(&path)?;
        assert!(preferences.light_theme);
        assert!(preferences.profile_names.is_empty());
        Ok(())
    }

    #[test]
    fn older_preview_sync_preferences_do_not_remove_client_paths_or_daily_choices() -> io::Result<()>
    {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("preferences.json");
        fs::write(&path, br#"{ "auto_settings_sync": false, "folders": ["/sample/_retail_", null], "daily_updates": true }"#)?;
        let prefs = Preferences::load(&path)?;
        assert_eq!(prefs.folders[0], Some(PathBuf::from("/sample/_retail_")));
        assert!(prefs.daily_updates);
        prefs.save(&path)?;
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        assert!(saved.get("auto_settings_sync").is_none());
        assert_eq!(saved["folders"][0], "/sample/_retail_");
        Ok(())
    }

    #[test]
    fn discovers_the_other_client_beside_a_custom_folder() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let retail = dir.path().join("_retail_");
        let forever = dir.path().join("_forever_");
        let mut prefs = Preferences::default();
        prefs.folders[Client::Retail.index()] = Some(retail);
        let defaults = [None, Some(PathBuf::from("default/_forever_"))];
        assert_eq!(prefs.detected_folders(&defaults), defaults);
        fs::create_dir_all(forever.join("Interface/AddOns"))?;
        assert_eq!(
            prefs.detected_folders(&defaults)[Client::Forever.index()],
            Some(forever)
        );
        assert!(prefs.folders[Client::Forever.index()].is_none());
        Ok(())
    }

    #[test]
    fn saves_each_client_folder_and_replaces_previous_preferences() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("preferences.json");
        let mut preferences = Preferences::load(&path)?;
        assert!(preferences.client == Client::Retail);
        preferences.folders[Client::Retail.index()] = Some(dir.path().join("_retail_"));
        preferences.local_accounts[Client::Retail.index()] =
            Some((dir.path().join("_retail_"), "AccountOne".into()));
        preferences.batch_exclusions[Client::Retail.index()].push("AstralTalents".into());
        preferences.save(&path)?;
        preferences.client = Client::Forever;
        preferences.light_theme = true;
        preferences.folders[Client::Forever.index()] = Some(dir.path().join("_forever_"));
        preferences.save(&path)?;

        let reopened = Preferences::load(&path)?;
        assert!(reopened.client == Client::Forever);
        assert!(reopened.light_theme);
        assert_eq!(reopened.folders, preferences.folders);
        assert_eq!(
            reopened.local_account(Client::Retail, &dir.path().join("_retail_")),
            Some("AccountOne")
        );
        assert_eq!(
            reopened.local_account(Client::Retail, &dir.path().join("other/_retail_")),
            None
        );
        assert_eq!(
            reopened.local_account(Client::Forever, &dir.path().join("_retail_")),
            None
        );
        assert_eq!(
            reopened.batch_exclusions[Client::Retail.index()],
            ["AstralTalents"]
        );
        assert!(reopened.batch_exclusions[Client::Forever.index()].is_empty());
        Ok(())
    }
}
