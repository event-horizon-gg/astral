use crate::{Astral, Client, Palette, icon, installer, preferences};
use gpui::{
    App, Context, ElementId, FocusHandle, SharedString, Stateful, Window, div, prelude::*, px, rgb,
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
pub struct State {
    request: u64,
    entries: Option<Result<Vec<installer::Backup>, String>>,
    confirmation: Option<PathBuf>,
    confirmation_focus: Option<RestoreFocus>,
    restoring: bool,
    opening_collection: bool,
    outcome: Option<Result<String, String>>,
    release_notes: std::collections::HashSet<(u64, usize, usize)>,
    backup_notes: std::collections::HashSet<PathBuf>,
    archive_error: Option<String>,
    recent: Vec<Action>,
    recent_error: Option<String>,
    button_focus: std::cell::RefCell<std::collections::HashMap<ButtonKey, FocusHandle>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ButtonKey {
    OpenFolder,
    OpenLibrary,
    Archive(PathBuf),
    ShowFiles(PathBuf),
    Restore(PathBuf),
    Retry(String, usize),
}

fn retry_keys(entries: &[Action]) -> Vec<ButtonKey> {
    let mut counts = std::collections::HashMap::new();
    entries
        .iter()
        .map(|entry| {
            // Older activity records have no ID. Preserve content identity across
            // reordering, with an occurrence counter for otherwise identical rows.
            let identity = format!(
                "{:?}",
                (
                    &entry.root,
                    entry.client.index(),
                    entry.time,
                    &entry.name,
                    &entry.retry_source,
                    &entry.from,
                    &entry.to,
                    &entry.verb,
                    &entry.error
                )
            );
            let count = counts.entry(identity.clone()).or_insert(0);
            let key = ButtonKey::Retry(identity, *count);
            *count += 1;
            key
        })
        .collect()
}

struct RestoreFocus {
    cancel: FocusHandle,
    restore: FocusHandle,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Action {
    pub root: PathBuf,
    pub client: Client,
    pub name: String,
    pub retry_source: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub verb: String,
    pub error: Option<String>,
    pub time: u64,
}

fn recent_actions(path: &Path) -> io::Result<Vec<Action>> {
    match fs::read(path.with_file_name("addon-activity.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

pub(crate) fn record(path: Option<&Path>, mut action: Action) {
    let Some(path) = path else { return };
    let result = (|| -> io::Result<()> {
        let lock_path = path.with_file_name("addon-activity.lock");
        fs::create_dir_all(lock_path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
        let lock = fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock()?;
        let mut entries = recent_actions(path)?;
        action.root = action.root.canonicalize().unwrap_or(action.root);
        action.time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        entries.insert(0, action);
        let roots: Vec<_> = entries
            .iter()
            .map(|entry| (entry.client, entry.root.clone()))
            .collect();
        if let Err(error) = crate::backup_roots::remember(path, &roots) {
            eprintln!("Couldn't record backup installations: {error}");
        }
        entries.truncate(100);
        preferences::save_json(&path.with_file_name("addon-activity.json"), &entries)
    })();
    if let Err(error) = result {
        eprintln!("Couldn't save addon activity. {error}");
    }
}

fn earlier_archives(preferences: Option<&Path>) -> Vec<(&'static str, PathBuf)> {
    let support = preferences.and_then(Path::parent).and_then(Path::parent);
    let electron = if cfg!(target_os = "windows") {
        env::var_os("APPDATA").map(PathBuf::from)
    } else {
        support.map(Path::to_path_buf)
    };
    archive_locations(electron.as_deref(), support)
}

fn archive_locations(
    electron: Option<&Path>,
    support: Option<&Path>,
) -> Vec<(&'static str, PathBuf)> {
    let mut paths: Vec<_> = electron
        .into_iter()
        .flat_map(|dir| ["astral-companion", "Astral"].map(|name| dir.join(name).join("backups")))
        .map(|path| ("Earlier Astral backups", path))
        .chain(support.map(|dir| {
            (
                "Earlier Raider.IO backups",
                dir.join("Astral RaiderIO/Backups"),
            )
        }))
        .filter(|(_, path)| path.is_dir())
        .collect();
    let mut seen = std::collections::HashSet::new();
    paths.retain(|(_, path)| seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())));
    paths
}

fn earlier_activity_locations(preferences: Option<&Path>) -> Vec<PathBuf> {
    let support = if cfg!(target_os = "windows") {
        env::var_os("APPDATA").map(PathBuf::from)
    } else {
        preferences
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    };
    support
        .into_iter()
        .flat_map(|dir| ["astral-companion", "Astral"].map(|name| dir.join(name)))
        .collect()
}

fn earlier_actions(
    data_folders: &[PathBuf],
    root: &Path,
    client: Client,
) -> io::Result<Vec<Action>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EarlierAction {
        id: String,
        root: PathBuf,
        addon_id: String,
        name: String,
        action: String,
        status: String,
        time: String,
        from: Option<String>,
        to: Option<String>,
        message: Option<String>,
    }
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut seen = std::collections::HashSet::new();
    let mut actions = Vec::new();
    for data in data_folders {
        let bytes = match fs::read(data.join("addon-activity.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        // Read entries independently so one old or malformed row cannot hide the rest.
        let entries: Vec<serde_json::Value> = serde_json::from_slice(&bytes)?;
        for value in entries {
            let Ok(entry) = serde_json::from_value::<EarlierAction>(value) else {
                continue;
            };
            if entry
                .root
                .canonicalize()
                .unwrap_or_else(|_| entry.root.clone())
                != canonical
            {
                continue;
            }
            let Ok(time) = time::OffsetDateTime::parse(
                &entry.time,
                &time::format_description::well_known::Rfc3339,
            ) else {
                continue;
            };
            let Ok(time) = u64::try_from(time.unix_timestamp()) else {
                continue;
            };
            if !seen.insert(entry.id) {
                continue;
            }
            let verb = match entry.status.as_str() {
                "error" => "Failed",
                "current" => "Already up to date",
                "success" if entry.action == "restore" => "Restored",
                "success" => "Updated",
                _ => continue,
            };
            actions.push(Action {
                root: canonical.clone(),
                client,
                name: entry.name,
                retry_source: (entry.action == "update").then_some(entry.addon_id),
                from: entry.from,
                to: entry.to,
                verb: verb.into(),
                error: (entry.status == "error").then(|| {
                    entry
                        .message
                        .unwrap_or_else(|| "Addon operation failed.".into())
                }),
                time,
            });
        }
    }
    Ok(actions)
}

fn preserve_history(preferences: Option<&Path>, roots: &[(Client, PathBuf)]) -> Option<String> {
    let preferences = preferences?;
    let mut warning = None;
    for (client, root) in roots {
        if let Err(error) = installer::preserve_backups(root, *client, preferences) {
            warning = Some(error);
        }
    }
    warning
}

fn backup_history(
    root: Option<&Path>,
    client: Client,
    known_roots: &[(Client, PathBuf)],
    archives: &[(&str, PathBuf)],
    catalog: &[crate::Definition],
) -> (Result<Vec<installer::Backup>, String>, Option<String>) {
    let mut archive_error = None;
    let mut records = if let Some(root) = root {
        installer::list(root, client)
    } else {
        let mut records = Vec::new();
        for (client, root) in known_roots {
            match installer::list(root, *client) {
                Ok(found) => records.extend(found),
                Err(error) => archive_error = Some(error),
            }
        }
        Ok(records)
    };
    if let Ok(records) = &mut records {
        for (_, archive) in archives
            .iter()
            .filter(|(label, _)| *label == "Earlier Astral backups")
        {
            let earlier = if let Some(root) = root {
                installer::list_electron(archive, root, client, catalog)
            } else {
                installer::list_electron_unfiltered(archive, catalog)
            };
            match earlier {
                Ok(found) => records.extend(found),
                Err(error) => archive_error = Some(error),
            }
            match installer::list_saved(archive, root.map(|root| (root, client))) {
                Ok(found) => {
                    let journals: std::collections::HashSet<_> =
                        found.iter().map(|(_, path)| path.clone()).collect();
                    records.retain(|record| record.interrupted || !journals.contains(&record.path));
                    records.extend(found.into_iter().map(|(record, _)| record));
                }
                Err(error) => archive_error = Some(error),
            }
        }
        let mut seen = std::collections::HashSet::new();
        records.retain(|backup| {
            seen.insert(
                backup
                    .path
                    .canonicalize()
                    .unwrap_or_else(|_| backup.path.clone()),
            )
        });
        installer::sort_backups(records);
    }
    (records, archive_error)
}

struct PreparedCollection {
    path: PathBuf,
    warning: Option<String>,
}

fn activity_button(
    button: Stateful<gpui::Div>,
    palette: Palette,
    activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    crate::keyboard_button(button, palette, activate)
}

fn activity_text_base(
    palette: Palette,
    id: impl Into<ElementId>,
    label: &str,
) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(7.))
        .py(px(5.))
        .flex_shrink_0()
        .rounded(px(4.))
        .border_1()
        .border_color(gpui::transparent_black())
        .cursor_pointer()
        .text_size(px(11.))
        .text_color(rgb(palette.muted))
        .hover(move |style| style.text_color(rgb(palette.accent)))
        .child(label.to_owned())
}

fn activity_text_button(
    palette: Palette,
    id: impl Into<ElementId>,
    label: &str,
    activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    activity_button(activity_text_base(palette, id, label), palette, activate)
}

fn outlined_activity_text_button(
    palette: Palette,
    id: impl Into<ElementId>,
    label: &str,
    focus: &FocusHandle,
    enabled: bool,
    activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    let button = activity_text_base(palette, id, label);
    if enabled {
        crate::outlined_button(button, palette, focus, activate)
    } else {
        button.opacity(0.45).cursor_default()
    }
}

fn activity_link(
    palette: Palette,
    id: impl Into<ElementId>,
    label: &str,
    focus: &FocusHandle,
    activate: impl Fn(&mut App) + 'static,
) -> Stateful<gpui::Div> {
    crate::button_outline(
        activity_text_button(palette, id, label, move |_, _, cx| activate(cx)),
        palette,
        focus,
    )
}

impl State {
    fn control_focus(&self, key: ButtonKey, cx: &App) -> FocusHandle {
        self.button_focus
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }

    fn retain_control_focus(&self, archives: &[(&str, PathBuf)]) {
        let mut keys =
            std::collections::HashSet::from([ButtonKey::OpenFolder, ButtonKey::OpenLibrary]);
        if let Some(Ok(entries)) = &self.entries {
            for backup in entries {
                keys.insert(ButtonKey::ShowFiles(backup.path.clone()));
                keys.insert(ButtonKey::Restore(backup.path.clone()));
            }
        }
        keys.extend(
            archives
                .iter()
                .map(|(_, path)| ButtonKey::Archive(path.clone())),
        );
        keys.extend(retry_keys(&self.recent));
        self.button_focus
            .borrow_mut()
            .retain(|key, _| keys.contains(key));
    }

    pub(crate) fn addon_backup(&self, addon: &crate::Addon) -> Option<&installer::Backup> {
        self.entries.as_ref()?.as_ref().ok()?.iter().find(|backup| {
            !backup.interrupted
                && !backup.recovered
                && !backup.transaction.removed
                && backup.transaction.profile_switch.is_none()
                && backup.transaction.previous.contains(&addon.folder)
        })
    }

    pub(crate) fn addon_error(
        &self,
        addon: &crate::Addon,
        source_id: Option<&str>,
    ) -> Option<&str> {
        self.recent
            .iter()
            .find(|entry| {
                if let (Some(entry_source), Some(source)) =
                    (entry.retry_source.as_deref(), source_id)
                {
                    entry_source == source
                } else {
                    entry.name == addon.name
                }
            })?
            .error
            .as_deref()
    }

    pub(crate) fn confirm_addon_restore(&mut self, path: PathBuf) {
        if !self.restoring {
            self.confirmation = Some(path);
            self.outcome = None;
        }
    }

    pub fn has_confirmation(&self) -> bool {
        self.confirmation.is_some()
    }

    pub(crate) fn operation_running(&self) -> bool {
        self.restoring
    }

    pub(crate) fn has_pending_operation(&self) -> bool {
        self.has_confirmation() || self.restoring
    }

    fn selected(&self) -> Option<&installer::Backup> {
        let path = self.confirmation.as_ref()?;
        self.entries
            .as_ref()?
            .as_ref()
            .ok()?
            .iter()
            .find(|entry| &entry.path == path)
    }

    pub fn dismiss_confirmation(&mut self) -> bool {
        if self.restoring {
            return false;
        }
        let dismissed = self.confirmation.take().is_some();
        if dismissed {
            self.outcome = None;
        }
        dismissed
    }
}

impl Astral {
    pub(crate) fn update_addon_restore_focus(&mut self, window: &mut Window, cx: &Context<Self>) {
        if self.activity.selected().is_some() {
            self.activity
                .confirmation_focus
                .get_or_insert_with(|| RestoreFocus {
                    cancel: cx.focus_handle().tab_stop(true),
                    restore: cx.focus_handle().tab_stop(true),
                });
        } else if let Some(focus) = self.activity.confirmation_focus.take()
            && (focus.cancel.is_focused(window) || focus.restore.is_focused(window))
        {
            // main.jsx's ordinary modal has no autofocus, Tab trap or opener
            // restoration. A removed focused button falls back to the page.
            self.root_focus.focus(window);
        }
    }

    fn activity_archives(&self) -> Vec<(&'static str, PathBuf)> {
        let mut archives = if self.fixture {
            Vec::new()
        } else {
            earlier_archives(self.preferences_path.as_deref())
        };
        if let Some(folder) = self
            .preferences_path
            .as_deref()
            .and_then(Path::parent)
            .map(|folder| folder.join("backups"))
            && !archives.iter().any(|(_, path)| path == &folder)
        {
            archives.push(("Earlier Astral backups", folder));
        }
        archives
    }

    fn backup_collection_task(
        &self,
        cx: &Context<Self>,
    ) -> gpui::Task<Result<PreparedCollection, String>> {
        let preferences = self.preferences_path.clone();
        let known_roots: Vec<_> = Client::ALL
            .into_iter()
            .filter_map(|client| self.root(client).cloned().map(|root| (client, root)))
            .collect();
        let archives = self.activity_archives();
        let catalog = self.catalog.clone();
        cx.background_executor().spawn(async move {
            let preferences = preferences.ok_or("Astral storage is unavailable.")?;
            let (roots, roots_error) =
                crate::backup_roots::discover(Some(&preferences), &known_roots);
            let preserve_error = preserve_history(Some(&preferences), &roots);
            let (records, scan_error) =
                backup_history(None, Client::Retail, &roots, &archives, &catalog);
            let folders = records?
                .into_iter()
                .map(|backup| crate::backup_collection::Folder {
                    label: format!(
                        "{}-{}",
                        backup
                            .transaction
                            .created_ms
                            .unwrap_or(backup.transaction.created.saturating_mul(1000)),
                        backup.transaction.addon
                    ),
                    path: backup.path,
                })
                .collect();
            let warning = roots_error.or(preserve_error).or(scan_error);
            match crate::backup_collection::update(preferences.clone(), folders, warning.is_none())
            {
                Ok(path) => Ok(PreparedCollection { path, warning }),
                Err(error) => {
                    // Browsing existing backups must still work if refreshing
                    // optional collection metadata fails. Never replace that metadata.
                    let path = crate::backup_collection::existing(&preferences)
                        .map_err(|_| error.to_string())?;
                    Ok(PreparedCollection {
                        path,
                        warning: Some(error.to_string()),
                    })
                }
            }
        })
    }

    fn await_backup_collection(
        &mut self,
        task: impl std::future::Future<Output = Result<PreparedCollection, String>> + 'static,
        open: impl FnOnce(PathBuf, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.activity.opening_collection {
            return;
        }
        self.activity.opening_collection = true;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.activity.opening_collection = false;
                match result {
                    Ok(collection) => {
                        if let Some(warning) = collection.warning {
                            app.show_notification(warning, true, cx);
                        }
                        open(collection.path, cx);
                    }
                    Err(error) => app.show_notification(error, true, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_backup_collection(&mut self, cx: &mut Context<Self>) {
        if self.activity.opening_collection {
            return;
        }
        let task = self.backup_collection_task(cx);
        self.await_backup_collection(
            task,
            |path, cx| {
                #[cfg(target_os = "windows")]
                let path = crate::backup_collection::shell_path(&path);
                cx.open_with_system(&path);
            },
            cx,
        );
    }

    pub fn load_backups(&mut self, cx: &mut Context<Self>) {
        if self.activity.restoring {
            return;
        }
        let client = self.preferences.client;
        self.activity.request += 1;
        let request = self.activity.request;
        self.activity.entries = None;
        self.activity.confirmation = None;
        self.activity.outcome = None;
        self.activity.archive_error = None;
        self.activity.backup_notes.clear();
        self.activity.recent.clear();
        self.activity.recent_error = None;
        let root = self.root(client).cloned();
        let known_roots: Vec<_> = [Client::Retail, Client::Forever]
            .into_iter()
            .filter_map(|client| self.root(client).cloned().map(|root| (client, root)))
            .collect();
        let archives = self.activity_archives();
        let catalog = self.catalog.clone();
        let preferences = self.preferences_path.clone();
        let earlier_activity = if self.fixture {
            Vec::new()
        } else {
            earlier_activity_locations(preferences.as_deref())
        };
        let task = cx.background_executor().spawn(async move {
            let mut recent = preferences
                .as_deref()
                .map_or_else(|| Ok(Vec::new()), recent_actions)
                .map(|mut entries| {
                    if let Some(root) = &root {
                        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
                        entries.retain(|entry| entry.client == client && entry.root == canonical);
                    } else {
                        entries.clear();
                    }
                    entries
                });
            match root.as_ref().map_or_else(
                || Ok(Vec::new()),
                |root| earlier_actions(&earlier_activity, root, client),
            ) {
                Ok(earlier) => {
                    if let Ok(entries) = &mut recent {
                        entries.extend(earlier);
                        entries.sort_by_key(|entry| std::cmp::Reverse(entry.time));
                        entries.truncate(100);
                    }
                }
                Err(error) => {
                    // Keep current native history available if an old archive is unreadable.
                    eprintln!("Couldn't read earlier addon activity. {error}");
                }
            }
            let (known_roots, roots_error) = if root.is_none() {
                crate::backup_roots::discover(preferences.as_deref(), &known_roots)
            } else {
                (known_roots, None)
            };
            let preserve_error = preserve_history(preferences.as_deref(), &known_roots);
            let (records, archive_error) =
                backup_history(root.as_deref(), client, &known_roots, &archives, &catalog);
            let archive_error = roots_error.or(preserve_error).or(archive_error);
            (records, archive_error, recent)
        });
        cx.spawn(async move |this, cx| {
            let (result, archive_error, recent) = task.await;
            let _ = this.update(cx, |app, cx| {
                if app.activity.request == request {
                    app.activity.entries = Some(result);
                    app.activity.archive_error = archive_error;
                    match recent {
                        Ok(entries) => app.activity.recent = entries,
                        Err(error) => app.activity.recent_error = Some(error.to_string()),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn restore_backup(&mut self, cx: &mut Context<Self>) {
        if self.addon_batch_running() {
            return;
        }
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            if self.activity.selected().is_some() && !self.activity.restoring {
                self.activity.confirmation = None;
                self.show_notification(
                    "This backup belongs to another game installation.",
                    true,
                    cx,
                );
            }
            return;
        };
        let Some(backup) = self.activity.selected() else {
            return;
        };
        if self.activity.restoring {
            return;
        }
        let path = backup.path.clone();
        let interrupted = backup.interrupted;
        let electron = backup.electron;
        let profile_switch = backup.transaction.profile_switch.clone();
        let data = self
            .preferences_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let catalog = self.catalog.clone();
        let prefs = self.preferences_path.clone();
        let name = backup.transaction.addon.clone();
        let task = cx.background_executor().spawn(async move {
            let result = if let (true, Some(operation)) = (interrupted, profile_switch) {
                let data = data.ok_or("Couldn't find the Astral settings folder.")?;
                crate::settings_backup::Store::open(&data, &root, client, &operation.account)
                    .and_then(|store| store.recover_package_switch(&path, &operation, &catalog))
                    .map_err(|error| error.to_string())
            } else if interrupted {
                installer::recover(&root, client, &path)
            } else if electron {
                installer::restore_electron(&root, client, &path, &catalog, prefs.as_deref())
            } else {
                installer::restore(&root, client, &path, prefs.as_deref())
            };
            record(
                prefs.as_deref(),
                Action {
                    root,
                    client,
                    name,
                    retry_source: None,
                    from: None,
                    to: None,
                    verb: if interrupted {
                        "Recovered".into()
                    } else {
                        "Restored".into()
                    },
                    error: result.as_ref().err().cloned(),
                    time: 0,
                },
            );
            result
        });
        self.await_addon_restore(task, interrupted, cx);
    }

    fn await_addon_restore(
        &mut self,
        task: gpui::Task<Result<(), String>>,
        interrupted: bool,
        cx: &mut Context<Self>,
    ) {
        self.activity.restoring = true;
        self.activity.outcome = None;
        if !interrupted {
            // main.jsx closes the ordinary confirmation before awaiting restore.
            // Keep the operation guard independently of the removed dialog.
            self.activity.confirmation = None;
        }
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.activity.restoring = false;
                if result.is_ok() {
                    app.activity.confirmation = None;
                    app.install_receipt = None;
                }
                if result.is_ok() || !interrupted {
                    // The source refreshes state after ordinary failures too:
                    // an interrupted swap may have left recovery work to list.
                    app.refresh_after_change(cx);
                }
                if interrupted {
                    app.activity.outcome =
                        Some(result.map(|_| "Interrupted operation recovered.".into()));
                } else {
                    match result {
                        Ok(()) => {
                            app.show_notification("Previous addon version restored.", false, cx)
                        }
                        Err(error) => app.show_notification(error, true, cx),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn backups_page(&self, palette: Palette, cx: &Context<Self>) -> impl IntoElement {
        let archives = self.activity_archives();
        self.activity.retain_control_focus(&archives);
        let mut page = div().flex().flex_col();
        if let Some(Ok(message)) = &self.activity.outcome {
            page = page.child(
                palette
                    .label(message.clone())
                    .text_color(rgb(palette.success)),
            );
        }
        if self.activity.confirmation.is_none()
            && let Some(Err(error)) = &self.activity.outcome
        {
            page = page.child(palette.label(error.clone()).text_color(rgb(palette.error)));
        }
        page = page.child(self.recent_actions_panel(palette, cx));
        if let Some(error) = &self.activity.archive_error {
            page = page.child(
                palette
                    .label(format!("Couldn't read earlier backups. {error}"))
                    .text_color(rgb(palette.error)),
            );
        }
        // Electron backups are part of the normal list; only the separate
        // Raider.IO archive gets a navigation row in the reference interface.
        for (index, (label, path)) in archives
            .into_iter()
            .filter(|(label, _)| *label == "Earlier Raider.IO backups")
            .enumerate()
        {
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(20.))
                    .py(px(24.))
                    .mb(px(30.))
                    .border_t_1()
                    .border_color(rgb(palette.text))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .child(
                        activity_link(
                            palette,
                            ("earlier-archive", index),
                            "Open archive",
                            &self
                                .activity
                                .control_focus(ButtonKey::Archive(path.clone()), cx),
                            move |cx| {
                                cx.open_with_system(&path);
                            },
                        )
                        .child(gpui::svg().path("arrow-up-right").size(px(14.))),
                    ),
            );
        }
        page = page.child(self.daily_history_panel(palette, cx));
        let count = self
            .activity
            .entries
            .as_ref()
            .and_then(|result| result.as_ref().ok());
        page = page.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mt(px(31.))
                .mb(px(18.))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_size(px(12.))
                        .child(count.map_or_else(
                            || "Backups".into(),
                            |records| format!("Backups ({})", records.len()),
                        )),
                )
                .child(
                    crate::outlined_button(
                        palette
                            .button("open-backup-folder", "")
                            .debug_selector(|| "activity-open-backups".into()),
                        palette,
                        &self.activity.control_focus(ButtonKey::OpenFolder, cx),
                        cx.listener(|app, _: &(), _, cx| app.open_backup_collection(cx)),
                    )
                    .child(icon("folder", 14., palette.muted))
                    .child("Open backups"),
                ),
        );
        let records = match &self.activity.entries {
            Some(Ok(records)) if !records.is_empty() => records,
            Some(Ok(_)) => {
                return page.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .py(px(70.))
                        .px(px(20.))
                        .border_t_1()
                        .border_color(palette.line)
                        .child(icon("history", 30., palette.muted))
                        .child(
                            div()
                                .mt(px(18.))
                                .mb(px(10.))
                                .text_size(px(27.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("No backups yet"),
                        )
                        .child(
                            crate::outlined_button(
                                palette
                                    .button("open-library", "Open library")
                                    .debug_selector(|| "activity-open-library".into()),
                                palette,
                                &self.activity.control_focus(ButtonKey::OpenLibrary, cx),
                                cx.listener(|app, _: &(), window, cx| {
                                    app.navigate(crate::Page::Library, window, cx);
                                }),
                            )
                            .mt(px(20.)),
                        ),
                );
            }
            Some(Err(error)) => {
                return page.child(palette.label(error.clone()).text_color(rgb(palette.error)));
            }
            None => return page.child(palette.label("Loading backups…")),
        };
        let mut rows = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(rgb(palette.text));
        for (index, backup) in records.iter().enumerate() {
            let record = &backup.transaction;
            let path = backup.path.clone();
            let show = backup.path.clone();
            let notes_path = backup.path.clone();
            let notes_open = self.activity.backup_notes.contains(&notes_path);
            let ordinary_update = !backup.interrupted
                && !backup.recovered
                && !record.restored
                && !record.removed
                && record.profile_switch.is_none();
            let detail = if ordinary_update {
                format!(
                    "{} → {} · {}",
                    record
                        .previous_version
                        .as_deref()
                        .unwrap_or(if record.previous.is_empty() {
                            "First install"
                        } else {
                            "Unknown version"
                        }),
                    record.release,
                    activity_time(record.created)
                )
            } else if backup.interrupted {
                "Interrupted operation".to_owned()
            } else if backup.recovered {
                format!("Recovered · {}", activity_time(record.created))
            } else if record.restored {
                format!("Restored · {}", activity_time(record.created))
            } else if record.removed {
                format!(
                    "Uninstalled {} · {}",
                    record.release,
                    activity_time(record.created)
                )
            } else {
                format!(
                    "Installed {} · {}",
                    record.release,
                    activity_time(record.created)
                )
            };
            rows =
                rows.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_between()
                        .gap(px(20.))
                        .py(px(25.))
                        .border_b_1()
                        .border_color(palette.line)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(20.))
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(40.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(palette.line)
                                        .child(icon(
                                            "history",
                                            18.,
                                            if backup.interrupted {
                                                palette.error
                                            } else {
                                                palette.muted
                                            },
                                        )),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_size(px(16.))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(if ordinary_update {
                                                    format!("{} updated", record.addon)
                                                } else {
                                                    record.addon.clone()
                                                }),
                                        )
                                        .child(palette.label(detail).mt(px(8.)).text_size(px(12.)))
                                        .when_some(
                                            (!record.notes.is_empty()).then_some(&record.notes),
                                            |body, notes| {
                                                body.child(activity_text_button(
                                                    palette,
                                                    ("backup-notes", index),
                                                    "Changelog",
                                                    cx.listener(move |app, _: &(), _, cx| {
                                                        if !app
                                                            .activity
                                                            .backup_notes
                                                            .remove(&notes_path)
                                                        {
                                                            app.activity
                                                                .backup_notes
                                                                .insert(notes_path.clone());
                                                        }
                                                        cx.notify();
                                                    }),
                                                ))
                                                .when(notes_open, |body| {
                                                    body.child(
                                                        palette
                                                            .label(notes.clone())
                                                            .mt_2()
                                                            .p_3()
                                                            .bg(rgb(palette.surface))
                                                            .border_1()
                                                            .border_color(palette.line)
                                                            .text_size(px(11.))
                                                            .line_height(px(20.)),
                                                    )
                                                })
                                            },
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .flex_shrink_0()
                                .gap_2()
                                .when(!ordinary_update, |actions| {
                                    actions.child(activity_link(
                                        palette,
                                        ("show-backup", index),
                                        "Show files",
                                        &self.activity.control_focus(
                                            ButtonKey::ShowFiles(backup.path.clone()),
                                            cx,
                                        ),
                                        move |cx| cx.reveal_path(&show),
                                    ))
                                })
                                .when(
                                    ordinary_update
                                        || backup.interrupted
                                        || (!backup.recovered
                                            && (!record.previous.is_empty() || backup.electron)),
                                    |actions| {
                                        actions.child(
                                            outlined_activity_text_button(
                                                palette,
                                                ("restore-backup", index),
                                                if backup.interrupted {
                                                    "Recover…"
                                                } else {
                                                    "Restore previous"
                                                },
                                                &self.activity.control_focus(
                                                    ButtonKey::Restore(backup.path.clone()),
                                                    cx,
                                                ),
                                                !self.addon_controls_busy()
                                                    && !self.profile_game_running(),
                                                cx.listener(move |app, _: &(), _, cx| {
                                                    if app.addon_controls_busy()
                                                        || app.profile_game_running()
                                                    {
                                                        return;
                                                    }
                                                    app.activity
                                                        .confirm_addon_restore(path.clone());
                                                    cx.notify();
                                                }),
                                            )
                                            .debug_selector({
                                                let path = backup.path.clone();
                                                move || {
                                                    format!(
                                                        "restore-backup:{}",
                                                        path.file_name()
                                                            .unwrap_or_default()
                                                            .to_string_lossy()
                                                    )
                                                }
                                            }),
                                        )
                                    },
                                ),
                        ),
                );
        }
        page.child(rows)
    }

    fn recent_actions_panel(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let mut panel = div().flex().flex_col();
        if let Some(error) = &self.activity.recent_error {
            panel = panel.child(
                palette
                    .label(format!("Couldn't read recent actions. {error}"))
                    .text_color(rgb(palette.error)),
            );
        }
        if self.activity.recent.is_empty() {
            return panel;
        }
        panel = panel.child(
            div()
                .mt(px(31.))
                .mb(px(18.))
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child("Recent actions"),
        );
        let mut rows = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(rgb(palette.text));
        let keys = retry_keys(&self.activity.recent);
        for (index, entry) in self.activity.recent.iter().enumerate() {
            let failed = entry.error.is_some();
            let detail = match (&entry.error, &entry.to) {
                (Some(error), _) => Some(error.clone()),
                (_, Some(to)) => Some(format!(
                    "{} → {to}",
                    entry.from.as_deref().unwrap_or("Not installed")
                )),
                _ => None,
            };
            let retry = entry
                .retry_source
                .as_ref()
                .filter(|id| {
                    failed
                        && self.catalog.iter().any(|source| {
                            &source.id == *id
                                && source.supports(entry.client)
                                && (source.bundled == Some(true)
                                    || (source.repo.is_some() && source.asset.is_some()))
                        })
                })
                .cloned();
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(20.))
                    .py(px(25.))
                    .border_b_1()
                    .border_color(palette.line)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(40.))
                            .flex_shrink_0()
                            .rounded_full()
                            .border_1()
                            .border_color(palette.line)
                            .child(icon(
                                if failed { "alert" } else { "check" },
                                18.,
                                palette.muted,
                            )),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(format!(
                                        "{} · {}",
                                        entry.name,
                                        if failed { "Failed" } else { &entry.verb }
                                    )),
                            )
                            .when_some(detail, |body, detail| {
                                body.child(palette.label(detail).text_size(px(12.)))
                            })
                            .child(palette.label(activity_time(entry.time)).text_size(px(12.))),
                    )
                    .when_some(retry, |row, source| {
                        let button = palette
                            .text_button(("retry-addon-action", index), "Retry")
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .py(px(5.))
                            .flex_shrink_0()
                            .rounded(px(4.));
                        row.child(if self.can_start_update_batch() {
                            crate::outlined_button(
                                button,
                                palette,
                                &self.activity.control_focus(keys[index].clone(), cx),
                                cx.listener(move |app, _: &(), _, cx| {
                                    app.update_one_addon(&source, cx)
                                }),
                            )
                        } else {
                            button.opacity(0.45).cursor_default()
                        })
                    }),
            );
        }
        panel.child(rows)
    }

    fn daily_history_panel(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let mut panel = div().flex().flex_col();
        if let Some(error) = &self.daily_history_error {
            panel = panel.child(
                palette
                    .label(format!("Couldn't read update history. {error}"))
                    .text_color(rgb(palette.error)),
            );
        }
        for group in crate::daily::grouped_history(&self.daily_history)
            .into_iter()
            .take(3)
        {
            let checked_at = group.checked_at();
            let when = if checked_at == 0 {
                "Earlier check".into()
            } else {
                activity_time(checked_at / 1000)
            };
            let mut row = div()
                .relative()
                .flex()
                .flex_col()
                .py(px(24.))
                .mb(px(30.))
                .border_t_1()
                .border_color(rgb(palette.text))
                .child(
                    div()
                        .border_b_1()
                        .border_color(palette.line)
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full(),
                )
                .child(div().font_weight(gpui::FontWeight(650.)).mb(px(10.)).child(
                    crate::typography::tracked(
                        format!("Daily update check · {when}"),
                        19.,
                        1.2,
                        -0.6,
                    ),
                ))
                .child(
                    palette
                        .label(format!(
                            "{} updated · {} need attention{}",
                            group.updated(),
                            group.attention(),
                            if group.deferred() {
                                " · Waiting for WoW to close"
                            } else {
                                ""
                            }
                        ))
                        .text_size(px(12.))
                        .line_height(px(21.6)),
                );
            for entry in group.entries {
                let run = &entry.run;
                let key_time = run.check_id.filter(|id| *id > 0).unwrap_or(run.checked_at);
                for (index, result) in run.results().iter().enumerate() {
                    if result.status == crate::batch::DailyStatus::Current {
                        continue;
                    }
                    let key = (key_time, entry.client.index(), index);
                    let expanded = self.activity.release_notes.contains(&key);
                    row = row.child(
                        activity_text_button(
                            palette,
                            SharedString::from(format!("daily-notes-{key:?}")),
                            "",
                            cx.listener(move |app, _: &(), _, cx| {
                                if !app.activity.release_notes.remove(&key) {
                                    app.activity.release_notes.insert(key);
                                }
                                cx.notify();
                            }),
                        )
                        .py(px(13.))
                        .my(px(10.))
                        .text_size(px(12.))
                        .line_height(px(21.6))
                        .child(if expanded { "▾" } else { "▸" })
                        .child(result.summary(entry.client)),
                    );
                    if expanded && !result.notes.is_empty() {
                        row = row.child(
                            div()
                                .id(SharedString::from(format!("daily-notes-scroll-{key:?}")))
                                .max_h(px(200.))
                                .overflow_y_scroll()
                                .child(
                                    palette
                                        .label(result.notes.clone())
                                        .font_family("monospace")
                                        .text_size(px(11.))
                                        .line_height(px(19.8))
                                        .p_3()
                                        .bg(rgb(palette.surface)),
                                ),
                        );
                    }
                }
            }
            panel = panel.child(row);
        }
        panel
    }

    pub fn restore_dialog(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let mut modal = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center();
        let Some(backup) = self.activity.selected() else {
            return modal;
        };
        let Some(focus) = &self.activity.confirmation_focus else {
            return modal;
        };
        let record = &backup.transaction;
        let mut body = crate::account::dialog_panel("addon-restore-dialog", palette)
            .child(icon("history", 26., palette.accent))
            .child(div().mt(px(20.)).text_size(px(24.)).line_height(px(29.)).font_weight(gpui::FontWeight::MEDIUM)
                .child(format!("{} {}?", if backup.interrupted { "Recover" } else { "Restore" }, record.addon)))
            .child(palette.label(if backup.interrupted && record.profile_switch.is_some() {
                "Astral checks whether the profile was saved and recovers the settings and addon files together. Close WoW to continue.".to_owned()
            } else if backup.interrupted {
                "Astral puts the files from before the interruption back in AddOns and keeps the interrupted files in the backup. Close WoW to continue.".to_owned()
            } else if record.previous.is_empty() {
                "Undo the first install. Your saved settings stay in place.".to_owned()
            } else if let Some(date) = backup_date(record.created) {
                format!("Restore the previous addon files from {date}. Your saved settings stay in place.")
            } else {
                "Restore the previous addon files. Your saved settings stay in place.".to_owned()
            }).mt(px(13.)).text_size(px(12.)).line_height(px(23.)));
        let removed: Vec<_> = record
            .folders
            .iter()
            .filter(|name| !record.previous.contains(name))
            .cloned()
            .collect();
        if !removed.is_empty() {
            body = body.child(palette.label(format!(
                "Astral will move these folders out of AddOns: {}.",
                removed.join(", ")
            )));
        }
        if let Some(Err(error)) = &self.activity.outcome {
            body = body.child(palette.label(error.clone()).text_color(rgb(palette.error)));
        }
        let mut actions = div().mt(px(25.)).flex().justify_end().gap(px(9.));
        if !self.activity.restoring {
            actions = actions.child(crate::outlined_button(
                palette
                    .button("cancel-restore", "Cancel")
                    .debug_selector(|| "cancel-addon-restore".into())
                    .track_focus(&focus.cancel),
                palette,
                &focus.cancel,
                cx.listener(|app, _: &(), _, cx| {
                    app.activity.dismiss_confirmation();
                    cx.notify();
                }),
            ));
        }
        let restore = if self.activity.restoring {
            palette
                .primary_button(
                    "confirm-restore",
                    if backup.interrupted {
                        "Recovering…"
                    } else {
                        "Restoring…"
                    },
                )
                .cursor_default()
                .opacity(0.6)
                .track_focus(&focus.restore)
        } else {
            crate::outlined_button(
                palette
                    .primary_button(
                        "confirm-restore",
                        if backup.interrupted {
                            "Recover addon"
                        } else {
                            "Restore previous"
                        },
                    )
                    .track_focus(&focus.restore),
                palette,
                &focus.restore,
                cx.listener(|app, _: &(), _, cx| app.restore_backup(cx)),
            )
        };
        modal = modal.child(
            div()
                .id("addon-restore-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .bg(gpui::rgba(0x050b08b3))
                .occlude()
                .on_click(cx.listener(|app, _, _, cx| {
                    if app.activity.dismiss_confirmation() {
                        cx.notify();
                    }
                })),
        );
        modal.child(body.child(actions.child(restore)))
    }
}

pub(crate) fn activity_time(created: u64) -> String {
    backup_date(created).unwrap_or_else(|| age(created))
}

fn backup_date(created: u64) -> Option<String> {
    let date = time::OffsetDateTime::from_unix_timestamp(created.try_into().ok()?).ok()?;
    if let Some(formatted) = crate::local_time::format_activity_date(date) {
        return Some(formatted);
    }
    let local =
        crate::local_time::offset_at(date).and_then(|offset| date.checked_to_offset(offset));
    let date = local.unwrap_or(date);
    let month = date.month().to_string();
    Some(format!(
        "{} {}, {:02}:{:02}{}",
        &month[..3],
        date.day(),
        date.hour(),
        date.minute(),
        if local.is_some() { "" } else { " UTC" },
    ))
}

pub(crate) fn age(created: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let minutes = now.saturating_sub(created) / 60;
    match minutes {
        0 => "Just now".into(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} hr ago", minutes / 60),
        _ => format!("{} d ago", minutes / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_both_earlier_astral_archives() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        for name in [
            "astral-companion/backups",
            "Astral/backups",
            "Astral RaiderIO/Backups",
        ] {
            fs::create_dir_all(temp.path().join(name))?;
        }
        let archives = archive_locations(Some(temp.path()), Some(temp.path()));
        assert_eq!(archives.len(), 3);
        assert_eq!(
            archives
                .iter()
                .filter(|(label, _)| *label == "Earlier Astral backups")
                .count(),
            2
        );
        #[cfg(unix)]
        {
            fs::remove_dir(temp.path().join("Astral/backups"))?;
            std::os::unix::fs::symlink(
                temp.path().join("astral-companion/backups"),
                temp.path().join("Astral/backups"),
            )?;
            assert_eq!(
                archive_locations(Some(temp.path()), Some(temp.path())).len(),
                2
            );
        }
        Ok(())
    }

    #[test]
    fn earlier_actions_read_actual_electron_history_without_changing_it() -> io::Result<()> {
        let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") else {
            return Ok(());
        };
        let temp = tempfile::tempdir()?;
        let data = temp.path().join("astral-companion");
        let retail = temp.path().join("_retail_");
        let forever = temp.path().join("_forever_");
        fs::create_dir_all(&data)?;
        fs::create_dir_all(&retail)?;
        fs::create_dir_all(&forever)?;
        let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../desktop/addon-activity.cjs");
        let script = r#"
            const [module, data, retail, forever] = process.argv.slice(1);
            const {track} = require(module);
            (async () => {
                const addon = name => ({id:'bigwigs',name,version:'1'});
                await track(data,retail,addon('Updated addon'),'update',async()=>({version:'2'}));
                await track(data,retail,addon('Failed addon'),'update',async()=>{throw Error('Download failed');}).catch(()=>{});
                await track(data,retail,addon('Restored addon'),'restore',async()=>({version:'0'}));
                await track(data,retail,addon('Current addon'),'update',async()=>({unchanged:true,version:'1'}));
                await track(data,forever,addon('Other client'),'update',async()=>({version:'3'}));
            })().catch(error=>{console.error(error);process.exitCode=1;});
        "#;
        let output = std::process::Command::new(node)
            .arg("-e")
            .arg(script)
            .arg(module)
            .arg(&data)
            .arg(&retail)
            .arg(&forever)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let history = data.join("addon-activity.json");
        let original = fs::read(&history)?;
        // Failed-only operations may have a history file but no backups directory.
        assert!(!data.join("backups").exists());
        let entries = earlier_actions(&[data.clone(), data.clone()], &retail, Client::Retail)?;
        assert_eq!(entries.len(), 4);
        let find = |name| entries.iter().find(|entry| entry.name == name).unwrap();
        assert_eq!(find("Updated addon").verb, "Updated");
        assert_eq!(find("Updated addon").to.as_deref(), Some("2"));
        assert_eq!(
            find("Failed addon").error.as_deref(),
            Some("Download failed")
        );
        assert_eq!(
            find("Failed addon").retry_source.as_deref(),
            Some("bigwigs")
        );
        assert_eq!(find("Restored addon").verb, "Restored");
        assert!(find("Restored addon").retry_source.is_none());
        assert_eq!(find("Current addon").verb, "Already up to date");
        assert!(
            entries
                .iter()
                .all(|entry| entry.time > 0 && entry.client == Client::Retail)
        );
        assert_eq!(
            earlier_actions(&[data.clone()], &forever, Client::Forever)?.len(),
            1
        );
        assert_eq!(fs::read(history)?, original);
        Ok(())
    }

    #[test]
    fn recent_actions_persist_failures_and_bound_history() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("preferences.json");
        for index in 0..101 {
            record(
                Some(&path),
                Action {
                    root: directory.path().to_path_buf(),
                    client: Client::Retail,
                    name: format!("Addon {index}"),
                    retry_source: Some("bigwigs".into()),
                    from: Some("1".into()),
                    to: Some("2".into()),
                    verb: "Updated".into(),
                    error: (index == 100).then(|| "Download failed".into()),
                    time: 0,
                },
            );
        }
        let entries = recent_actions(&path)?;
        assert_eq!(entries.len(), 100);
        assert_eq!(entries[0].name, "Addon 100");
        assert_eq!(entries[0].error.as_deref(), Some("Download failed"));
        assert_eq!(entries[0].retry_source.as_deref(), Some("bigwigs"));
        assert_eq!(entries[99].name, "Addon 1");
        Ok(())
    }
}

#[cfg(all(test, feature = "headless-ui-tests"))]
mod ui_tests {
    use super::*;
    use crate::{Page, ui_tests::press};
    use gpui::{Modifiers, TestAppContext, size};

    fn backup(path: PathBuf, name: &str) -> installer::Backup {
        installer::Backup {
            path,
            transaction: installer::Transaction {
                addon: name.into(),
                release: "2".into(),
                previous_version: Some("1".into()),
                notes: String::new(),
                created: 1_700_000_000,
                created_ms: None,
                folders: vec![name.into()],
                previous: vec![name.into()],
                restored: false,
                removed: false,
                profile_switch: None,
            },
            interrupted: false,
            recovered: false,
            electron: false,
        }
    }

    #[gpui::test]
    fn ordinary_restore_closes_before_completion_and_reports_result_in_toast(
        cx: &mut TestAppContext,
    ) {
        for light in [false, true] {
            for succeeds in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("Fixture");
                let (view, cx) = cx.add_window_view(|_, cx| {
                    let mut app = Astral::new(
                        [None, None],
                        Some(directory.path()),
                        Some(Client::Retail),
                        light,
                        cx,
                    );
                    app.page = Page::Backups;
                    app.activity.entries = Some(Ok(vec![backup(path.clone(), "Fixture")]));
                    app.activity.confirm_addon_restore(path.clone());
                    app
                });
                cx.simulate_resize(size(px(1250.), px(1000.)));
                cx.run_until_parked();
                assert!(cx.debug_bounds("cancel-addon-restore").is_some());
                let (send, receive) = async_channel::bounded(1);
                view.update(cx, |app, cx| {
                    let task = cx
                        .background_executor()
                        .spawn(async move { receive.recv().await.unwrap() });
                    app.await_addon_restore(task, false, cx);
                });
                cx.run_until_parked();
                assert!(cx.debug_bounds("notification").is_none());
                view.update_in(cx, |app, window, cx| {
                    assert!(app.activity.operation_running());
                    assert!(app.activity.has_pending_operation());
                    assert!(!app.activity.has_confirmation());
                    assert!(app.activity.confirmation_focus.is_none());
                    assert!(app.addon_controls_busy());
                    assert!(!app.can_start_update_batch());
                    // Navigation must not drop ownership while files are changing.
                    app.page = Page::Library;
                    app.root_focus.focus(window);
                    app.activity.confirm_addon_restore(path.clone());
                    assert!(!app.activity.has_confirmation());
                    cx.notify();
                });
                send.try_send(if succeeds {
                    Ok(())
                } else {
                    Err("Fixture restore failed.".into())
                })
                .unwrap();
                cx.run_until_parked();
                assert!(cx.debug_bounds("notification").is_some());
                view.read_with(cx, |app, _| {
                    assert!(!app.activity.operation_running());
                    assert!(!app.activity.has_pending_operation());
                    assert!(app.activity.outcome.is_none());
                });
                assert!(!path.exists());
            }
        }
    }

    #[gpui::test]
    fn activity_restore_focus_tracks_backup_and_busy_links_do_not_open_confirmation(
        cx: &mut TestAppContext,
    ) {
        for light in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let first = directory.path().join("FirstFixture");
            let second = directory.path().join("SecondFixture");
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut app = Astral::new(
                    [None, None],
                    Some(directory.path()),
                    Some(Client::Retail),
                    light,
                    cx,
                );
                app.detected_roots[0] = Some(directory.path().to_owned());
                app.page = Page::Backups;
                app.activity.entries = Some(Ok(vec![
                    backup(first.clone(), "First"),
                    backup(second.clone(), "Second"),
                ]));
                app
            });
            cx.simulate_resize(size(px(1250.), px(1000.)));
            cx.run_until_parked();
            let before = cx.debug_bounds("restore-backup:SecondFixture").unwrap();
            view.update_in(cx, |app, window, cx| {
                app.activity
                    .control_focus(ButtonKey::Restore(second.clone()), cx)
                    .focus(window)
            });
            cx.run_until_parked();
            assert_eq!(
                before,
                cx.debug_bounds("restore-backup:SecondFixture").unwrap()
            );
            view.update_in(cx, |app, _, cx| {
                app.activity
                    .entries
                    .as_mut()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .reverse();
                cx.notify();
            });
            cx.run_until_parked();
            view.update_in(cx, |app, window, cx| {
                assert!(
                    app.activity
                        .control_focus(ButtonKey::Restore(second.clone()), cx)
                        .is_focused(window)
                )
            });
            press(cx, "enter");
            assert_eq!(
                view.read_with(cx, |app, _| app.activity.confirmation.clone()),
                Some(second.clone())
            );
            view.update_in(cx, |app, window, cx| {
                assert!(
                    app.activity
                        .control_focus(ButtonKey::Restore(second.clone()), cx)
                        .is_focused(window)
                )
            });
            // main.jsx does not autofocus/trap this modal: the next underlying
            // backup action precedes Cancel in ordinary document Tab order.
            press(cx, "tab");
            view.update_in(cx, |app, window, cx| {
                assert!(
                    app.activity
                        .control_focus(ButtonKey::Restore(first.clone()), cx)
                        .is_focused(window)
                )
            });
            press(cx, "tab");
            view.update_in(cx, |app, window, _| {
                assert!(
                    app.activity
                        .confirmation_focus
                        .as_ref()
                        .unwrap()
                        .cancel
                        .is_focused(window)
                )
            });
            press(cx, "shift-tab");
            view.update_in(cx, |app, window, cx| {
                assert!(
                    app.activity
                        .control_focus(ButtonKey::Restore(first.clone()), cx)
                        .is_focused(window)
                )
            });
            press(cx, "tab");
            press(cx, "enter");
            assert!(view.read_with(cx, |app, _| app.activity.confirmation.is_none()));
            view.update_in(cx, |app, window, cx| {
                assert!(app.root_focus.is_focused(window));
                app.activity.confirm_addon_restore(second.clone());
                app.brand_focus.focus(window);
                cx.notify();
            });
            cx.run_until_parked();
            view.update_in(cx, |app, window, cx| {
                assert!(app.brand_focus.is_focused(window));
                app.activity.dismiss_confirmation();
                cx.notify();
            });
            cx.run_until_parked();
            view.update_in(cx, |app, window, cx| {
                assert!(app.brand_focus.is_focused(window));
                app.checking = true;
                app.brand_focus.focus(window);
                cx.notify();
            });
            cx.run_until_parked();
            let blocked = cx.debug_bounds("restore-backup:SecondFixture").unwrap();
            cx.simulate_click(blocked.center(), Modifiers::default());
            cx.run_until_parked();
            assert!(view.read_with(cx, |app, _| app.activity.confirmation.is_none()));
            view.update_in(cx, |app, _, cx| {
                app.checking = false;
                app.activity
                    .entries
                    .as_mut()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .retain(|backup| backup.path != second);
                cx.notify();
            });
            cx.run_until_parked();
            assert!(view.read_with(cx, |app, _| {
                !app.activity
                    .button_focus
                    .borrow()
                    .contains_key(&ButtonKey::Restore(second.clone()))
            }));
        }
    }
}

#[cfg(all(test, feature = "headless-ui-tests"))]
mod navigation_ui_tests {
    use super::*;
    use crate::{Page, ui_tests::press};
    use gpui::{TestAppContext, size};

    #[gpui::test]
    fn empty_activity_tab_reaches_library_and_retries_keep_distinct_focus(cx: &mut TestAppContext) {
        for light in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut app = Astral::new(
                    [None, None],
                    Some(directory.path()),
                    Some(Client::Retail),
                    light,
                    cx,
                );
                app.detected_roots[0] = Some(directory.path().to_owned());
                app.page = Page::Backups;
                app.activity.entries = Some(Ok(vec![]));
                app
            });
            cx.simulate_resize(size(px(1250.), px(1000.)));
            cx.run_until_parked();
            let empty = cx.debug_bounds("activity-open-library").unwrap();
            view.update_in(cx, |app, window, cx| {
                app.activity
                    .control_focus(ButtonKey::OpenFolder, cx)
                    .focus(window)
            });
            press(cx, "tab");
            view.update_in(cx, |app, window, cx| {
                assert!(
                    app.activity
                        .control_focus(ButtonKey::OpenLibrary, cx)
                        .is_focused(window)
                )
            });
            assert_eq!(empty, cx.debug_bounds("activity-open-library").unwrap());
            press(cx, "enter");
            assert!(view.read_with(cx, |app, _| app.page == Page::Library));
            view.update_in(cx, |app, _, cx| {
                app.page = Page::Backups;
                let source = app
                    .catalog
                    .iter()
                    .find(|source| {
                        source.supports(Client::Retail)
                            && source.repo.is_some()
                            && source.asset.is_some()
                    })
                    .unwrap()
                    .id
                    .clone();
                app.activity.recent = [1, 2]
                    .into_iter()
                    .map(|time| Action {
                        root: directory.path().to_owned(),
                        client: Client::Retail,
                        name: "Fixture addon".into(),
                        retry_source: Some(source.clone()),
                        from: Some("1".into()),
                        to: Some("2".into()),
                        verb: "Updated".into(),
                        error: Some("Fixture update failed".into()),
                        time,
                    })
                    .collect();
                cx.notify();
            });
            cx.run_until_parked();
            let keys = view.read_with(cx, |app, _| retry_keys(&app.activity.recent));
            view.update_in(cx, |app, window, cx| {
                app.activity
                    .control_focus(keys[0].clone(), cx)
                    .focus(window)
            });
            press(cx, "tab");
            view.update_in(cx, |app, window, cx| {
                assert!(
                    app.activity
                        .control_focus(keys[1].clone(), cx)
                        .is_focused(window)
                );
                assert!(
                    !app.activity
                        .control_focus(keys[0].clone(), cx)
                        .is_focused(window)
                );
                app.activity.recent.reverse();
                cx.notify();
            });
            cx.run_until_parked();
            view.update_in(cx, |app, window, cx| {
                assert!(
                    app.activity
                        .control_focus(keys[1].clone(), cx)
                        .is_focused(window)
                )
            });
            // Never activate Retry: it would start an addon operation.
        }
    }
}

