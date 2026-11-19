use crate::{
    Client, library,
    settings_backup::{Entry, child, link, read_bounded, safe_component},
    sync_format::{Selection, invalid, valid_addon},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
    sync::LazyLock,
};

type Choices = BTreeMap<String, bool>;

pub type ProfileEdits = BTreeMap<String, bool>;

pub struct ProfileAddon {
    pub folder: String,
    pub name: String,
    pub enabled: Option<bool>,
    pub managed: bool,
}

#[derive(Clone)]
pub struct ChoiceChange {
    pub name: String,
    pub before: Option<bool>,
    pub after: Option<bool>,
}

struct Installed {
    name: String,
    default: bool,
    dependencies: Vec<String>,
}

pub struct Inventory {
    installed: BTreeMap<String, Installed>,
    files: BTreeMap<String, ChoiceFile>,
}

struct ChoiceFile {
    path: String,
    text: String,
}

fn directories(path: &Path) -> io::Result<Vec<String>> {
    let mut result = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir()
            && !link(&metadata)
            && let Ok(name) = entry.file_name().into_string()
            && safe_component(&name)
        {
            result.push(name);
        }
    }
    result.sort();
    Ok(result)
}

impl Inventory {
    pub fn open(root: &Path, account: &Path, client: Client, config: &[u8]) -> io::Result<Self> {
        static DEFAULT: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r#"(?im)^SET\s+disableUserAddonsByDefault\s+"1"[ \t\r]*$"#).unwrap()
        });
        let disabled_by_default = DEFAULT.is_match(&String::from_utf8_lossy(config));
        let addons = child(root, "Interface/AddOns")?;
        let mut installed = BTreeMap::new();
        for name in directories(&addons)?
            .into_iter()
            .filter(|name| valid_addon(name, 128))
        {
            let meta = library::toc(&addons.join(&name), &name, client)?.unwrap_or_default();
            let dependencies = meta
                .get("Dependencies")
                .or_else(|| meta.get("RequiredDeps"))
                .map(|names| {
                    names
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .filter(|name| {
                            !name.is_empty() && !name.to_ascii_lowercase().starts_with("blizzard_")
                        })
                        .map(str::to_lowercase)
                        .collect()
                })
                .unwrap_or_default();
            let default = !disabled_by_default
                && !meta
                    .get("DefaultState")
                    .is_some_and(|value| value.eq_ignore_ascii_case("disabled"));
            if installed
                .insert(
                    name.to_lowercase(),
                    Installed {
                        name,
                        default,
                        dependencies,
                    },
                )
                .is_some()
            {
                return Err(invalid("Two addon folders differ only by letter case."));
            }
        }
        let mut files = BTreeMap::from([(String::new(), choice_file(account, "")?)]);
        for realm in directories(account)?
            .into_iter()
            .filter(|name| !name.eq_ignore_ascii_case("SavedVariables"))
        {
            for character in directories(&account.join(&realm))?
                .into_iter()
                .filter(|name| !name.eq_ignore_ascii_case("SavedVariables"))
            {
                let scope = format!("{realm}/{character}");
                if files.len() == 500 {
                    return Err(invalid("Too many character addon selections."));
                }
                files.insert(scope.clone(), choice_file(account, &scope)?);
            }
        }
        Ok(Self { installed, files })
    }

    pub fn selection(&self) -> io::Result<Selection> {
        let mut known = BTreeMap::new();
        let mut explicit = BTreeMap::new();
        for (scope, file) in &self.files {
            let mut choices = BTreeMap::new();
            for line in file.text.trim_start_matches('\u{feff}').lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((name, enabled)) = line.split_once(':') else {
                    return Err(invalid(
                        "An addon selection file has an unsupported format.",
                    ));
                };
                let name = name.trim();
                if name.to_ascii_lowercase().starts_with("blizzard_") {
                    continue;
                }
                if !valid_addon(name, 128) {
                    return Err(invalid("Invalid addon in selection file."));
                }
                let enabled = match enabled.trim().to_ascii_lowercase().as_str() {
                    "enabled" => true,
                    "disabled" => false,
                    _ => {
                        return Err(invalid(
                            "An addon selection file has an unsupported format.",
                        ));
                    }
                };
                known.insert(name.to_lowercase(), name.to_owned());
                choices.insert(name.to_lowercase(), enabled);
            }
            explicit.insert(scope.clone(), choices);
        }
        for (key, addon) in &self.installed {
            known.insert(key.clone(), addon.name.clone());
        }
        let account = &explicit[""];
        let scopes = explicit
            .iter()
            .map(|(scope, choices)| {
                let values = known
                    .iter()
                    .map(|(key, name)| {
                        let enabled = choices
                            .get(key)
                            .or_else(|| account.get(key))
                            .copied()
                            .unwrap_or_else(|| {
                                self.installed.get(key).is_none_or(|addon| addon.default)
                            });
                        (name.clone(), enabled)
                    })
                    .collect();
                (scope.clone(), values)
            })
            .collect();
        let selection = Selection { schema: 1, scopes };
        selection.validate()?;
        Ok(selection)
    }

    pub fn plan(&self, account: &Path, selection: &Selection) -> io::Result<Vec<Entry>> {
        self.plan_choices(account, selection, None)
    }

    pub fn plan_profile_edits(
        &self,
        account: &Path,
        addons: &[library::Addon],
        edits: &ProfileEdits,
    ) -> io::Result<Vec<Entry>> {
        let mut selection = self.selection()?;
        let mut changed = BTreeSet::new();
        for (folder, enabled) in edits {
            let addon = addons
                .iter()
                .find(|addon| &addon.folder == folder)
                .ok_or_else(|| {
                    invalid("An addon is no longer installed. Review the profile again.")
                })?;
            for module in &addon.modules {
                let key = module.to_lowercase();
                changed.insert(key.clone());
                for choices in selection.scopes.values_mut() {
                    if let Some(name) = choices
                        .keys()
                        .find(|name| name.to_lowercase() == key)
                        .cloned()
                    {
                        choices.insert(name, *enabled);
                    }
                }
            }
        }
        self.plan_choices(account, &selection, Some(&changed))
    }

    pub fn with_snapshot(mut self, files: &[Entry]) -> io::Result<Self> {
        for file in self.files.values_mut() {
            file.text.clear();
        }
        for entry in files {
            let (scope, name) = entry.path.rsplit_once('/').unwrap_or(("", &entry.path));
            if name.eq_ignore_ascii_case("AddOns.txt") {
                self.files.insert(
                    scope.to_owned(),
                    ChoiceFile {
                        path: entry.path.clone(),
                        text: String::from_utf8(entry.decode()?)
                            .map_err(|_| invalid("An addon selection file is unreadable."))?,
                    },
                );
            }
        }
        Ok(self)
    }

    fn plan_choices(
        &self,
        account: &Path,
        selection: &Selection,
        changed: Option<&BTreeSet<String>>,
    ) -> io::Result<Vec<Entry>> {
        selection.validate()?;
        let before = self.selection()?;
        let mut known = BTreeMap::new();
        for values in before.scopes.values().chain(selection.scopes.values()) {
            for name in values.keys() {
                known.insert(name.to_lowercase(), name.clone());
            }
        }
        for (key, addon) in &self.installed {
            known.insert(key.clone(), addon.name.clone());
        }
        if let Some(changed) = changed {
            known.retain(|key, _| changed.contains(key));
        }
        let scopes: BTreeSet<_> = self.files.keys().chain(selection.scopes.keys()).collect();
        if scopes.len() > 500 || known.len() > 2_000 {
            return Err(invalid(
                "The combined addon selections exceed the sync limit.",
            ));
        }
        let mut result = Vec::new();
        for scope in scopes {
            let previous = scope_choices(&before, scope);
            let target = scope_choices(selection, scope);
            for (key, addon) in &self.installed {
                if !target.get(key).copied().unwrap_or(false) {
                    continue;
                }
                for dependency in &addon.dependencies {
                    let enabled = self.installed.contains_key(dependency)
                        && target.get(dependency) == Some(&true);
                    if !enabled
                        && (previous.get(key) != Some(&true)
                            || previous.get(dependency) == Some(&true))
                    {
                        return Err(invalid(&format!(
                            "{} requires {}. Enable both addons in this profile.",
                            addon.name, dependency
                        )));
                    }
                }
            }
            let loaded;
            let file = if let Some(file) = self.files.get(scope) {
                file
            } else {
                loaded = choice_file(account, scope)?;
                &loaded
            };
            let ending = if file.text.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let mut lines: Vec<_> = file
                .text
                .trim_start_matches('\u{feff}')
                .lines()
                .filter(|line| {
                    !line
                        .split_once(':')
                        .is_some_and(|(name, _)| known.contains_key(&name.trim().to_lowercase()))
                })
                .map(str::to_owned)
                .collect();
            while lines.last().is_some_and(String::is_empty) {
                lines.pop();
            }
            for (key, name) in &known {
                let enabled = if target.get(key) == Some(&true) {
                    "enabled"
                } else {
                    "disabled"
                };
                lines.push(format!("{name}: {enabled}"));
            }
            if !lines.is_empty() {
                lines.push(String::new());
            }
            let mut text = lines.join(ending);
            if file.text.starts_with('\u{feff}') {
                text.insert(0, '\u{feff}');
            }
            if !text.is_empty() {
                result.push(Entry::new(file.path.clone(), text.as_bytes()));
            }
        }
        Ok(result)
    }
}

