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
    outcome: Option<Result<String, String>>,
    release_notes: std::collections::HashSet<(u64, usize, usize)>,
    backup_notes: std::collections::HashSet<PathBuf>,
    archive_error: Option<String>,
    recent: Vec<Action>,
    recent_error: Option<String>,
}

struct RestoreFocus {
    cancel: FocusHandle,
    restore: FocusHandle,
    previous: Option<FocusHandle>,
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

fn activity_button(
    button: Stateful<gpui::Div>,
    palette: Palette,
    activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    crate::keyboard_button(button, palette, activate)
}

fn activity_text_button(
    palette: Palette,
    id: impl Into<ElementId>,
    label: &str,
    activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    activity_button(
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
            .child(label.to_owned()),
        palette,
        activate,
    )
}

fn activity_link(
    palette: Palette,
    id: impl Into<ElementId>,
    label: &str,
    activate: impl Fn(&mut App) + 'static,
) -> Stateful<gpui::Div> {
    activity_text_button(palette, id, label, move |_, _, cx| activate(cx))
}

impl State {
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
            let focus = self
                .activity
                .confirmation_focus
                .get_or_insert_with(|| RestoreFocus {
                    cancel: cx.focus_handle().tab_stop(true),
                    restore: cx.focus_handle().tab_stop(true),
                    previous: window.focused(cx),
                });
            if self.activity.restoring {
                focus.restore.focus(window);
            } else if !focus.cancel.is_focused(window) && !focus.restore.is_focused(window) {
                focus.cancel.focus(window);
            }
        } else if let Some(focus) = self.activity.confirmation_focus.take() {
            focus
                .previous
                .as_ref()
                .unwrap_or(&self.root_focus)
                .focus(window);
        }
    }

    fn activity_archives(&self) -> Vec<(&'static str, PathBuf)> {
        if self.fixture {
            return Vec::new();
        }
        earlier_archives(self.preferences_path.as_deref())
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
        let Some(root) = self.root(client).cloned() else {
            return;
        };
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
                    let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
                    entries.retain(|entry| entry.client == client && entry.root == canonical);
                    entries
                });
            match earlier_actions(&earlier_activity, &root, client) {
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
            let mut archive_error = None;
            let records = installer::list(&root, client).map(|mut records| {
                for (_, archive) in archives
                    .into_iter()
                    .filter(|(label, _)| *label == "Earlier Astral backups")
                {
                    match installer::list_electron(&archive, &root, client, &catalog) {
                        Ok(earlier) => records.extend(earlier),
                        Err(error) => archive_error = Some(error),
                    }
                }
                installer::sort_backups(&mut records);
                records
            });
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
        self.activity.restoring = true;
        self.activity.outcome = None;
        let task = cx.background_executor().spawn(async move {
            let result = if let (true, Some(operation)) = (interrupted, profile_switch) {
                let data = data.ok_or("Couldn't find the Astral settings folder.")?;
                crate::settings_backup::Store::open(&data, &root, client, &operation.account)
                    .and_then(|store| store.recover_package_switch(&path, &operation, &catalog))
                    .map_err(|error| error.to_string())
            } else if interrupted {
                installer::recover(&root, client, &path)
            } else if electron {
                installer::restore_electron(&root, client, &path, &catalog)
            } else {
                installer::restore(&root, client, &path)
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
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.activity.restoring = false;
                if result.is_ok() {
                    app.activity.confirmation = None;
                    app.install_receipt = None;
                    app.refresh_after_change(cx);
                }
                app.activity.outcome = Some(result.map(|_| {
                    if interrupted {
                        "Interrupted operation recovered.".into()
                    } else {
                        "Addon files restored.".into()
                    }
                }));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn backups_page(&self, palette: Palette, cx: &Context<Self>) -> impl IntoElement {
        let client = self.preferences.client;
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
        for (index, (label, path)) in self
            .activity_archives()
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
                .when_some(self.root(client).cloned(), |row, root| {
                    row.child(
                        activity_button(
                            palette.button("open-backup-folder", ""),
                            palette,
                            cx.listener(move |app, _: &(), _, cx| {
                                match installer::open_backup_folder(&root, client) {
                                    Ok(path) => cx.open_with_system(&path),
                                    Err(error) => {
                                        app.activity.outcome = Some(Err(error));
                                        cx.notify();
                                    }
                                }
                            }),
                        )
                        .child(icon("folder", 14., palette.muted))
                        .child("Open backups"),
                    )
                }),
        );
        if self.root(client).is_none() {
            return page.child(palette.label(format!(
                "Choose your {} folder in Settings.",
                client.label()
            )));
        }
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
                        .border_color(rgb(palette.line))
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
                            activity_button(
                                palette.button("open-library", "Open library"),
                                palette,
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
                        .border_color(rgb(palette.line))
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
                                        .border_color(rgb(palette.line))
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
                                                            .border_color(rgb(palette.line))
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
                                        move |cx| cx.reveal_path(&show),
                                    ))
                                })
                                .when(
                                    ordinary_update
                                        || backup.interrupted
                                        || (!backup.recovered
                                            && (!record.previous.is_empty() || backup.electron)),
                                    |actions| {
                                        actions.child(activity_text_button(
                                            palette,
                                            ("restore-backup", index),
                                            if backup.interrupted {
                                                "Recover…"
                                            } else {
                                                "Restore previous"
                                            },
                                            cx.listener(move |app, _: &(), _, cx| {
                                                app.activity.confirm_addon_restore(path.clone());
                                                cx.notify();
                                            }),
                                        ))
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
                    .border_color(rgb(palette.line))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(40.))
                            .flex_shrink_0()
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(palette.line))
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
                        row.child(activity_text_button(
                            palette,
                            ("retry-addon-action", index),
                            "Retry",
                            cx.listener(move |app, _: &(), window, cx| {
                                app.open_release(&source, window, cx)
                            }),
                        ))
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
        for entry in self.daily_history.iter().take(3) {
            let run = &entry.run;
            let when = if run.checked_at == 0 {
                "Earlier check".into()
            } else {
                activity_time(run.checked_at / 1000)
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
                        .border_color(rgb(palette.line))
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
                            "{} updated · {} need attention",
                            run.updated,
                            run.errors.len()
                        ))
                        .text_size(px(12.))
                        .line_height(px(21.6)),
                );
            for (index, update) in run.updates.iter().enumerate() {
                let key = (run.checked_at, entry.client.index(), index);
                let expanded = self.activity.release_notes.contains(&key);
                let summary = format!(
                    "{} · {} · {} → {}",
                    update.name,
                    entry.client.label(),
                    update.from,
                    update.to
                );
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
                    .child(summary),
                );
                if expanded && !update.notes.is_empty() {
                    row = row.child(
                        div()
                            .id(SharedString::from(format!("daily-notes-scroll-{key:?}")))
                            .max_h(px(200.))
                            .overflow_y_scroll()
                            .child(
                                palette
                                    .label(update.notes.clone())
                                    .font_family("monospace")
                                    .text_size(px(11.))
                                    .line_height(px(19.8))
                                    .p_3()
                                    .bg(rgb(palette.surface)),
                            ),
                    );
                }
            }
            for (index, error) in run.errors.iter().enumerate() {
                let key = (
                    run.checked_at,
                    entry.client.index(),
                    run.updates.len() + index,
                );
                let expanded = self.activity.release_notes.contains(&key);
                row = row.child(
                    activity_text_button(
                        palette,
                        SharedString::from(format!("daily-error-{key:?}")),
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
                    .child(format!("{} · {error}", entry.client.label())),
                );
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
        let cancel_focus = focus.cancel.clone();
        let restore_focus = focus.restore.clone();
        let restoring = self.activity.restoring;
        let record = &backup.transaction;
        let mut body = crate::account::dialog_panel("addon-restore-dialog", palette)
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "tab" {
                    if !restoring {
                        if cancel_focus.is_focused(window) {
                            restore_focus.focus(window);
                        } else {
                            cancel_focus.focus(window);
                        }
                    }
                    cx.stop_propagation();
                }
            })
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
            actions = actions.child(activity_button(
                palette
                    .button("cancel-restore", "Cancel")
                    .track_focus(&focus.cancel),
                palette,
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
            activity_button(
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
                cx.listener(|app, _: &(), _, cx| app.restore_backup(cx)),
            )
        };
        modal = modal.child(
            div()
                .id("addon-restore-backdrop")
                .absolute()
                .top(px(78.))
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

fn activity_time(created: u64) -> String {
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