#[cfg(all(test, feature = "headless-ui-tests"))]
mod global_history_ui_tests {
    use super::*;
    use crate::{Page, ui_tests::press};
    use gpui::{TestAppContext, size};

    #[gpui::test]
    fn activity_reads_central_native_backups_after_the_game_folder_is_removed(
        cx: &mut TestAppContext,
    ) {
        for light in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("game/_retail_");
            fs::create_dir_all(root.join("Interface/AddOns")).unwrap();
            let root = root.canonicalize().unwrap();
            let data = temp.path().join("app");
            let backup = data.join("backups/native-1c46cb04-349d-4a58-a620-676144b42c12");
            fs::create_dir_all(backup.join("previous/Fixture")).unwrap();
            fs::write(backup.join("previous/Fixture/old.lua"), "preserved addon").unwrap();
            preferences::save_json(
                &backup.join("native-manifest.json"),
                &serde_json::json!({
                    "root": root,
                    "client": "retail",
                    "journal": root.join("Interface/.astral-native-backups/release-fixture"),
                    "transaction": {
                        "addon": "Fixture", "release": "2", "created": 1_700_000_000,
                        "folders": ["Fixture"], "previous": ["Fixture"]
                    }
                }),
            )
            .unwrap();
            fs::remove_dir_all(&root).unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut app =
                    Astral::new([None, None], Some(&data), Some(Client::Retail), light, cx);
                app.page = Page::Backups;
                app.load_backups(cx);
                app
            });
            cx.simulate_resize(size(px(1250.), px(1000.)));
            cx.run_until_parked();
            view.read_with(cx, |app, _| {
                assert!(app.activity.archive_error.is_none());
                let records = app.activity.entries.as_ref().unwrap().as_ref().unwrap();
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].path, backup.canonicalize().unwrap());
                assert_eq!(records[0].transaction.addon, "Fixture");
            });
            assert!(
                cx.debug_bounds("restore-backup:native-1c46cb04-349d-4a58-a620-676144b42c12")
                    .is_some()
            );
            assert_eq!(
                fs::read_to_string(backup.join("previous/Fixture/old.lua")).unwrap(),
                "preserved addon"
            );
        }
    }

    #[gpui::test]
    fn rootless_activity_finishes_loading_and_keeps_folder_and_library_actions(
        cx: &mut TestAppContext,
    ) {
        for light in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut app = Astral::new(
                    [None, None],
                    Some(directory.path()),
                    Some(Client::Retail),
                    light,
                    cx,
                );
                app.page = Page::Backups;
                app.load_backups(cx);
                app
            });
            cx.simulate_resize(size(px(1250.), px(1000.)));
            cx.run_until_parked();
            assert!(view.read_with(
                cx,
                |app, _| matches!(&app.activity.entries, Some(Ok(entries)) if entries.is_empty())
            ));
            assert!(cx.debug_bounds("activity-open-backups").is_some());
            assert!(cx.debug_bounds("activity-open-library").is_some());
            view.update_in(cx, |app, window, cx| {
                app.activity
                    .control_focus(ButtonKey::OpenFolder, cx)
                    .focus(window)
            });
            press(cx, "tab");
            view.update_in(cx, |app, window, cx| {
                assert!(
                    app.activity
                        .control_focus(ButtonKey::OpenLibrary, cx)
                        .is_focused(window)
                )
            });
            press(cx, "enter");
            assert!(view.read_with(cx, |app, _| app.page == Page::Library));
            let backup_path = directory.path().join("global-fixture");
            view.update_in(cx, |app, _, cx| {
                app.page = Page::Backups;
                app.activity.entries = Some(Ok(vec![installer::Backup {
                    path: backup_path.clone(),
                    transaction: serde_json::from_value(serde_json::json!({
                        "addon": "Fixture", "release": "2", "created": 1_700_000_000,
                        "folders": ["Fixture"], "previous": ["Fixture"]
                    }))
                    .unwrap(),
                    interrupted: false,
                    recovered: false,
                    electron: true,
                }]));
                app.activity.confirm_addon_restore(backup_path.clone());
                cx.notify();
            });
            cx.run_until_parked();
            view.update_in(cx, |app, window, _| {
                app.activity
                    .confirmation_focus
                    .as_ref()
                    .unwrap()
                    .restore
                    .focus(window)
            });
            press(cx, "enter");
            assert!(
                view.read_with(cx, |app, _| app.activity.confirmation.is_none()
                    && !app.activity.restoring)
            );
            assert!(cx.debug_bounds("notification").is_some());
            assert!(!backup_path.exists());
            // GPUI's fake platform cannot open a file manager. Verify folder
            // creation separately without calling the OS opener.
            let prefs = directory.path().join("preferences.json");
            assert_eq!(
                crate::backup_collection::update(prefs.clone(), vec![], true).unwrap(),
                directory.path().join("backups").canonicalize().unwrap()
            );
            assert!(directory.path().join("backups").is_dir());
        }
    }
}

