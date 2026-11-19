//! The Library's single-addon action updates directly; package review belongs
//! to community source discovery and manual ZIP imports.
use crate::{Astral, Client, Definition, bundled, installer, library, packages, releases};
use gpui::Context;
use std::{cmp::Ordering, path::Path};

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
    download: impl FnOnce() -> Result<Vec<u8>, String>,
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
    let package = packages::inspect(download()?, root, client, source, catalog)?;
    if package.installed_version.as_deref() != Some(installed.version.as_str()) {
        return Err("The installed version changed. Check for updates again.".into());
    }
    installer::install(&package, &source.name, &tag, &notes)?;
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
) -> Result<Outcome, String> {
    installer::game_closed()?;
    if source.bundled == Some(true) {
        let tag = bundled::version(&source.folder).ok_or("Bundled addon unavailable.")?;
        install_if_newer(root, client, source, catalog, tag, String::new(), || {
            bundled::package(&source.folder)
        })
    } else {
        let release = releases::latest(&releases::client(), source)?;
        install_if_newer(
            root,
            client,
            source,
            catalog,
            release.tag,
            release.notes,
            || releases::download(&release.package),
        )
    }
}

impl Astral {
    pub(crate) fn update_one_addon(&mut self, id: &str, cx: &mut Context<Self>) {
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
                    && (source.bundled == Some(true) || source.repo.is_some())
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
        self.single_update = Some(folder.clone());
        let task = cx.background_executor().spawn(async move {
            let result = update(&root, client, &source, &catalog);
            crate::activity::record(
                preferences.as_deref(),
                crate::activity::Action {
                    root: root.clone(),
                    client,
                    name: source.name.clone(),
                    retry_source: Some(source.id.clone()),
                    from: Some(original.clone()),
                    to: result.as_ref().ok().map(|outcome| outcome.tag.clone()),
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
            let message = format!(
                "{}{}",
                source.name,
                if outcome.version == original {
                    " is already up to date."
                } else {
                    " is installed and backed up."
                }
            );
            let check = releases::Check {
                comparison: releases::compare(&source, &outcome.version, &outcome.tag),
                tag: outcome.tag,
                notes: outcome.notes,
            };
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
                        app.checks.insert(folder.clone(), Ok(check));
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
                || panic!("An already-current addon must not download a ZIP"),
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
            || {
                fs::write(&toc, "## Interface: 120105\n## Version: 0.2.0\nold.lua\n").unwrap();
                bundled::package(&source.folder)
            },
        );
        assert!(result.err().unwrap().contains("installed version changed"));
        assert!(fs::read_to_string(toc).unwrap().contains("0.2.0"));
        assert!(installer::list(&root, Client::Retail).unwrap().is_empty());
    }

    #[test]
    fn direct_update_installs_with_a_restorable_backup_and_preserves_settings() {
        let (_temp, root, catalog, source) = fixture("0.1.0");
        let result = update(&root, Client::Retail, &source, &catalog).unwrap();
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
