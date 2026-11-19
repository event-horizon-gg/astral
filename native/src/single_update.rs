//! The Library's single-addon action updates directly; package review belongs
//! to community source discovery.
use crate::{Astral, Client, Definition, bundled, installer, library, packages, releases};
use gpui::{Context, PathPromptOptions, Window};
use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering as AtomicOrdering},
    },
    time::Duration,
};

pub(crate) struct State {
    pub folder: String,
    progress: Option<Arc<AtomicU8>>,
}

impl State {
    pub fn percentage(&self) -> Option<u8> {
        self.progress
            .as_ref()
            .map(|progress| progress.load(AtomicOrdering::Relaxed))
    }
}

struct Outcome {
    version: String,
    tag: String,
    notes: String,
    changed: bool,
}

fn install_if_newer(
    root: &Path,
    client: Client,
    source: &Definition,
    catalog: &[Definition],
    tag: String,
    notes: String,
    download: impl FnOnce(&mut dyn FnMut(u8)) -> Result<Vec<u8>, String>,
    mut progress: impl FnMut(u8),
) -> Result<Outcome, String> {
    let installed = library::scan(root, client, catalog)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|addon| addon.folder == source.folder)
        .ok_or("The addon is no longer installed. Refresh your Library.")?;
    let comparison = releases::compare(source, &installed.version, &tag)
        .ok_or("Couldn't compare the installed version. Check the addon source before updating.")?;
    if comparison != Ordering::Greater {
        return Ok(Outcome {
            version: installed.version,
            tag,
            notes,
            changed: false,
        });
    }
    let package = packages::inspect(download(&mut progress)?, root, client, source, catalog)?;
    if package.installed_version.as_deref() != Some(installed.version.as_str()) {
        return Err("The installed version changed. Check for updates again.".into());
    }
    progress(90);
    installer::install(&package, &source.name, &tag, &notes)?;
    progress(100);
    let version = library::scan(root, client, catalog)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|addon| addon.folder == source.folder)
        .ok_or("The updated addon could not be found. Refresh your Library.")?
        .version;
    Ok(Outcome {
        version,
        tag,
        notes,
        changed: true,
    })
}

fn update(
    root: &Path,
    client: Client,
    source: &Definition,
    catalog: &[Definition],
    progress: impl FnMut(u8),
) -> Result<Outcome, String> {
    installer::game_closed()?;
    if source.bundled == Some(true) {
        let tag = bundled::version(&source.folder).ok_or("Bundled addon unavailable.")?;
        install_if_newer(
            root,
            client,
            source,
            catalog,
            tag,
            String::new(),
            |_| bundled::package(&source.folder),
            progress,
        )
    } else {
        let release = releases::latest(&releases::client(), source)?;
        install_if_newer(
            root,
            client,
            source,
            catalog,
            release.tag,
            release.notes,
            |progress| releases::download_with_progress(&release.package, progress),
            progress,
        )
    }
}

enum Input {
    Latest,
    Zip(PathBuf),
}

fn import_scope_matches(
    root: &Path,
    client: Client,
    current_root: Option<&Path>,
    current_client: Client,
) -> bool {
    current_root == Some(root) && current_client == client
}

fn import_zip(
    root: &Path,
    client: Client,
    source: &Definition,
    catalog: &[Definition],
    archive: &Path,
    mut progress: impl FnMut(u8),
) -> Result<Outcome, String> {
    installer::game_closed()?;
    if source.bundled == Some(true) || source.local == Some(true) {
        return Err("ZIP updates are available for supported third-party addons.".into());
    }
    if !archive
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        return Err("Choose an addon ZIP downloaded from its official source.".into());
    }
    let installed = library::scan(root, client, catalog)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|addon| addon.folder == source.folder)
        .ok_or("Select an installed addon before importing its update.")?;
    let package = packages::inspect(packages::read_zip(archive)?, root, client, source, catalog)?;
    if package.installed_version.as_deref() != Some(installed.version.as_str()) {
        return Err("The installed version changed. Choose the ZIP again.".into());
    }
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Addon.zip");
    let tag = format!("ZIP: {name}");
    progress(90);
    installer::install(&package, &source.name, &tag, "")?;
    progress(100);
    let version = library::scan(root, client, catalog)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|addon| addon.folder == source.folder)
        .ok_or("The imported addon could not be found. Refresh your Library.")?
        .version;
    Ok(Outcome {
        version,
        tag,
        notes: String::new(),
        changed: true,
    })
}