#[cfg(test)]
mod global_history_tests {
    use super::*;

    #[test]
    fn global_history_merges_known_clients_and_archives_but_selected_history_stays_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let retail = temp.path().join("_retail_");
        let forever = temp.path().join("_forever_");
        let archive = temp.path().join("archive");
        let catalog = vec![
            serde_json::from_value(serde_json::json!({
                "id": "keyui", "name": "KeyUI", "folder": "KeyUI"
            }))
            .unwrap(),
        ];
        for (index, (root, flavor)) in [(&retail, "retail"), (&forever, "forever")]
            .into_iter()
            .enumerate()
        {
            fs::create_dir_all(root.join("Interface/AddOns/KeyUI")).unwrap();
            fs::write(
                root.join("Interface/AddOns/KeyUI/unchanged.lua"),
                "original",
            )
            .unwrap();
            let native = root.join("Interface/.astral-native-backups/native-fixture");
            fs::create_dir_all(&native).unwrap();
            preferences::save_json(
                &native.join("manifest.json"),
                &serde_json::json!({
                    "addon": "KeyUI", "release": "2", "created": 1_793_959_050,
                    "folders": ["KeyUI"], "previous": ["KeyUI"]
                }),
            )
            .unwrap();
            let id = format!("{}-keyui", 1_793_959_050_000_u64 + index as u64);
            let legacy = archive.join(&id);
            fs::create_dir_all(&legacy).unwrap();
            preferences::save_json(
                &legacy.join("manifest.json"),
                &serde_json::json!({
                    "id": id, "root": root, "flavor": flavor, "addonId": "keyui",
                    "name": "KeyUI", "release": "2", "version": "1",
                    "folders": ["KeyUI"], "previousFolders": ["KeyUI"]
                }),
            )
            .unwrap();
        }
        let known = vec![
            (Client::Retail, retail.clone()),
            (Client::Forever, forever.clone()),
        ];
        let archives = vec![
            ("Earlier Astral backups", archive.clone()),
            ("Earlier Astral backups", archive),
        ];
        let (all, error) = backup_history(None, Client::Retail, &known, &archives, &catalog);
        assert!(error.is_none());
        assert_eq!(all.unwrap().len(), 4);
        let (selected, error) =
            backup_history(Some(&retail), Client::Retail, &known, &archives, &catalog);
        assert!(error.is_none());
        assert_eq!(selected.unwrap().len(), 2);
        for root in [&retail, &forever] {
            assert_eq!(
                fs::read_to_string(root.join("Interface/AddOns/KeyUI/unchanged.lua")).unwrap(),
                "original"
            );
        }
    }
}

