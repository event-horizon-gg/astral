use crate::{Astral, Client, Palette, batch, library, preferences};
use gpui::{Context, div, prelude::*, rgb};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Deserialize, Serialize)]
pub struct Run {
    pub root: PathBuf,
    pub next_at: u64,
    pub updated: usize,
    pub errors: Vec<String>,
    #[serde(default)]
    pub checked_at: u64,
    #[serde(default)]
    pub updates: Vec<batch::DailyUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub deferred: bool,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct HistoryEntry {
    pub client: Client,
    pub run: Run,
}

#[derive(Deserialize, Serialize)]
struct Reports {
    latest: [Option<Run>; 2],
    history: Vec<HistoryEntry>,
    #[serde(default)]
    next_check: u64,
}

#[derive(Clone, Copy)]
struct Check {
    id: u64,
    started_at: u64,
}

pub struct HistoryGroup<'a> {
    pub entries: Vec<&'a HistoryEntry>,
}

impl HistoryGroup<'_> {
    pub fn checked_at(&self) -> u64 {
        self.entries
            .iter()
            .map(|entry| entry.run.started_at.unwrap_or(entry.run.checked_at))
            .filter(|time| *time > 0)
            .min()
            .unwrap_or(0)
    }

    pub fn updated(&self) -> usize {
        self.entries.iter().map(|entry| entry.run.updated).sum()
    }

    pub fn attention(&self) -> usize {
        self.entries
            .iter()
            .map(|entry| entry.run.errors.len())
            .sum()
    }

    pub fn deferred(&self) -> bool {
        self.entries.iter().any(|entry| {
            entry.run.deferred || entry.run.errors.iter().any(|error| game_open(error))
        })
    }
}

pub fn grouped_history(history: &[HistoryEntry]) -> Vec<HistoryGroup<'_>> {
    let mut groups: Vec<HistoryGroup<'_>> = Vec::new();
    let mut indices = std::collections::HashMap::<u64, usize>::new();
    for entry in history {
        let id = entry.run.check_id.filter(|id| *id > 0);
        let index = id
            .and_then(|id| indices.get(&id).copied())
            .unwrap_or_else(|| {
                let index = groups.len();
                groups.push(HistoryGroup {
                    entries: Vec::new(),
                });
                if let Some(id) = id {
                    indices.insert(id, index);
                }
                index
            });
        groups[index].entries.push(entry);
    }
    for group in &mut groups {
        group.entries.sort_by_key(|entry| entry.client.index());
    }
    groups
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredReports {
    History(Reports),
    Latest([Option<Run>; 2]),
}

impl Reports {
    fn from_latest(latest: [Option<Run>; 2]) -> Self {
        let history = Client::ALL
            .into_iter()
            .filter_map(|client| {
                let run = latest[client.index()].clone()?;
                Some(HistoryEntry { client, run })
            })
            .collect();
        Self {
            latest,
            history,
            next_check: 0,
        }
    }

    fn begin_check(&mut self) -> io::Result<Check> {
        let previous = self
            .history
            .iter()
            .map(|entry| &entry.run)
            .chain(self.latest.iter().flatten())
            .filter_map(|run| run.check_id)
            .max()
            .unwrap_or(0)
            .max(self.next_check);
        self.next_check = previous
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Update history ID is exhausted."))?;
        Ok(Check {
            id: self.next_check,
            started_at: now_millis(),
        })
    }

    fn record(
        &mut self,
        path: &Path,
        client: Client,
        mut run: Run,
        check: Check,
    ) -> io::Result<()> {
        run.check_id = Some(check.id);
        run.started_at = Some(check.started_at);
        self.latest[client.index()] = Some(run.clone());
        self.history.insert(0, HistoryEntry { client, run });
        let mut seen = std::collections::HashSet::new();
        let mut count = 0;
        self.history.retain(|entry| {
            let id = entry.run.check_id.filter(|id| *id > 0);
            if id.is_some_and(|id| seen.contains(&id)) {
                return true;
            }
            count += 1;
            if count > 14 {
                return false;
            }
            if let Some(id) = id {
                seen.insert(id);
            }
            true
        });
        preferences::save_json(&reports_path(path), self)
    }