pub fn group_enabled(selection: &Selection, addon: &library::Addon) -> Option<bool> {
    let mut states = selection.scopes.values().flat_map(|choices| {
        addon.modules.iter().filter_map(|module| {
            choices
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(module))
                .map(|(_, enabled)| *enabled)
        })
    });
    let first = states.next()?;
    states.all(|enabled| enabled == first).then_some(first)
}

fn scope_choices(selection: &Selection, scope: &str) -> Choices {
    selection
        .scopes
        .get(scope)
        .unwrap_or(&selection.scopes[""])
        .iter()
        .map(|(name, enabled)| (name.to_lowercase(), *enabled))
        .collect()
}

fn choice_file(account: &Path, scope: &str) -> io::Result<ChoiceFile> {
    let directory = if scope.is_empty() {
        account.to_path_buf()
    } else {
        child(account, scope)?
    };
    let mut name = None;
    match fs::read_dir(&directory) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if let Ok(file) = entry.file_name().into_string()
                    && file.eq_ignore_ascii_case("AddOns.txt")
                {
                    if name.replace(file).is_some() {
                        return Err(invalid("Duplicate addon selection files need attention."));
                    }
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let name = name.as_deref().unwrap_or("AddOns.txt");
    let relative = if scope.is_empty() {
        name.into()
    } else {
        format!("{scope}/{name}")
    };
    let path = child(account, &relative)?;
    let text = match read_bounded(&path, 8 * 1024 * 1024) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| invalid("An addon selection file is unreadable."))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    Ok(ChoiceFile {
        path: relative,
        text,
    })
}