#[cfg(all(test, feature = "headless-ui-tests"))]
mod registered_history_ui_tests {
    use super::*;
    use crate::Page;
    use gpui::{TestAppContext, size};

    #[gpui::test]
    fn rootless_activity_loads_native_backups_from_a_previously_registered_installation(
        cx: &mut TestAppContext,
    ) {
        for light in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let prefs = temp.path().join("preferences.json");
            let root = temp.path().join("game/_retail_");
            fs::create_dir_all(root.join("Interface/AddOns/Fixture")).unwrap();
            let backup = root.join("Interface/.astral-native-backups/persisted-fixture");
            fs::create_dir_all(&backup).unwrap();
            preferences::save_json(
                &backup.join("manifest.json"),
                &serde_json::json!({
                    "addon": "Fixture", "release": "2", "created": 1_700_000_000,
                    "folders": ["Fixture"], "previous": ["Fixture"]
                }),
            )
            .unwrap();
            crate::backup_roots::remember(&prefs, &[(Client::Retail, root.clone())]).unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut app = Astral::new(
                    [None, None],
                    Some(temp.path()),
                    Some(Client::Retail),
                    light,
                    cx,
                );
                app.page = Page::Backups;
                assert!(app.root(Client::Retail).is_none());
                app.load_backups(cx);
                app
            });
            cx.simulate_resize(size(px(1250.), px(1000.)));
            cx.run_until_parked();
            let actual = view.read_with(cx, |app, _| {
                app.activity.entries.as_ref().map(|entries| {
                    entries
                        .as_ref()
                        .map(|entries| {
                            entries
                                .iter()
                                .map(|backup| backup.path.clone())
                                .collect::<Vec<_>>()
                        })
                        .map_err(Clone::clone)
                })
            });
            assert_eq!(actual, Some(Ok(vec![backup.canonicalize().unwrap()])));
            assert!(
                cx.debug_bounds("restore-backup:persisted-fixture")
                    .is_some()
            );
            assert!(backup.join("manifest.json").is_file());
            assert!(!root.join("Interface/AddOns/Fixture/manifest.json").exists());
        }
    }

    #[gpui::test]
    fn folder_action_prepares_the_real_collection_then_calls_the_opener(cx: &mut TestAppContext) {
        for light in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let prefs = temp.path().join("preferences.json");
            let root = temp.path().join("game/_retail_");
            fs::create_dir_all(root.join("Interface/AddOns/Fixture")).unwrap();
            let backup = root.join("Interface/.astral-native-backups/catalog-fixture");
            fs::create_dir_all(&backup).unwrap();
            let manifest = backup.join("manifest.json");
            preferences::save_json(
                &manifest,
                &serde_json::json!({
                    "addon": "Fixture", "release": "2", "created": 1_700_000_000,
                    "folders": ["Fixture"], "previous": ["Fixture"]
                }),
            )
            .unwrap();
            let original = fs::read(&manifest).unwrap();
            crate::backup_roots::remember(&prefs, &[(Client::Retail, root)]).unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut app = Astral::new(
                    [None, None],
                    Some(temp.path()),
                    Some(Client::Retail),
                    light,
                    cx,
                );
                app.page = Page::Backups;
                app
            });
            let opened = std::rc::Rc::new(std::cell::RefCell::new(None));
            view.update(cx, |app, cx| {
                let task = app.backup_collection_task(cx);
                let opened = opened.clone();
                app.await_backup_collection(
                    task,
                    move |path, _| *opened.borrow_mut() = Some(path),
                    cx,
                );
                assert!(app.activity.opening_collection);
            });
            cx.run_until_parked();
            let folder = opened.borrow().clone().unwrap();
            assert_eq!(folder, temp.path().join("backups").canonicalize().unwrap());
            assert!(fs::read_dir(&folder).unwrap().next().is_some());
            assert!(view.read_with(cx, |app, _| !app.activity.opening_collection));
            assert_eq!(fs::read(&manifest).unwrap(), original);
            fs::write(temp.path().join("backup-collection.json"), "broken").unwrap();
            *opened.borrow_mut() = None;
            view.update(cx, |app, cx| {
                let task = app.backup_collection_task(cx);
                let opened = opened.clone();
                app.await_backup_collection(
                    task,
                    move |path, _| *opened.borrow_mut() = Some(path),
                    cx,
                );
            });
            cx.run_until_parked();
            assert_eq!(opened.borrow().as_ref(), Some(&folder));
            assert_eq!(
                fs::read_to_string(temp.path().join("backup-collection.json")).unwrap(),
                "broken"
            );
            assert_eq!(fs::read(&manifest).unwrap(), original);
            // The real button also reports preparation errors without calling an OS opener.
            view.update_in(cx, |app, window, cx| {
                app.preferences_path = None;
                app.activity
                    .control_focus(ButtonKey::OpenFolder, cx)
                    .focus(window);
                cx.notify();
            });
            crate::ui_tests::press(cx, "enter");
            assert!(cx.debug_bounds("notification").is_some());
            assert!(view.read_with(cx, |app, _| !app.activity.opening_collection));
        }
    }
}