    fn defer_check(&mut self, path: &Path, check: Check) -> io::Result<()> {
        let mut changed = false;
        for run in self
            .history
            .iter_mut()
            .map(|entry| &mut entry.run)
            .chain(self.latest.iter_mut().flatten())
        {
            if run.check_id == Some(check.id) {
                run.deferred = true;
                run.next_at = run.next_at.min(now() + 60);
                changed = true;
            }
        }
        if changed {
            preferences::save_json(&reports_path(path), self)?;
        }
        Ok(())
    }
}

fn is_due(run: Option<&Run>, root: &Path) -> bool {
    run.is_none_or(|run| run.root != root || now() >= run.next_at)
}

pub fn lock(path: &Path) -> io::Result<fs::File> {
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(dir)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("daily.lock"))?;
    lock.try_lock()?;
    Ok(lock)
}

fn reports_path(path: &Path) -> PathBuf {
    path.with_file_name("daily-runs.json")
}

fn read_reports(path: &Path, fallback: &[Option<Run>; 2]) -> io::Result<Reports> {
    match fs::read(reports_path(path)) {
        Ok(data) => match serde_json::from_slice(&data)? {
            StoredReports::History(reports) => Ok(reports),
            StoredReports::Latest(latest) => Ok(Reports::from_latest(latest)),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(Reports::from_latest(fallback.clone()))
        }
        Err(error) => Err(error),
    }
}

pub fn run_background(path: &Path, roots: [Option<PathBuf>; 2]) -> io::Result<()> {
    let prefs = preferences::Preferences::load(path)?;
    if !prefs.daily_updates || !prefs.background_updates {
        return Ok(());
    }
    let _lock = match lock(path) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut reports = read_reports(path, &prefs.daily_runs)?;
    let mut catalog = library::catalog();
    catalog.extend(prefs.projects.iter().cloned());
    let detected = prefs.detected_folders(&roots);
    let mut check = None;
    for client in Client::ALL {
        let index = client.index();
        let Some(root) = prefs.folders[index].as_ref().or(detected[index].as_ref()) else {
            continue;
        };
        if !is_due(reports.latest[index].as_ref(), root) {
            continue;
        }
        let current = preferences::Preferences::load(path)?;
        if !current.daily_updates
            || !current.background_updates
            || current.folders[index]
                .as_ref()
                .or(current.detected_folders(&roots)[index].as_ref())
                != Some(root)
        {
            break;
        }
        if check.is_none() {
            check = Some(reports.begin_check()?);
        }
        let check = check.unwrap();
        let result = batch::daily_updates(root, client, &catalog, &current.batch_exclusions[index]);
        if let Err(error) = &result
            && game_open(error)
        {
            reports.defer_check(path, check)?;
            break;
        }
        let run = report(root.clone(), result);
        let interrupted = run.errors.iter().any(|error| game_open(error));
        reports.record(path, client, run, check)?;
        if interrupted {
            reports.defer_check(path, check)?;
            break;
        }
    }
    Ok(())
}

fn report(root: PathBuf, result: Result<batch::DailyCheck, String>) -> Run {
    let batch::DailyCheck { updates, errors } = result.unwrap_or_else(|error| batch::DailyCheck {
        errors: vec![error],
        ..Default::default()
    });
    let interval = if errors.is_empty() {
        86_400
    } else if errors.iter().any(|error| game_open(error)) {
        60
    } else {
        3_600
    };
    let checked_at = now_millis();
    Run {
        root,
        next_at: checked_at / 1000 + interval,
        updated: updates.len(),
        errors,
        checked_at,
        updates,
        check_id: None,
        started_at: None,
        deferred: false,
    }
}

fn game_open(error: &str) -> bool {
    error.contains("Close World of Warcraft") || error.contains("WoW started")
}

