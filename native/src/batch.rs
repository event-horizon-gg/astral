use crate::{Astral, Client, Definition, Palette, bundled, installer, packages, releases};
use gpui::{Context, div, prelude::*, px, rgb};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};

#[derive(Clone)]
struct Update {
    source: usize,
    versions: Option<VersionChange>,
}

const FOREVER_ESSENTIALS: [&str; 9] = [
    "ellesmere",
    "dialogueui",
    "narcissus",
    "keyui",
    "handynotes",
    "questie",
    "dbm",
    "dbm-dungeons",
    "dbm-raids",
];

#[derive(Clone)]
struct VersionChange {
    from: String,
    to: String,
}

impl VersionChange {
    fn needs_install(&self, source: &Definition, installed: Option<&str>) -> Result<bool, String> {
        match installed.and_then(|version| releases::compare(source, version, &self.to)) {
            Some(Ordering::Equal | Ordering::Less) => Ok(false),
            Some(Ordering::Greater) if installed == Some(self.from.as_str()) => Ok(true),
            _ => Err("The installed version changed. Check for updates again.".into()),
        }
    }
}

pub struct Batch {
    root: PathBuf,
    client: Client,
    updates: Vec<Update>,
    suite: bool,
    results: Vec<Result<&'static str, String>>,
    running: bool,
}

impl Batch {
    pub(crate) fn running(&self) -> bool {
        self.running
    }

    fn remaining_after_failure(&self) -> Option<&[Update]> {
        let failed = self.results.iter().position(Result::is_err)?;
        Some(&self.updates[failed..])
    }
}

fn apply_update(
    update: &Update,
    root: &Path,
    client: Client,
    catalog: &[Definition],
) -> Result<&'static str, String> {
    installer::game_closed()?;
    let source = &catalog[update.source];
    let (tag, notes, bytes) = fetch_package(update, source)?;
    install_package(bytes, (&tag, &notes), update, root, client, catalog)
}

fn fetch_package(
    update: &Update,
    source: &Definition,
) -> Result<(String, String, Vec<u8>), String> {
    if source.bundled == Some(true) {
        let tag = bundled::version(&source.folder).ok_or("Bundled addon unavailable.")?;
        check_release(update, &tag)?;
        Ok((tag, String::new(), bundled::package(&source.folder)?))
    } else {
        let release = releases::latest(&releases::client(), source)?;
        check_release(update, &release.tag)?;
        Ok((
            release.tag,
            release.notes,
            releases::download(&release.package)?,
        ))
    }
}

pub(crate) fn inspect_missing(
    source: usize,
    root: &Path,
    client: Client,
    catalog: &[Definition],
) -> Result<packages::Inspection, String> {
    let definition = &catalog[source];
    let (_, _, bytes) = fetch_package(
        &Update {
            source,
            versions: None,
        },
        definition,
    )?;
    packages::inspect(bytes, root, client, definition, catalog)
        .map_err(|error| format!("{}: {error}", definition.name))
}

pub(crate) fn install_missing(
    source: usize,
    root: &Path,
    client: Client,
    catalog: &[Definition],
) -> Result<bool, String> {
    apply_update(
        &Update {
            source,
            versions: None,
        },
        root,
        client,
        catalog,
    )
    .map(|status| status == "Installed")
}

fn install_package(
    bytes: Vec<u8>,
    (tag, notes): (&str, &str),
    update: &Update,
    root: &Path,
    client: Client,
    catalog: &[Definition],
) -> Result<&'static str, String> {
    let source = &catalog[update.source];
    let package = packages::inspect(bytes, root, client, source, catalog)?;
    let installed = package.installed_version.as_deref();
    let status = if let Some(change) = &update.versions {
        if !change.needs_install(source, installed)? {
            return Ok("Already current");
        }
        "Updated"
    } else {
        if installed.is_some() {
            return Ok("Already installed");
        }
        if package.modules.iter().any(|module| module.replaces) {
            return Err(
                "This package would replace existing folders. Review it individually.".into(),
            );
        }
        "Installed"
    };
    installer::install(&package, &source.name, tag, notes)?;
    Ok(status)
}

