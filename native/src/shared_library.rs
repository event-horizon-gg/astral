use crate::{
    Client, Definition, batch,
    cloud::{Service, Session, SharedLibrary},
    installer, library,
    settings_backup::Store,
    sync_format::{Bundle, LibraryAddon, invalid, validate_library},
};
use std::{
    collections::{BTreeMap, HashSet},
    error::Error,
    io,
    path::Path,
};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

fn shareable(addons: &[library::Addon]) -> Vec<LibraryAddon> {
    addons
        .iter()
        .filter(|addon| {
            addon.folder.len() <= 100
                && crate::sync_format::valid_addon(&addon.folder, 100)
                && addon
                    .folder
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-'))
        })
        .map(|addon| {
            let mut name = String::new();
            let mut remaining = 100;
            for ch in addon.name.trim().chars().filter(|ch| !ch.is_control()) {
                if ch.len_utf16() > remaining {
                    break;
                }
                name.push(ch);
                remaining -= ch.len_utf16();
            }
            if name.is_empty() {
                name = addon.folder.clone();
            }
            LibraryAddon {
                folder: addon.folder.clone(),
                name,
            }
        })
        .collect()
}

fn merge(remote: &[LibraryAddon], local: &[LibraryAddon]) -> io::Result<Vec<LibraryAddon>> {
    validate_library(remote)?;
    validate_library(local)?;
    let addons: Vec<_> = remote
        .iter()
        .chain(local)
        .map(|addon| (addon.folder.to_lowercase(), addon.clone()))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect();
    validate_library(&addons)?;
    Ok(addons)
}