impl Astral {
    pub(crate) fn update_one_addon(&mut self, id: &str, cx: &mut Context<Self>) {
        self.start_addon_operation(id, Input::Latest, cx);
    }

    pub fn choose_addon_zip(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture || !self.can_start_update_batch() {
            return;
        }
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            return;
        };
        let Some(source) = self.catalog.iter().find(|source| {
            source.id == id
                && source.supports(client)
                && source.bundled != Some(true)
                && source.local != Some(true)
        }) else {
            return;
        };
        let id = source.id.clone();
        let folder = source.folder.clone();
        let previous = window.focused(cx);
        self.single_update = Some(State {
            folder: folder.clone(),
            progress: None,
        });
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(format!("Update {} from ZIP", source.name).into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = match picker.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update_in(cx, |app, window, cx| {
                if !app
                    .single_update
                    .as_ref()
                    .is_some_and(|state| state.folder == folder && state.progress.is_none())
                {
                    return;
                }
                app.single_update = None;
                if let Some(previous) = previous {
                    previous.focus(window);
                }
                match result {
                    Ok(Some(path)) => {
                        if import_scope_matches(
                            &root,
                            client,
                            app.root(app.preferences.client).map(PathBuf::as_path),
                            app.preferences.client,
                        ) {
                            if app.profile_game_running() {
                                app.show_notification(
                                    "Close World of Warcraft before updating addons.",
                                    true,
                                    cx,
                                );
                            } else {
                                app.start_addon_operation(&id, Input::Zip(path), cx);
                            }
                        } else {
                            app.show_notification(
                                "The selected game client changed. Choose the ZIP again.",
                                true,
                                cx,
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(error) => app.show_notification(error, true, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start_addon_operation(&mut self, id: &str, input: Input, cx: &mut Context<Self>) {
        let importing = matches!(&input, Input::Zip(_));
        if self.fixture || !self.can_start_update_batch() {
            return;
        }
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            return;
        };
        let Some(source) = self
            .catalog
            .iter()
            .find(|source| {
                source.id == id
                    && source.supports(client)
                    && if importing {
                        source.bundled != Some(true) && source.local != Some(true)
                    } else {
                        source.bundled == Some(true) || source.repo.is_some()
                    }
            })
            .cloned()
        else {
            return;
        };
        let Some(addon) = self
            .addons
            .iter()
            .find(|addon| addon.folder == source.folder)
        else {
            return;
        };
        let original = addon.version.clone();
        let catalog = self.catalog.clone();
        let preferences = self.preferences_path.clone();
        let folder = source.folder.clone();
        let name = source.name.clone();
        let progress = Arc::new(AtomicU8::new(0));
        self.single_update = Some(State {
            folder: folder.clone(),
            progress: Some(progress.clone()),
        });
        let observed = progress.clone();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let mut last = 0;
            loop {
                executor.timer(Duration::from_millis(50)).await;
                let active = this.update(cx, |app, cx| {
                    let Some(state) = &app.single_update else {
                        return false;
                    };
                    if !state
                        .progress
                        .as_ref()
                        .is_some_and(|progress| Arc::ptr_eq(progress, &observed))
                    {
                        return false;
                    }
                    let percent = state.percentage().unwrap_or(0);
                    if percent != last {
                        last = percent;
                        cx.notify();
                    }
                    true
                });
                if !matches!(active, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        let task = cx.background_executor().spawn(async move {
            let progress = |percent| progress.store(percent, AtomicOrdering::Relaxed);
            let result = match input {
                Input::Latest => update(&root, client, &source, &catalog, progress),
                Input::Zip(path) => import_zip(&root, client, &source, &catalog, &path, progress),
            };
            crate::activity::record(
                preferences.as_deref(),
                crate::activity::Action {
                    root: root.clone(),
                    client,
                    name: source.name.clone(),
                    retry_source: (!importing).then(|| source.id.clone()),
                    from: Some(original.clone()),
                    to: result.as_ref().ok().map(|outcome| {
                        if importing {
                            "Imported ZIP".into()
                        } else {
                            outcome.tag.clone()
                        }
                    }),
                    verb: match &result {
                        Ok(outcome) if outcome.changed => "Updated",
                        Ok(_) => "Already up to date",
                        Err(_) => "Failed",
                    }
                    .into(),
                    error: result.as_ref().err().cloned(),
                    time: 0,
                },
            );
            let outcome = result?;
            let addons = library::scan_library(&root, client, &catalog)
                .map_err(|error| error.to_string())?;
            let message = if importing {
                format!(
                    "{} updated from ZIP. Previous version saved in Activity.",
                    source.name
                )
            } else {
                format!(
                    "{}{}",
                    source.name,
                    if outcome.version == original {
                        " is already up to date."
                    } else {
                        " is installed and backed up."
                    }
                )
            };
            let check = (!importing).then(|| releases::Check {
                comparison: releases::compare(&source, &outcome.version, &outcome.tag),
                tag: outcome.tag,
                notes: outcome.notes,
            });
            Ok::<_, String>((addons, check, message))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.single_update = None;
                match result {
                    Ok((addons, check, message)) => {
                        app.checks.retain(|folder, _| {
                            let previous = app.addons.iter().find(|addon| &addon.folder == folder);
                            let current = addons.iter().find(|addon| &addon.folder == folder);
                            previous.zip(current).is_some_and(|(previous, current)| {
                                previous.version == current.version
                            })
                        });
                        app.addons = addons;
                        if let Some(check) = check {
                            app.checks.insert(folder.clone(), Ok(check));
                        } else {
                            app.checks.remove(&folder);
                        }
                        app.open_addon_details(folder, cx);
                        app.show_notification(message, false, cx);
                    }
                    Err(error) => {
                        app.load_backups(cx);
                        app.show_notification(format!("{name}: {error}"), true, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(
        version: &str,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Vec<Definition>,
        Definition,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("_retail_");
        let folder = root.join("Interface/AddOns/AstralTalents");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(root.join("WTF")).unwrap();
        fs::write(root.join("WTF/config.wtf"), "settings").unwrap();
        fs::write(
            temp.path().join(".build.info"),
            "Product!STRING:0|Active!DEC:1|Version!STRING:0\nwow|1|12.1.5.123",
        )
        .unwrap();
        fs::write(
            folder.join("AstralTalents.toc"),
            format!(
                "## Interface: 120105\n## Title: Astral Talents\n## Version: {version}\nold.lua\n"
            ),
        )
        .unwrap();
        fs::write(folder.join("old.lua"), "-- previous").unwrap();
        let catalog = library::catalog();
        let source = catalog
            .iter()
            .find(|source| source.id == "talents")
            .unwrap()
            .clone();
        (temp, root, catalog, source)
    }

    fn third_party_fixture(
        version: &str,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Vec<Definition>,
        Definition,
    ) {
        let (temp, root, mut catalog, mut source) = fixture(version);
        source.bundled = None;
        source.local = None;
        source.repo = Some("fixture/AstralTalents".into());
        let index = catalog
            .iter()
            .position(|entry| entry.id == source.id)
            .unwrap();
        catalog[index] = source.clone();
        (temp, root, catalog, source)
    }

    #[test]
    fn explicit_zip_import_can_restore_an_older_version_and_keeps_settings_and_backup() {
        let (temp, root, catalog, source) = third_party_fixture("999.0.0");
        let archive = temp.path().join("restore.ZIP");
        fs::write(&archive, bundled::package(&source.folder).unwrap()).unwrap();
        let mut progress = vec![];
        let outcome = import_zip(
            &root,
            Client::Retail,
            &source,
            &catalog,
            &archive,
            |percent| progress.push(percent),
        )
        .unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.tag, "ZIP: restore.ZIP");
        assert_eq!(outcome.version, bundled::version(&source.folder).unwrap());
        assert_eq!(progress, [90, 100]);
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
        let backups = installer::list(&root, Client::Retail).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read_to_string(backups[0].path.join("previous/AstralTalents/old.lua")).unwrap(),
            "-- previous"
        );
    }

    #[test]
    fn zip_import_rejects_another_addon_non_zip_files_and_bundled_targets_without_writes() {
        let (temp, root, catalog, source) = third_party_fixture("0.1.0");
        let archive = temp.path().join("wrong.zip");
        fs::write(&archive, bundled::package("AstralClock").unwrap()).unwrap();
        assert!(import_zip(&root, Client::Retail, &source, &catalog, &archive, |_| {}).is_err());
        let renamed = temp.path().join("wrong.txt");
        fs::copy(&archive, &renamed).unwrap();
        assert!(
            import_zip(&root, Client::Retail, &source, &catalog, &renamed, |_| {})
                .err()
                .unwrap()
                .contains("Choose an addon ZIP")
        );
        let mut bundled_source = source.clone();
        bundled_source.bundled = Some(true);
        assert!(
            import_zip(
                &root,
                Client::Retail,
                &bundled_source,
                &catalog,
                &archive,
                |_| {}
            )
            .err()
            .unwrap()
            .contains("third-party")
        );
        assert_eq!(
            fs::read_to_string(root.join("Interface/AddOns/AstralTalents/old.lua")).unwrap(),
            "-- previous"
        );
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
        assert!(installer::list(&root, Client::Retail).unwrap().is_empty());
    }

    #[test]
    fn choosing_a_zip_cannot_target_a_changed_game_client_or_folder() {
        let root = Path::new("installation/_retail_");
        let other = Path::new("other/_retail_");
        assert!(import_scope_matches(
            root,
            Client::Retail,
            Some(root),
            Client::Retail
        ));
        assert!(!import_scope_matches(
            root,
            Client::Retail,
            Some(other),
            Client::Retail
        ));
        assert!(!import_scope_matches(
            root,
            Client::Retail,
            Some(root),
            Client::Forever
        ));
        assert!(!import_scope_matches(
            root,
            Client::Retail,
            None,
            Client::Retail
        ));
    }

    #[test]
    fn current_or_newer_installs_skip_the_download_and_keep_files() {
        let tag = bundled::version("AstralTalents").unwrap();
        for version in [&tag, "999.0.0"] {
            let (_temp, root, catalog, source) = fixture(version);
            let result = install_if_newer(
                &root,
                Client::Retail,
                &source,
                &catalog,
                tag.clone(),
                String::new(),
                |_| panic!("An already-current addon must not download a ZIP"),
                |_| {},
            )
            .unwrap();
            assert!(!result.changed);
            assert_eq!(result.version, version);
            assert!(
                root.join("Interface/AddOns/AstralTalents/old.lua")
                    .is_file()
            );
            assert!(installer::list(&root, Client::Retail).unwrap().is_empty());
        }
    }

    #[test]
    fn a_version_change_during_download_cannot_overwrite_the_new_install() {
        let (_temp, root, catalog, source) = fixture("0.1.0");
        let toc = root.join("Interface/AddOns/AstralTalents/AstralTalents.toc");
        let result = install_if_newer(
            &root,
            Client::Retail,
            &source,
            &catalog,
            bundled::version(&source.folder).unwrap(),
            String::new(),
            |_| {
                fs::write(&toc, "## Interface: 120105\n## Version: 0.2.0\nold.lua\n").unwrap();
                bundled::package(&source.folder)
            },
            |_| {},
        );
        assert!(result.err().unwrap().contains("installed version changed"));
        assert!(fs::read_to_string(toc).unwrap().contains("0.2.0"));
        assert!(installer::list(&root, Client::Retail).unwrap().is_empty());
    }

    #[test]
    fn direct_update_installs_with_a_restorable_backup_and_preserves_settings() {
        let (_temp, root, catalog, source) = fixture("0.1.0");
        let mut progress = vec![];
        let result = update(&root, Client::Retail, &source, &catalog, |percent| {
            progress.push(percent)
        })
        .unwrap();
        assert_eq!(progress, [90, 100]);
        assert!(result.changed);
        assert_eq!(result.version, bundled::version(&source.folder).unwrap());
        assert!(
            root.join("Interface/AddOns/AstralTalents/Talents.lua")
                .is_file()
        );
        assert_eq!(
            fs::read_to_string(root.join("WTF/config.wtf")).unwrap(),
            "settings"
        );
        let backups = installer::list(&root, Client::Retail).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read_to_string(backups[0].path.join("previous/AstralTalents/old.lua")).unwrap(),
            "-- previous"
        );
    }
}
