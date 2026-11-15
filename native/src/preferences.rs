use crate::library::Client;
use serde::{Deserialize, Serialize};
use std::{
    env, fs, io,
    io::Write,
    path::{Path, PathBuf},
};

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