fn run_checks(
    path: &Path,
    reports: &mut Reports,
    targets: Vec<(Client, PathBuf, Vec<String>)>,
    force: bool,
    mut check: impl FnMut(&Path, Client, &[String]) -> Result<batch::DailyCheck, String>,
) -> Result<(), String> {
    let mut check_id = None;
    for (client, root, excluded) in targets {
        let index = client.index();
        if !force && !is_due(reports.latest[index].as_ref(), &root) {
            continue;
        }
        if check_id.is_none() {
            check_id = Some(reports.begin_check().map_err(|error| error.to_string())?);
        }
        let check_id = check_id.unwrap();
        let result = check(&root, client, &excluded);
        if let Err(error) = &result
            && game_open(error)
        {
            reports
                .defer_check(path, check_id)
                .map_err(|error| error.to_string())?;
            return Err(error.clone());
        }
        let run = report(root, result);
        let interrupted = run.errors.iter().any(|error| game_open(error));
        reports
            .record(path, client, run, check_id)
            .map_err(|error| error.to_string())?;
        if interrupted {
            reports
                .defer_check(path, check_id)
                .map_err(|error| error.to_string())?;
            return Err("Close World of Warcraft to finish checking addons.".into());
        }
    }
    Ok(())
}

impl Astral {
    pub fn configure_daily_schedule(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        let Some(path) = self.preferences_path.clone() else {
            return;
        };
        if self.daily_schedule_busy {
            return;
        }
        if self.preferences.background_updates != enabled {
            self.preferences.background_updates = enabled;
            self.save_preferences();
            if self.preferences_error.is_some() {
                self.preferences.background_updates = !enabled;
                cx.notify();
                return;
            }
        }
        self.daily_schedule_busy = true;
        self.daily_schedule_error = None;
        let task = cx.background_executor().spawn(async move {
            let _lock = lock(&path).map_err(|error| {
                if error.kind() == io::ErrorKind::WouldBlock {
                    io::Error::other("An update is in progress. Try again after it finishes.")
                } else {
                    error
                }
            })?;
            crate::schedule::configure(enabled)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.daily_schedule_busy = false;
                app.daily_schedule_error = result.err().map(|error| error.to_string());
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn start_daily_timer(&self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_secs(60)).await;
                if this.update(cx, |app, cx| app.run_due_updates(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn run_due_updates(&mut self, cx: &mut Context<Self>) {
        if self.daily_running {
            return;
        }
        if crate::schedule::SUPPORTED
            && self.preferences.background_updates
            && self.daily_schedule_error.is_none()
        {
            if let Some(path) = &self.preferences_path {
                let reports = read_reports(path, &self.preferences.daily_runs);
                self.receive_daily_reports(reports, cx);
            }
            return;
        }
        if !self.preferences.daily_updates {
            return;
        }
        self.run_daily_checks(false, cx);
    }

    pub fn load_daily_history(&mut self) {
        if let Some(path) = &self.preferences_path {
            match read_reports(path, &self.preferences.daily_runs) {
                Ok(reports) => {
                    self.daily_history = reports.history;
                    self.daily_history_error = None;
                }
                Err(error) => self.daily_history_error = Some(error.to_string()),
            }
        }
    }

    fn receive_daily_reports(&mut self, reports: io::Result<Reports>, cx: &mut Context<Self>) {
        match reports {
            Ok(reports) => {
                let client = self.preferences.client;
                let old = self.preferences.daily_runs[client.index()].as_ref();
                let changed = reports.latest[client.index()].as_ref().is_some_and(|run| {
                    self.root(client) == Some(&run.root)
                        && run.updated > 0
                        && old.is_none_or(|old| {
                            old.root != run.root || old.checked_at != run.checked_at
                        })
                });
                self.preferences.daily_runs = reports.latest;
                self.daily_history = reports.history;
                self.daily_history_error = None;
                if changed && self.can_run_daily_checks() {
                    self.refresh_after_change(cx);
                }
            }
            Err(error) => self.daily_history_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn can_run_daily_checks(&self) -> bool {
        !self.fixture
            && self.preferences_path.is_some()
            && !self.daily_running
            && !self.app_updates.installing()
            && !self.daily_schedule_busy
            && !self.checking
            && !self.scanning
            && self.batch.is_none()
            && self.release_review.is_none()
            && self.removal.is_none()
            && !self.activity.has_confirmation()
            && !self.talent_refreshing
            && self.project_review.is_none()
            && !self.settings_write_in_progress()
            && !self.account_work_in_progress()
    }

    fn run_daily_checks(&mut self, force: bool, cx: &mut Context<Self>) {
        if !self.can_run_daily_checks() {
            return;
        }
        let targets: Vec<_> = Client::ALL
            .into_iter()
            .filter_map(|client| {
                let index = client.index();
                let root = self.root(client)?;
                (force || is_due(self.preferences.daily_runs[index].as_ref(), root)).then(|| {
                    (
                        client,
                        root.clone(),
                        self.preferences.batch_exclusions[index].clone(),
                    )
                })
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let catalog = self.catalog.clone();
        let Some(path) = self.preferences_path.clone() else {
            return;
        };
        let old_runs = self.preferences.daily_runs.clone();
        self.daily_running = true;
        self.daily_run_error = None;
        let task = cx.background_executor().spawn(async move {
            let result = (|| -> Result<(), String> {
                let _lock = lock(&path).map_err(|error| {
                    if error.kind() == io::ErrorKind::WouldBlock {
                        "An addon update is already in progress.".into()
                    } else {
                        error.to_string()
                    }
                })?;
                let mut reports =
                    read_reports(&path, &old_runs).map_err(|error| error.to_string())?;
                run_checks(
                    &path,
                    &mut reports,
                    targets,
                    force,
                    |root, client, excluded| batch::daily_updates(root, client, &catalog, excluded),
                )
            })();
            (read_reports(&path, &old_runs), result)
        });
        cx.spawn(async move |this, cx| {
            let (reports, result) = task.await;
            let _ = this.update(cx, |app, cx| {
                app.daily_running = false;
                app.daily_waiting = result.as_ref().is_err_and(|error| game_open(error));
                app.daily_run_error = result.err().filter(|_| force || !app.daily_waiting);
                app.receive_daily_reports(reports, cx);
            });
        })
        .detach();
        cx.notify();
    }

    pub fn daily_settings(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let enabled = self.preferences.daily_updates;
        let mut controls = div().flex().flex_col().child(palette.setting_checkbox(
            "toggle-daily",
            "Update addons daily",
            enabled,
            !self.fixture
                && !self.daily_running
                && !self.daily_schedule_busy
                && self.preferences_path.is_some(),
            |app, cx| {
                let previous = app.preferences.daily_updates;
                app.preferences.daily_updates = !previous;
                app.save_preferences();
                if app.preferences_error.is_some() {
                    app.preferences.daily_updates = previous;
                } else {
                    if crate::schedule::SUPPORTED {
                        app.configure_daily_schedule(!previous, cx);
                    }
                    app.run_due_updates(cx);
                }
                cx.notify();
            },
            cx,
        ));
        let can_check = !self.profile_game_running()
            && self.can_run_daily_checks()
            && Client::ALL
                .into_iter()
                .any(|client| self.root(client).is_some());
        let last_check = Client::ALL
            .into_iter()
            .filter_map(|client| {
                self.preferences.daily_runs[client.index()]
                    .as_ref()
                    .filter(|run| self.root(client) == Some(&run.root) && run.checked_at > 0)
                    .map(|run| run.checked_at)
            })
            .max();
        let status = if self.daily_running {
            "Checking addons…".into()
        } else if self.daily_waiting || self.profile_game_running() {
            "Waiting for WoW to close".into()
        } else if let Some(checked_at) = last_check {
            format!(
                "Last check {}",
                crate::activity::activity_time(checked_at / 1000)
            )
        } else {
            "First daily check is pending".into()
        };
        controls = controls.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .mt_3()
                .child(
                    palette
                        .button("check-daily-now", "Check now")
                        .when(!can_check, |button| button.opacity(0.6).cursor_default())
                        .when(can_check, |button| {
                            crate::keyboard_button(
                                button,
                                palette,
                                cx.listener(|app, _: &(), _, cx| app.run_daily_checks(true, cx)),
                            )
                        }),
                )
                .child(palette.label(status).text_size(gpui::px(11.))),
        );
        if crate::schedule::SUPPORTED && !self.fixture {
            let background = self.preferences.background_updates;
            if enabled && !background && self.daily_schedule_error.is_none() {
                controls = controls.child(palette.label("Keep Astral open for daily updates."));
            }
            if let Some(error) = &self.daily_schedule_error {
                controls = controls
                    .child(palette.label(error.clone()).text_color(rgb(palette.error)))
                    .child(
                        div().child(palette.button("retry-daily-background", "Retry").on_click(
                            cx.listener(move |app, _, _, cx| {
                                app.configure_daily_schedule(background, cx)
                            }),
                        )),
                    );
            }
        }
        if let Some(error) = &self.daily_run_error {
            controls = controls.child(palette.label(error.clone()).text_color(rgb(palette.error)));
        }
        controls
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn updated_addon() -> batch::DailyUpdate {
        batch::DailyUpdate {
            name: "Example".into(),
            from: "1.0".into(),
            to: "1.1".into(),
            notes: "Example release notes".into(),
        }
    }

    #[test]
    fn background_run_skips_a_busy_worker_and_a_client_checked_today() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("preferences.json");
        let root = temp.path().join("_retail_");
        let mut prefs = preferences::Preferences {
            daily_updates: true,
            background_updates: true,
            folders: [Some(root.clone()), None],
            ..Default::default()
        };
        prefs.save(&path)?;
        let busy = lock(&path)?;
        run_background(&path, [None, None])?;
        assert!(!reports_path(&path).exists());
        drop(busy);

        prefs.daily_runs[0] = Some(report(root, Ok(batch::DailyCheck::default())));
        prefs.save(&path)?;
        run_background(&path, [None, None])?;
        assert!(!reports_path(&path).exists());
        assert!(read_reports(&path, &prefs.daily_runs)?.latest[0].is_some());
        Ok(())
    }

    #[test]
    fn forced_check_runs_both_clients_before_they_are_due_and_keeps_exclusions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        let targets = [
            (
                Client::Retail,
                temp.path().join("_retail_"),
                vec!["BigWigs".into()],
            ),
            (
                Client::Forever,
                temp.path().join("_forever_"),
                vec!["Questie".into()],
            ),
        ];
        let latest = targets
            .each_ref()
            .map(|(_, root, _)| Some(report(root.clone(), Ok(batch::DailyCheck::default()))));
        let mut reports = Reports::from_latest(latest);
        run_checks(&path, &mut reports, targets.to_vec(), false, |_, _, _| {
            panic!("neither installation is due");
        })
        .unwrap();
        let mut checked = Vec::new();
        run_checks(
            &path,
            &mut reports,
            targets.to_vec(),
            true,
            |root, client, excluded| {
                checked.push((client, root.to_path_buf(), excluded.to_vec()));
                Ok(batch::DailyCheck {
                    updates: vec![updated_addon()],
                    errors: Vec::new(),
                })
            },
        )
        .unwrap();
        assert!(checked == targets);
        let saved = read_reports(&path, &[None, None]).unwrap();
        assert!(
            saved
                .latest
                .iter()
                .all(|run| run.as_ref().unwrap().updated == 1)
        );
        assert_eq!(saved.history.len(), 4);
        let groups = grouped_history(&saved.history);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].entries.len(), 2);
        assert!(groups[0].entries[0].client == Client::Retail);
        assert!(groups[0].entries[1].client == Client::Forever);
        assert_eq!(groups[0].updated(), 2);
        assert_eq!(groups[0].attention(), 0);
        assert!(!groups[0].deferred());
        assert_eq!(saved.history[0].run.updates[0].to, "1.1");
        assert_eq!(
            saved.history[0].run.updates[0].notes,
            "Example release notes"
        );
    }

    #[test]
    fn opening_wow_mid_check_keeps_completed_updates_and_stops_before_other_client() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        let targets = vec![
            (Client::Retail, temp.path().join("_retail_"), Vec::new()),
            (Client::Forever, temp.path().join("_forever_"), Vec::new()),
        ];
        let mut reports = Reports::from_latest([None, None]);
        let result = run_checks(&path, &mut reports, targets, true, |_, client, _| {
            assert!(client == Client::Retail);
            Ok(batch::DailyCheck {
                updates: vec![updated_addon()],
                errors: vec!["Close World of Warcraft before changing addon files.".into()],
            })
        });
        assert!(result.as_ref().is_err_and(|error| game_open(error)));
        let saved = read_reports(&path, &[None, None]).unwrap();
        let retail = saved.latest[0].as_ref().unwrap();
        assert_eq!(retail.updated, 1);
        assert!(retail.next_at <= now() + 60);
        assert!(saved.latest[1].is_none());
        let groups = grouped_history(&saved.history);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].updated(), 1);
        assert!(groups[0].deferred());
    }

    #[test]
    fn opening_wow_before_second_client_preserves_first_client_and_defers_the_check() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        let targets = vec![
            (Client::Retail, temp.path().join("_retail_"), Vec::new()),
            (Client::Forever, temp.path().join("_forever_"), Vec::new()),
        ];
        let mut reports = Reports::from_latest([None, None]);
        let result = run_checks(&path, &mut reports, targets, true, |_, client, _| {
            if client == Client::Forever {
                return Err("WoW started before changing addon files.".into());
            }
            Ok(batch::DailyCheck {
                updates: vec![updated_addon()],
                errors: Vec::new(),
            })
        });
        assert!(result.as_ref().is_err_and(|error| game_open(error)));
        let saved = read_reports(&path, &[None, None]).unwrap();
        let retail = saved.latest[0].as_ref().unwrap();
        assert_eq!(retail.updated, 1);
        assert!(retail.deferred);
        assert!(retail.next_at <= now() + 60);
        assert!(saved.latest[1].is_none());
        let groups = grouped_history(&saved.history);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].updated(), 1);
        assert!(groups[0].deferred());
    }

    #[test]
    fn retention_keeps_fourteen_whole_checks_without_merging_equal_timestamps() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("preferences.json");
        let mut reports = Reports::from_latest([None, None]);
        for _ in 0..15 {
            let mut check = reports.begin_check()?;
            check.started_at = 1234;
            for client in Client::ALL {
                let mut run = report(
                    temp.path().join(client.index().to_string()),
                    Ok(batch::DailyCheck {
                        updates: vec![updated_addon()],
                        errors: Vec::new(),
                    }),
                );
                run.checked_at = 1234;
                reports.record(&path, client, run, check)?;
            }
        }
        let saved = read_reports(&path, &[None, None])?;
        let groups = grouped_history(&saved.history);
        assert_eq!(saved.history.len(), 28);
        assert_eq!(groups.len(), 14);
        assert!(
            groups
                .iter()
                .all(|group| group.entries.len() == 2 && group.updated() == 2)
        );
        assert!(groups.iter().all(|group| group.checked_at() == 1234));
        assert_eq!(groups[0].entries[0].run.check_id, Some(15));
        assert_eq!(groups[13].entries[0].run.check_id, Some(2));

        // Older report files have no counter; retained IDs still prevent collisions.
        let mut json = serde_json::to_value(&saved)?;
        json.as_object_mut().unwrap().remove("next_check");
        preferences::save_json(&reports_path(&path), &json)?;
        let mut migrated = read_reports(&path, &[None, None])?;
        assert_eq!(migrated.begin_check()?.id, 16);
        Ok(())
    }

    #[test]
    fn legacy_entries_remain_separate_even_with_identical_timestamps() {
        let entries: Vec<_> = Client::ALL
            .into_iter()
            .map(|client| {
                let mut run = report(PathBuf::from("example"), Ok(batch::DailyCheck::default()));
                run.checked_at = 1234;
                // Zero is also treated as an absent ID for old or partial report files.
                run.check_id = Some(0);
                HistoryEntry { client, run }
            })
            .collect();
        let groups = grouped_history(&entries);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|group| group.entries.len() == 1));
    }

    #[test]
    fn existing_report_keeps_schedule_and_count_when_history_is_added() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("preferences.json");
        let root = temp.path().join("_forever_");
        let deadline = now() + 86_400;
        preferences::save_json(
            &reports_path(&path),
            &serde_json::json!([
                null,
                {"root": root, "next_at": deadline, "updated": 4, "errors": []}
            ]),
        )?;
        let mut reports = read_reports(&path, &[None, None])?;
        assert!(!is_due(reports.latest[1].as_ref(), &root));
        assert_eq!(reports.history[0].run.updated, 4);
        for _ in 0..15 {
            let check = reports.begin_check()?;
            reports.record(
                &path,
                Client::Retail,
                report(root.clone(), Ok(batch::DailyCheck::default())),
                check,
            )?;
        }
        let saved = read_reports(&path, &[None, None])?;
        assert_eq!(saved.latest[1].as_ref().unwrap().next_at, deadline);
        assert_eq!(saved.history.len(), 14);
        Ok(())
    }
}