#[derive(Clone, Deserialize, Serialize)]
pub struct DailyUpdate {
    pub name: String,
    pub from: String,
    pub to: String,
    pub notes: String,
}

#[derive(Default)]
pub struct DailyCheck {
    pub updates: Vec<DailyUpdate>,
    pub errors: Vec<String>,
    pub results: Vec<DailyResult>,
    pub deferred: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DailyStatus {
    Updated,
    Current,
    Review,
    Error,
    External,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct DailyResult {
    pub id: String,
    pub name: String,
    pub status: DailyStatus,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub notes: String,
}

impl DailyResult {
    pub fn summary(&self, client: Client) -> String {
        let detail = if self.status == DailyStatus::Updated
            && (!self.from.is_empty() || !self.to.is_empty())
        {
            format!("{} → {}", self.from, self.to)
        } else {
            self.message.clone()
        };
        if self.name.is_empty() {
            format!("{} · {detail}", client.label())
        } else {
            format!("{} · {} · {detail}", self.name, client.label())
        }
    }

    pub fn issue(id: &str, name: &str, status: DailyStatus, message: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            status,
            from: String::new(),
            to: String::new(),
            message: message.into(),
            notes: String::new(),
        }
    }
}

pub fn game_open(error: &str) -> bool {
    error.contains("Close World of Warcraft") || error.contains("WoW started")
}

impl DailyCheck {
    pub fn interrupted(&self) -> bool {
        self.deferred || self.errors.iter().any(|error| game_open(error))
    }
}

pub fn daily_updates(
    root: &Path,
    client: Client,
    catalog: &[Definition],
    excluded: &[String],
) -> Result<DailyCheck, String> {
    installer::game_closed()?;
    let installed =
        crate::library::scan(root, client, catalog).map_err(|error| error.to_string())?;
    let agent = releases::client();
    let mut report = DailyCheck::default();
    let legacy_rio = crate::schedule::legacy_raider_io();
    for (index, source) in catalog.iter().enumerate() {
        let Some(addon) = installed.iter().find(|addon| addon.folder == source.folder) else {
            continue;
        };
        if !source.supports(client) || excluded.contains(&source.folder) {
            continue;
        }
        if source.id == "rio" && legacy_rio {
            report.results.push(DailyResult::issue(
                &source.id,
                &source.name,
                DailyStatus::External,
                "Handled by the existing daily Raider.IO updater on this Mac.",
            ));
            continue;
        }
        if installer::game_closed().is_err() {
            report.deferred = true;
            break;
        }
        let result = (|| -> Result<(), String> {
            let bundled = source.bundled == Some(true);
            let release = if bundled {
                None
            } else if source.repo.is_some() && source.asset.is_some() {
                Some(releases::latest(&agent, source)?)
            } else {
                return Ok(());
            };
            let tag = match &release {
                Some(release) => release.tag.clone(),
                None => bundled::version(&source.folder).ok_or("Bundled addon unavailable.")?,
            };
            let Some(comparison) = releases::compare(source, &addon.version, &tag) else {
                report.results.push(DailyResult::issue(
                    &source.id,
                    &source.name,
                    DailyStatus::Review,
                    "Version format needs a manual review.",
                ));
                return Ok(());
            };
            let mut outcome =
                DailyResult::issue(&source.id, &source.name, DailyStatus::Current, "");
            if comparison == Ordering::Greater {
                let notes = release
                    .as_ref()
                    .map(|release| release.notes.chars().take(6000).collect::<String>())
                    .unwrap_or_default();
                let bytes = match release {
                    Some(release) => releases::download(&release.package)?,
                    None => bundled::package(&source.folder)?,
                };
                let update = Update {
                    source: index,
                    versions: Some(VersionChange {
                        from: addon.version.clone(),
                        to: tag.clone(),
                    }),
                };
                if install_package(bytes, (&tag, &notes), &update, root, client, catalog)?
                    == "Updated"
                {
                    outcome.status = DailyStatus::Updated;
                    outcome.from = addon.version.clone();
                    outcome.to = tag;
                    outcome.notes = notes;
                }
            }
            if source.id == "talents"
                && crate::talents::refresh_changed(root)?
                && outcome.status == DailyStatus::Current
            {
                outcome.status = DailyStatus::Updated;
                outcome.message = "Druid talent builds refreshed.".into();
            }
            if outcome.status == DailyStatus::Updated {
                report.updates.push(DailyUpdate {
                    name: outcome.name.clone(),
                    from: if outcome.from.is_empty() {
                        addon.version.clone()
                    } else {
                        outcome.from.clone()
                    },
                    to: if outcome.to.is_empty() {
                        addon.version.clone()
                    } else {
                        outcome.to.clone()
                    },
                    notes: if outcome.notes.is_empty() {
                        outcome.message.clone()
                    } else {
                        outcome.notes.clone()
                    },
                });
            }
            report.results.push(outcome);
            Ok(())
        })();
        if let Err(error) = result {
            let rate_limited = error.to_ascii_lowercase().contains("rate limit");
            report.deferred |= game_open(&error);
            report.results.push(DailyResult::issue(
                &source.id,
                &source.name,
                DailyStatus::Error,
                &error,
            ));
            report.errors.push(format!("{}: {error}", source.name));
            if report.deferred || (rate_limited && source.id != "talents") {
                break;
            }
        }
    }
    Ok(report)
}

fn check_release(update: &Update, tag: &str) -> Result<(), String> {
    if update
        .versions
        .as_ref()
        .is_some_and(|change| change.to != tag)
    {
        return Err("The available release changed. Check for updates again.".into());
    }
    Ok(())
}

fn missing_essentials(catalog: &[Definition], installed: &[crate::Addon]) -> Vec<usize> {
    FOREVER_ESSENTIALS
        .iter()
        .filter_map(|id| {
            catalog.iter().position(|source| {
                source.id == *id
                    && source.supports(Client::Forever)
                    && source.repo.is_some()
                    && !installed.iter().any(|addon| addon.folder == source.folder)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_keeps_the_failed_and_pending_addons_in_order() {
        let mut batch = Batch {
            root: PathBuf::new(),
            client: Client::Retail,
            updates: (0..4)
                .map(|source| Update {
                    source,
                    versions: None,
                })
                .collect(),
            suite: false,
            results: vec![
                Ok("Updated"),
                Ok("Already current"),
                Err("Unavailable".into()),
            ],
            running: false,
        };
        let remaining = batch.remaining_after_failure().unwrap();
        assert_eq!(
            remaining
                .iter()
                .map(|update| update.source)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        batch.results[2] = Ok("Updated");
        assert!(batch.remaining_after_failure().is_none());
    }

    #[test]
    fn forever_suite_lists_only_missing_addons_in_install_order() {
        let catalog = crate::library::catalog();
        let installed = crate::Addon {
            name: "Questie".into(),
            category: "Exploration".into(),
            folder: "Questie".into(),
            version: "1".into(),
            source: "GitHub",
            modules: vec!["Questie".into()],
            artwork: None,
        };
        let plan: Vec<_> = missing_essentials(&catalog, &[installed])
            .into_iter()
            .map(|index| catalog[index].id.as_str())
            .collect();
        assert_eq!(
            plan,
            [
                "ellesmere",
                "dialogueui",
                "narcissus",
                "keyui",
                "handynotes",
                "dbm",
                "dbm-dungeons",
                "dbm-raids"
            ]
        );
    }

    #[test]
    fn batch_skips_current_versions_and_refuses_to_install_after_an_unreviewed_change() {
        let catalog = crate::library::catalog();
        let source = catalog
            .iter()
            .position(|source| source.id == "talents")
            .unwrap();
        let update = VersionChange {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
        };
        let check = |version| update.needs_install(&catalog[source], version);
        assert_eq!(check(Some("1.0.0")), Ok(true));
        assert_eq!(check(Some("1.1.0")), Ok(false));
        assert_eq!(check(Some("1.2.0")), Ok(false));
        assert!(check(Some("1.0.1")).is_err());
        assert!(check(None).is_err());
    }
}

impl Astral {
    pub(crate) fn addon_batch_running(&self) -> bool {
        self.batch.as_ref().is_some_and(Batch::running)
            || self.single_update.is_some()
            || self
                .project_review
                .as_ref()
                .is_some_and(|review| review.is_busy())
    }

    pub fn excluded_from_daily(&self, folder: &str) -> bool {
        self.preferences.batch_exclusions[self.preferences.client.index()]
            .iter()
            .any(|value| value == folder)
    }

    pub fn toggle_daily_exclusion(&mut self, folder: &str, cx: &mut Context<Self>) {
        let excluded = self.excluded_from_daily(folder);
        let folders = &mut self.preferences.batch_exclusions[self.preferences.client.index()];
        if excluded {
            folders.retain(|value| value != folder);
        } else {
            folders.push(folder.to_owned());
        }
        self.save_preferences();
        cx.notify();
    }

    pub fn has_batch_updates(&self) -> bool {
        self.can_start_update_batch() && self.batch_update_count() > 0
    }

    pub(crate) fn can_start_update_batch(&self) -> bool {
        !self.checking
            && self.single_update.is_none()
            && !self.scanning
            && self.batch.is_none()
            && !self.daily_running
            && !self.account_work_in_progress()
            && !self.settings_write_in_progress()
            && !self.app_updates.installing()
            && !self.talent_refreshing
            && self.release_review.is_none()
            && self.project_review.is_none()
            && self.removal.is_none()
            && !self.activity.has_confirmation()
            && !self.profile_game_running()
    }

    pub fn batch_update_count(&self) -> usize {
        self.addons
            .iter()
            .filter(|addon| self.update_status(addon) == "Update available")
            .count()
    }

    pub fn update_all(&mut self, cx: &mut Context<Self>) {
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            return;
        };
        if !self.has_batch_updates() {
            return;
        }
        let updates =
            self.addons
                .iter()
                .filter_map(|addon| {
                    let check = self.checks.get(&addon.folder)?.as_ref().ok()?;
                    if check.comparison != Some(Ordering::Greater) {
                        return None;
                    }
                    let source = self.catalog.iter().position(|source| {
                        source.folder == addon.folder && source.supports(client)
                    })?;
                    Some(Update {
                        source,
                        versions: Some(VersionChange {
                            from: addon.version.clone(),
                            to: check.tag.clone(),
                        }),
                    })
                })
                .collect::<Vec<_>>();
        if updates.is_empty() {
            return;
        }
        self.start_update_batch(root, client, updates, cx);
    }

    fn start_update_batch(
        &mut self,
        root: PathBuf,
        client: Client,
        updates: Vec<Update>,
        cx: &mut Context<Self>,
    ) {
        self.batch_result = None;
        self.batch = Some(Batch {
            root,
            client,
            updates,
            suite: false,
            results: Vec::new(),
            running: false,
        });
        self.run_batch(cx);
    }

    fn retry_update_batch(&mut self, cx: &mut Context<Self>) {
        if !self.can_start_update_batch() {
            return;
        }
        let Some(batch) = &self.batch_result else {
            return;
        };
        if self.preferences.client != batch.client || self.root(batch.client) != Some(&batch.root) {
            return;
        }
        let Some(remaining) = batch.remaining_after_failure() else {
            return;
        };
        self.start_update_batch(batch.root.clone(), batch.client, remaining.to_vec(), cx);
    }

    pub fn forever_essentials(&self, palette: Palette, cx: &Context<Self>) -> Option<gpui::Div> {
        if self.preferences.client != Client::Forever
            || self.scanning
            || self.root(Client::Forever).is_none()
        {
            return None;
        }
        let active = self.batch.as_ref().filter(|batch| batch.suite);
        let result = self
            .batch_result
            .as_ref()
            .filter(|batch| batch.suite && self.root(Client::Forever) == Some(&batch.root));
        let missing = missing_essentials(&self.catalog, &self.addons);
        if missing.is_empty() && result.is_none() {
            return None;
        }
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p(px(20.))
            .border_1()
            .border_color(palette.line)
            .bg(rgb(palette.surface));
        if !missing.is_empty() {
            let button = palette.button(
                "install-forever-essentials",
                format!("Install missing ({})", missing.len()),
            );
            let button = if self.can_start_update_batch() {
                crate::keyboard_button(
                    button,
                    palette,
                    cx.listener(|app, _: &(), _, cx| app.install_suite(cx)),
                )
            } else {
                button.opacity(0.45).cursor_default()
            };
            let names = missing
                .iter()
                .map(|&source| self.catalog[source].name.as_str())
                .collect::<Vec<_>>();
            panel = panel
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child("Forever essentials"),
                        )
                        .child(button),
                )
                .child(palette.label(names.join(", ")).text_size(px(13.)))
                .when(self.profile_game_running(), |panel| {
                    panel.child(
                        palette
                            .label("Close WoW before installing.")
                            .text_size(px(13.)),
                    )
                });
        }
        if let Some(batch) = active {
            let index = batch.results.len();
            let progress = batch.updates.get(index).map_or_else(
                || "Finishing installation…".to_owned(),
                |update| {
                    format!(
                        "Installing {} ({}/{})",
                        self.catalog[update.source].name,
                        index + 1,
                        batch.updates.len()
                    )
                },
            );
            panel = panel.child(palette.label(progress).text_size(px(13.)));
        }
        if let Some(batch) = result {
            let installed = batch
                .results
                .iter()
                .filter(|status| matches!(status, Ok("Installed")))
                .count();
            let failed = batch
                .updates
                .iter()
                .zip(&batch.results)
                .find_map(|(update, status)| {
                    status
                        .as_ref()
                        .err()
                        .map(|error| (&self.catalog[update.source].name, error))
                });
            let summary = match failed {
                Some((name, error)) => format!("{installed} installed. {name}: {error}"),
                None => format!("{installed} addons installed."),
            };
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        palette
                            .label(summary)
                            .text_size(px(13.))
                            .when(failed.is_some(), |label| {
                                label.text_color(rgb(palette.error))
                            }),
                    )
                    .child(crate::keyboard_button(
                        palette.text_button("dismiss-suite-result", "Dismiss"),
                        palette,
                        cx.listener(|app, _: &(), _, cx| {
                            app.batch_result = None;
                            cx.notify();
                        }),
                    )),
            );
        }
        Some(panel)
    }

    fn install_suite(&mut self, cx: &mut Context<Self>) {
        if self.preferences.client != Client::Forever || !self.can_start_update_batch() {
            return;
        }
        let Some(root) = self.root(Client::Forever).cloned() else {
            return;
        };
        let updates: Vec<_> = missing_essentials(&self.catalog, &self.addons)
            .into_iter()
            .map(|source| Update {
                source,
                versions: None,
            })
            .collect();
        if updates.is_empty() {
            return;
        }
        self.batch_result = None;
        self.batch = Some(Batch {
            root,
            client: Client::Forever,
            updates,
            suite: true,
            results: Vec::new(),
            running: false,
        });
        self.run_batch(cx);
    }

    fn run_batch(&mut self, cx: &mut Context<Self>) {
        let Some(batch) = self
            .batch
            .as_mut()
            .filter(|batch| !batch.running && batch.results.is_empty())
        else {
            return;
        };
        batch.running = true;
        let (root, client, updates) = (batch.root.clone(), batch.client, batch.updates.clone());
        let catalog = self.catalog.clone();
        let preferences = self.preferences_path.clone();
        cx.spawn(async move |this, cx| {
            for update in updates {
                let root = root.clone();
                let catalog = catalog.clone();
                let preferences = preferences.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let result = apply_update(&update, &root, client, &catalog);
                        let source = &catalog[update.source];
                        crate::activity::record(
                            preferences.as_deref(),
                            crate::activity::Action {
                                root,
                                client,
                                name: source.name.clone(),
                                retry_source: Some(source.id.clone()),
                                from: update.versions.as_ref().map(|version| version.from.clone()),
                                to: update.versions.as_ref().map(|version| version.to.clone()),
                                verb: match &result {
                                    Ok("Already current" | "Already installed") => {
                                        "Already up to date"
                                    }
                                    Ok(status) => status,
                                    Err(_) => "Failed",
                                }
                                .to_owned(),
                                error: result.as_ref().err().cloned(),
                                time: 0,
                            },
                        );
                        result
                    })
                    .await;
                let failed = result.is_err();
                if this
                    .update(cx, |app, cx| {
                        if let Some(batch) = &mut app.batch {
                            batch.results.push(result);
                        }
                        cx.notify();
                    })
                    .is_err()
                    || failed
                {
                    break;
                }
            }
            let _ = this.update(cx, |app, cx| {
                app.batch_result = app.batch.take();
                app.refresh_after_change(cx);
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn batch_result_panel(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<gpui::Div> {
        let active = self.batch.as_ref().filter(|batch| !batch.suite);
        let batch = active.or(self.batch_result.as_ref().filter(|batch| !batch.suite))?;
        if self.preferences.client != batch.client || self.root(batch.client) != Some(&batch.root) {
            return None;
        }
        let running = active.is_some_and(|batch| batch.running);
        let mut panel = div()
            .p(px(20.))
            .border_1()
            .border_color(palette.line)
            .bg(rgb(palette.surface))
            .flex()
            .flex_col()
            .gap_2();
        if running {
            let index = batch.results.len();
            let message = batch
                .updates
                .get(index)
                .map(|update| {
                    format!(
                        "Updating {} ({}/{})",
                        self.catalog[update.source].name,
                        index + 1,
                        batch.updates.len()
                    )
                })
                .unwrap_or_else(|| "Finishing updates…".into());
            return Some(panel.child(palette.label(message).text_size(px(13.))));
        }
        let mut updated = Vec::new();
        let mut current = 0;
        let mut failed = None;
        for (update, result) in batch.updates.iter().zip(&batch.results) {
            match result {
                Ok("Updated") => updated.push(self.catalog[update.source].name.as_str()),
                Ok(_) => current += 1,
                Err(error) => failed = Some((self.catalog[update.source].name.as_str(), error)),
            }
        }
        let pending = &batch.updates[batch.results.len()..];
        let mut summary = format!("{} updated", updated.len());
        if current > 0 {
            summary.push_str(&format!(" · {current} already current"));
        }
        if failed.is_some() {
            summary.push_str(&format!(" · 1 failed · {} not attempted", pending.len()));
        }
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(13.))
                        .child(summary),
                )
                .child(crate::keyboard_button(
                    palette.text_button("dismiss-update-results", "Dismiss"),
                    palette,
                    cx.listener(|app, _: &(), _, cx| {
                        app.batch_result = None;
                        cx.notify();
                    }),
                )),
        );
        if !updated.is_empty() {
            panel = panel.child(palette.label(updated.join(", ")).text_size(px(13.)));
        }
        if let Some((name, error)) = failed {
            panel = panel.child(
                palette
                    .label(format!("{name}: {error}"))
                    .text_size(px(13.))
                    .text_color(rgb(palette.error)),
            );
            if !pending.is_empty() {
                let names = pending
                    .iter()
                    .map(|update| self.catalog[update.source].name.as_str())
                    .collect::<Vec<_>>();
                panel = panel.child(
                    palette
                        .label(format!("Not attempted: {}", names.join(", ")))
                        .text_size(px(13.)),
                );
            }
            let button = palette
                .text_button("retry-update-results", "Retry remaining updates")
                .mt_2();
            panel = panel.child(div().child(if !self.can_start_update_batch() {
                button.opacity(0.45).cursor_default()
            } else {
                crate::keyboard_button(
                    button,
                    palette,
                    cx.listener(|app, _: &(), _, cx| app.retry_update_batch(cx)),
                )
            }));
        }
        Some(panel)
    }
}