fn publish(
    service: &Service,
    session: &Session,
    client: Client,
    local: &[LibraryAddon],
) -> Result<SharedLibrary> {
    for attempt in 0..2 {
        let current = service.shared_library(session, client)?;
        let remote = current.addons.as_deref().unwrap_or_default();
        let addons = merge(remote, local)?;
        if addons == merge(remote, &[])? {
            return Ok(current);
        }
        match service.save_library(session, client, current.revision, &addons) {
            Ok(updated) => return Ok(updated),
            Err(error) if error.status == Some(409) && attempt == 0 => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(invalid("The shared addon list changed. Sync again.").into())
}

#[derive(Default)]
struct Plan {
    queue: Vec<usize>,
    manual: Vec<MissingAddon>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MissingAddon {
    pub folder: String,
    pub name: String,
    pub reason: &'static str,
}

pub fn missing_names(addons: &[MissingAddon]) -> String {
    addons
        .iter()
        .map(|addon| addon.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

struct Planner<'a> {
    catalog: &'a [Definition],
    client: Client,
    installed: HashSet<String>,
    visiting: HashSet<String>,
    seen: HashSet<String>,
    plan: Plan,
}

impl Planner<'_> {
    fn visit(&mut self, folder: &str, name: &str, required: bool) -> io::Result<()> {
        let key = folder.to_lowercase();
        if self.installed.contains(&key) || self.seen.contains(&key) {
            return Ok(());
        }
        let mut definitions = self
            .catalog
            .iter()
            .enumerate()
            .filter(|(_, source)| source.folder.eq_ignore_ascii_case(folder));
        let definition = definitions.clone().next().map(|(_, source)| source);
        let source = definitions.find_map(|(index, source)| {
            source
                .supports(self.client)
                .then_some(index)
                .filter(|_| source.repo.is_some() || source.bundled == Some(true))
        });
        let Some(source) = source else {
            if required {
                self.plan.manual.push(MissingAddon {
                    folder: folder.into(),
                    name: definition
                        .map(|source| source.name.as_str())
                        .unwrap_or(if name.is_empty() { folder } else { name })
                        .into(),
                    reason: if definition.is_some_and(|source| !source.supports(self.client)) {
                        "No compatible source for this game client."
                    } else {
                        "A manual install or source review is required."
                    },
                });
                self.seen.insert(key);
            }
            return Ok(());
        };
        if !self.visiting.insert(key.clone()) {
            return Err(invalid("The addon collection contains a dependency cycle."));
        }
        let requires = self.catalog[source].requires.clone();
        for dependency in requires {
            self.visit(&dependency, &dependency, true)?;
        }
        self.visiting.remove(&key);
        self.seen.insert(key);
        self.plan.queue.push(source);
        Ok(())
    }
}

fn build_plan(
    inventory: &[library::Addon],
    catalog: &[Definition],
    client: Client,
    shared: &[LibraryAddon],
    target: Option<&Bundle>,
) -> io::Result<Plan> {
    let installed = inventory
        .iter()
        .flat_map(|addon| addon.modules.iter())
        .map(|folder| folder.to_lowercase())
        .collect();
    let mut planner = Planner {
        catalog,
        client,
        installed,
        visiting: HashSet::new(),
        seen: HashSet::new(),
        plan: Plan::default(),
    };
    for addon in shared {
        planner.visit(&addon.folder, &addon.name, false)?;
    }
    if let Some(target) = target
        && let Some(addons) = &target.library
    {
        for addon in addons {
            let enabled = target.addon_selection.as_ref().is_none_or(|selection| {
                selection.scopes.values().any(|scope| {
                    scope.iter().any(|(folder, enabled)| {
                        *enabled && folder.eq_ignore_ascii_case(&addon.folder)
                    })
                })
            });
            if enabled {
                planner.visit(&addon.folder, &addon.name, true)?;
            }
        }
    }
    Ok(planner.plan)
}

pub struct CollectionResult {
    pub library: SharedLibrary,
    pub installed: usize,
    pub manual: Vec<MissingAddon>,
}

pub fn install_for_restore(
    root: &Path,
    client: Client,
    catalog: &[Definition],
    store: &Store,
    target: &Bundle,
) -> Result<usize> {
    let inventory = library::scan(root, client, catalog)?;
    let plan = build_plan(&inventory, catalog, client, &[], Some(target))?;
    if !plan.manual.is_empty() {
        return Err(invalid(&format!(
            "Install these addons before restoring: {}.",
            missing_names(&plan.manual)
        ))
        .into());
    }
    install_packages(root, client, catalog, store, &plan.queue)
}

fn install_packages(
    root: &Path,
    client: Client,
    catalog: &[Definition],
    store: &Store,
    queue: &[usize],
) -> Result<usize> {
    if queue.is_empty() {
        return Ok(0);
    }
    let original = store.addon_choices()?;
    let mut installed = 0;
    for &index in queue {
        let source = &catalog[index];
        let done =
            batch::install_missing(index, root, client, catalog, Some(store.preferences_path()))
                .map_err(|error| io::Error::other(format!("{}: {error}", source.name)))?;
        if done {
            store.preserve_addon_choices(&original, catalog)?;
            installed += 1;
        }
    }
    Ok(installed)
}

pub fn sync_installation(
    service: &Service,
    session: &Session,
    root: &Path,
    client: Client,
    catalog: &[Definition],
    data: &Path,
    selected: Option<String>,
) -> Result<CollectionResult> {
    let accounts = crate::settings_backup::accounts(root)?;
    let account = selected
        .filter(|name| accounts.contains(name))
        .or_else(|| (accounts.len() == 1).then(|| accounts[0].clone()));
    let store = account
        .as_ref()
        .map(|account| Store::open(data, root, client, account))
        .transpose()?;
    let _lock = store.as_ref().map(Store::lock).transpose()?;
    sync(
        service,
        session,
        root,
        client,
        catalog,
        store.as_ref(),
        None,
    )
}

/// Caller holds the settings lock when a local account is present.
pub fn sync(
    service: &Service,
    session: &Session,
    root: &Path,
    client: Client,
    catalog: &[Definition],
    store: Option<&Store>,
    target: Option<(crate::settings_backup::Profile, &Bundle)>,
) -> Result<CollectionResult> {
    installer::game_closed().map_err(io::Error::other)?;
    let inventory = library::scan(root, client, catalog)?;
    let mut local = shareable(&inventory);
    if let Some((profile, _)) = target
        && profile != crate::settings_backup::Profile::Regular
    {
        let legacy = service.profile_library(session, client, profile)?;
        local = merge(&local, legacy.addons.as_deref().unwrap_or_default())?;
    }
    let mut shared = publish(service, session, client, &local)?;
    let mut installed = 0;
    let mut manual = Vec::new();
    if let Some(store) = store {
        let plan = build_plan(
            &inventory,
            catalog,
            client,
            shared.addons.as_deref().unwrap_or_default(),
            target.map(|(_, bundle)| bundle),
        )?;
        manual = plan.manual;
        if !plan.queue.is_empty() {
            let install = || install_packages(root, client, catalog, store, &plan.queue);
            installed = if target.is_none() {
                crate::sync_engine::with_collection_baseline(store, session, install)?
            } else {
                install()?
            };
        }
        if installed > 0 {
            shared = publish(
                service,
                session,
                client,
                &shareable(&library::scan(root, client, catalog)?),
            )?;
        }
    }
    Ok(CollectionResult {
        library: shared,
        installed,
        manual,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_format::Selection;

    fn item(folder: &str) -> LibraryAddon {
        LibraryAddon {
            folder: folder.into(),
            name: folder.into(),
        }
    }

    #[test]
    fn required_addon_from_another_client_reports_the_compatible_source_gap() -> io::Result<()> {
        let catalog = library::catalog();
        let target = Bundle {
            schema: 2,
            files: vec![],
            addon_selection: None,
            library: Some(vec![item("ConsolePort")]),
            input: None,
        };
        let plan = build_plan(&[], &catalog, Client::Forever, &[], Some(&target))?;
        assert!(plan.queue.is_empty());
        assert_eq!(
            plan.manual,
            [MissingAddon {
                folder: "ConsolePort".into(),
                name: "ConsolePort".into(),
                reason: "No compatible source for this game client.",
            }]
        );
        Ok(())
    }

    #[test]
    fn shared_plan_installs_dependencies_first_and_reports_only_enabled_manual_addons()
    -> io::Result<()> {
        let catalog = library::catalog();
        let target = Bundle {
            schema: 2,
            files: vec![],
            addon_selection: Some(Selection {
                schema: 1,
                scopes: BTreeMap::from([(
                    "".into(),
                    BTreeMap::from([("ManualOne".into(), true), ("ManualTwo".into(), false)]),
                )]),
            }),
            library: Some(vec![item("ManualOne"), item("ManualTwo")]),
            input: None,
        };
        let plan = build_plan(
            &[],
            &catalog,
            Client::Forever,
            &[
                item("AstralProfiles"),
                item("EllesmereUI"),
                item("ConsolePort"),
                item("UnknownShared"),
            ],
            Some(&target),
        )?;
        assert_eq!(
            plan.queue
                .iter()
                .map(|&index| catalog[index].folder.as_str())
                .collect::<Vec<_>>(),
            ["EllesmereUI", "AstralProfiles"]
        );
        assert_eq!(
            plan.manual,
            [MissingAddon {
                folder: "ManualOne".into(),
                name: "ManualOne".into(),
                reason: "A manual install or source review is required.",
            }]
        );
        let merged = merge(
            &[item("KeyUI"), item("Questie")],
            &[item("keyui"), item("TomTom")],
        )?;
        assert_eq!(
            merged
                .into_iter()
                .map(|item| item.folder)
                .collect::<Vec<_>>(),
            ["keyui", "Questie", "TomTom"]
        );
        Ok(())
    }
}
