use crate::{
    Astral, Palette,
    cloud::{self, Overview, Service, Session},
    credentials,
};
use gpui::{Context, Entity, FocusHandle, Window, div, prelude::*, px, rgb};
use std::{
    error::Error,
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[path = "cloud_profiles_view.rs"]
mod profiles;

#[derive(Clone)]
struct HistoryScope {
    client: crate::Client,
    root: PathBuf,
    account: String,
    profile: crate::settings_backup::Profile,
}

#[derive(Clone)]
struct HistoryRestore {
    scope: HistoryScope,
    point: crate::sync_engine::RestorePoint,
    label: String,
    version: u64,
}

fn cloud_backup_date(created: &str) -> String {
    let Ok(date) =
        time::OffsetDateTime::parse(created, &time::format_description::well_known::Rfc3339)
    else {
        return "Unknown date".into();
    };
    if let Some(formatted) = crate::local_time::format_medium_short(date) {
        return formatted;
    }
    let local =
        crate::local_time::offset_at(date).and_then(|offset| date.checked_to_offset(offset));
    let date = local.unwrap_or_else(|| date.to_offset(time::UtcOffset::UTC));
    format!(
        "{} {}, {}, {:02}:{:02}{}",
        &date.month().to_string()[..3],
        date.day(),
        date.year(),
        date.hour(),
        date.minute(),
        if local.is_some() { "" } else { " UTC" },
    )
}

fn cloud_backup_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KB", (bytes + 512) / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.),
    }
}

pub(super) struct PageFocus {
    manage_data: FocusHandle,
    delete_data: FocusHandle,
    shared_sync: FocusHandle,
    shared_refresh: FocusHandle,
    devices_refresh: FocusHandle,
    profiles_manage: FocusHandle,
    rows: std::cell::RefCell<std::collections::HashMap<(&'static str, String), FocusHandle>>,
}

impl PageFocus {
    pub(super) fn new(cx: &gpui::App) -> Self {
        Self {
            manage_data: cx.focus_handle().tab_stop(true),
            delete_data: cx.focus_handle().tab_stop(true),
            shared_sync: cx.focus_handle().tab_stop(true),
            shared_refresh: cx.focus_handle().tab_stop(true),
            devices_refresh: cx.focus_handle().tab_stop(true),
            profiles_manage: cx.focus_handle().tab_stop(true),
            rows: Default::default(),
        }
    }

    fn row(&self, kind: &'static str, id: &str, cx: &gpui::App) -> FocusHandle {
        self.rows
            .borrow_mut()
            .entry((kind, id.to_owned()))
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }

    fn retain_rows(&self, state: &State) {
        let mut current = std::collections::HashSet::new();
        if let Some(overview) = &state.overview {
            current.extend(
                overview
                    .history
                    .iter()
                    .map(|backup| ("history", backup.id.clone())),
            );
            current.extend(
                overview
                    .devices
                    .iter()
                    .map(|device| ("device", device.id.clone())),
            );
        }
        for shared in state.shared.iter().flatten() {
            for addon in shared.addons.iter().flatten() {
                for kind in ["install", "zip", "source"] {
                    current.insert((kind, addon.folder.clone()));
                }
            }
        }
        self.rows
            .borrow_mut()
            .retain(|key, _| current.contains(key));
    }
}

#[derive(Default)]
pub struct State {
    loaded: bool,
    credentials_unavailable: bool,
    machine: Option<String>,
    manage_data: bool,
    busy: Option<&'static str>,
    metadata: Metadata,
    metadata_context: Option<AccountContext>,
    metadata_error: Option<String>,
    session: Option<Session>,
    overview: Option<Overview>,
    error: Option<String>,
    revoke: Option<String>,
    sync_conflict: Option<crate::sync_engine::Conflict>,
    sync_missing: Vec<crate::shared_library::MissingAddon>,
    sync_dialog_focus: Option<SyncDialogFocus>,
    sync_message: Option<String>,
    last_sync: Option<i64>,
    last_sync_context: Option<(
        crate::Client,
        PathBuf,
        Option<String>,
        Option<crate::settings_backup::Profile>,
    )>,
    last_sync_request: u64,
    shared: [Option<cloud::SharedLibrary>; 2],
    shared_error: Option<String>,
    shared_message: Option<String>,
    history_scope: Option<HistoryScope>,
    history_restore: Option<HistoryRestore>,
    history_error: Option<String>,
    delete_confirmation: Option<String>,
    delete_input: Option<Entity<crate::search_input::SearchInput>>,
    profiles: profiles::State,
    auto_sync: AutoSync,
}

#[derive(Clone, PartialEq)]
struct AccountContext {
    client: crate::Client,
    root: Option<PathBuf>,
    account: Option<String>,
    profile: Option<crate::settings_backup::Profile>,
    session: String,
}

#[derive(Default)]
struct Metadata {
    request: u64,
    loading: bool,
}

impl State {
    fn begin_work(&mut self, label: &'static str) {
        self.metadata.invalidate();
        self.busy = Some(label);
    }
}

impl Metadata {
    fn current(&self, request: u64) -> bool {
        self.request == request
    }

    fn begin(&mut self) -> u64 {
        self.request = self.request.wrapping_add(1);
        self.loading = true;
        self.request
    }

    fn invalidate(&mut self) {
        self.request = self.request.wrapping_add(1);
        self.loading = false;
    }

    fn finish(&mut self, request: u64) -> bool {
        if !self.current(request) || !self.loading {
            return false;
        }
        self.loading = false;
        true
    }
}

#[cfg(all(test, feature = "headless-ui-tests"))]
mod ui_tests {
    use super::*;
    use crate::{Client, Page, ToolbarMenu, ui_tests::press};
    use gpui::{KeyBinding, Modifiers, TestAppContext, size};

    fn fixture(directory: &std::path::Path, cx: &mut Context<Astral>) -> Astral {
        cx.bind_keys([KeyBinding::new("escape", crate::DismissOverlay, None)]);
        let mut app = Astral::new(
            [None, None],
            Some(directory),
            Some(Client::Retail),
            false,
            cx,
        );
        app.page = Page::Account;
        app.start_cloud_deletion(cx);
        app
    }

    #[gpui::test]
    fn cloud_confirmation_preserves_header_focus_and_captures_dialog_keys(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            assert!(
                app.account
                    .sync_dialog_focus
                    .as_ref()
                    .unwrap()
                    .close
                    .is_focused(window)
            )
        });
        let appearance = cx.debug_bounds("appearance-trigger").unwrap().center();
        cx.simulate_click(appearance, Modifiers::default());
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(app.header_focus.contains_focused(window, cx))
        });
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(
                app.account
                    .sync_dialog_focus
                    .as_ref()
                    .unwrap()
                    .close
                    .is_focused(window)
            )
        });
        press(cx, "shift-tab");
        view.update_in(cx, |app, window, _| {
            assert!(
                app.account
                    .sync_dialog_focus
                    .as_ref()
                    .unwrap()
                    .cloud
                    .is_focused(window)
            )
        });
        let tools = cx.debug_bounds("tools-trigger").unwrap().center();
        cx.simulate_click(tools, Modifiers::default());
        press(cx, "escape");
        assert!(view.read_with(cx, |app, _| app.account.delete_confirmation.is_none()));
        assert!(view.read_with(cx, |app, _| app.menu == Some(ToolbarMenu::Tools)));
    }

    #[gpui::test]
    fn busy_cloud_deletion_still_traps_tab_in_its_editable_confirmation_input(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        view.update(cx, |app, cx| {
            app.account.busy = Some("Fixture operation");
            cx.notify();
        });
        cx.run_until_parked();
        let appearance = cx.debug_bounds("appearance-trigger").unwrap().center();
        cx.simulate_click(appearance, Modifiers::default());
        press(cx, "tab");
        view.update_in(cx, |app, window, cx| {
            assert!(
                app.account
                    .delete_input
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .focus
                    .is_focused(window)
            )
        });
        cx.simulate_input("DELETE");
        assert!(
            view.read_with(cx, |app, _| app.account.delete_confirmation.as_deref()
                == Some("DELETE"))
        );
        press(cx, "shift-tab");
        view.update_in(cx, |app, window, cx| {
            assert!(
                app.account
                    .delete_input
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .focus
                    .is_focused(window)
            )
        });
        press(cx, "escape");
        assert!(view.read_with(cx, |app, _| app.account.delete_confirmation.is_some()));
        // Never activate a destructive button, load credentials or call a service.
    }

    #[gpui::test]
    fn leaving_account_confirmation_keeps_navigation_focus(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        let brand = cx.debug_bounds("astral-brand").unwrap().center();
        cx.simulate_click(brand, Modifiers::default());
        cx.run_until_parked();
        assert!(view.read_with(cx, |app, _| app.page == Page::Library
            && app.account.delete_confirmation.is_none()));
        view.update_in(cx, |app, window, _| {
            assert!(app.brand_focus.is_focused(window))
        });
    }

    #[gpui::test]
    fn account_page_focus_keeps_data_controls_and_device_identity_in_both_themes(
        cx: &mut TestAppContext,
    ) {
        for light in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (view, cx) =
                cx.add_window_view(|_, cx| {
                    let mut app = Astral::new(
                        [None, None],
                        Some(directory.path()),
                        Some(Client::Retail),
                        light,
                        cx,
                    );
                    app.page = Page::Account;
                    app.account.session = Some(
                        serde_json::from_value(serde_json::json!({
                            "token": "a".repeat(43), "name": "Fixture#1234"
                        }))
                        .unwrap(),
                    );
                    app.account.overview = Some(serde_json::from_value(serde_json::json!({
                    "devices": [
                        { "id": "first-fixture", "label": "First fixture", "current": true },
                        { "id": "second-fixture", "label": "Second fixture", "current": false }
                    ], "history": [], "revision": 0
                })).unwrap());
                    app
                });
            cx.simulate_resize(size(px(1250.), px(1600.)));
            cx.run_until_parked();
            let manage = cx.debug_bounds("manage-cloud-data").unwrap();
            view.update_in(cx, |app, window, _| {
                app.account_focus.manage_data.focus(window)
            });
            cx.run_until_parked();
            assert_eq!(manage, cx.debug_bounds("manage-cloud-data").unwrap());
            press(cx, "space");
            assert!(view.read_with(cx, |app, _| app.account.manage_data));
            assert_eq!(manage, cx.debug_bounds("manage-cloud-data").unwrap());
            press(cx, "tab");
            view.update_in(cx, |app, window, cx| {
                assert!(app.account_focus.delete_data.is_focused(window));
                app.account.busy = Some("Fixture operation");
                app.account_focus.manage_data.focus(window);
                cx.notify();
            });
            cx.run_until_parked();
            press(cx, "tab");
            view.update_in(cx, |app, window, cx| {
                assert!(!app.account_focus.delete_data.is_focused(window));
                app.account.busy = None;
                app.account_focus
                    .row("device", "second-fixture", cx)
                    .focus(window);
                cx.notify();
            });
            cx.run_until_parked();
            let before = cx.debug_bounds("revoke-device:second-fixture").unwrap();
            view.update_in(cx, |app, _, cx| {
                app.account.overview.as_mut().unwrap().devices.reverse();
                cx.notify();
            });
            cx.run_until_parked();
            view.update_in(cx, |app, window, cx| {
                assert!(
                    app.account_focus
                        .row("device", "second-fixture", cx)
                        .is_focused(window)
                );
                assert!(
                    !app.account_focus
                        .row("device", "first-fixture", cx)
                        .is_focused(window)
                );
            });
            let after = cx.debug_bounds("revoke-device:second-fixture").unwrap();
            assert_eq!(before.size.width, after.size.width);
            // Moving rows can round their fractional line height to a different
            // physical pixel; the focus identity must still follow the device.
            assert!((before.size.height - after.size.height).abs() <= px(1.));
            press(cx, "enter");
            assert_eq!(
                view.read_with(cx, |app, _| app.account.revoke.clone())
                    .as_deref(),
                Some("second-fixture")
            );
            // The entry point only opens a confirmation. Never confirm remote sign-out.
            view.update_in(cx, |app, _, cx| {
                app.dismiss_sync_dialog(cx);
                app.account
                    .overview
                    .as_mut()
                    .unwrap()
                    .devices
                    .retain(|device| device.id != "second-fixture");
                cx.notify();
            });
            cx.run_until_parked();
            assert!(view.read_with(cx, |app, _| {
                !app.account_focus
                    .rows
                    .borrow()
                    .contains_key(&("device", "second-fixture".into()))
            }));
        }
    }
}

struct SyncDialogFocus {
    close: FocusHandle,
    cloud: FocusHandle,
    local: FocusHandle,
    previous: Option<FocusHandle>,
}

pub(crate) fn dialog_panel(id: &'static str, palette: Palette) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .occlude()
        .w(px(460.))
        .max_w(gpui::relative(0.90))
        .p(px(30.))
        .flex()
        .flex_col()
        .rounded(px(14.))
        .bg(rgb(palette.panel))
        .border_1()
        .border_color(palette.line)
        .shadow(vec![gpui::BoxShadow {
            color: gpui::rgba(0x00000044).into(),
            offset: gpui::point(px(0.), px(20.)),
            blur_radius: px(35.),
            spread_radius: px(0.),
        }])
}

pub(crate) fn sync_dialog_title(title: impl Into<gpui::SharedString>) -> gpui::Div {
    let title: gpui::SharedString = title.into();
    div()
        .mt(px(20.))
        .text_size(px(24.))
        .line_height(px(29.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(crate::typography::tracked(
            title.to_string(),
            24.,
            1.2,
            -0.7,
        ))
}

#[derive(Default)]
struct AutoSync {
    probing: bool,
    was_running: bool,
    last_attempt: Option<Instant>,
    last_issue: Option<String>,
}

impl AutoSync {
    fn report_issue(&mut self, key: String, automatic: bool) -> bool {
        if !automatic {
            return true;
        }
        if self.last_issue.as_ref() == Some(&key) {
            return false;
        }
        self.last_issue = Some(key);
        true
    }

    fn due(&mut self, running: bool, now: Instant) -> bool {
        let just_closed = self.was_running && !running;
        self.was_running = running;
        !running
            && (just_closed
                || self.last_attempt.is_none_or(|last| {
                    now.saturating_duration_since(last) >= Duration::from_secs(300)
                }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_reads_do_not_block_controls_or_survive_a_mutation() {
        let mut state = State::default();
        let old_read = state.metadata.begin();
        assert!(state.metadata.loading);
        assert!(state.busy.is_none());
        state.begin_work("Syncing settings…");
        assert!(!state.metadata.finish(old_read));
        assert_eq!(state.busy, Some("Syncing settings…"));
        assert!(!state.metadata.loading);
        state.busy = None;
        let new_read = state.metadata.begin();
        assert!(!state.metadata.finish(old_read));
        assert!(state.metadata.loading);
        assert!(state.metadata.finish(new_read));
        assert!(
            state.metadata.current(new_read),
            "other sections may finish after the overview"
        );
        assert!(!state.metadata.loading);
        assert!(!state.metadata.finish(new_read));
    }

    #[test]
    fn replacing_a_metadata_read_ignores_the_older_response() {
        let mut metadata = Metadata::default();
        let retail = metadata.begin();
        let forever = metadata.begin();
        assert!(!metadata.finish(retail));
        assert!(metadata.loading);
        assert!(metadata.finish(forever));
        assert!(!metadata.loading);
    }

    #[test]
    fn automatic_sync_reports_changed_issues_without_reopening_dismissed_ones() {
        let mut sync = AutoSync::default();
        assert!(sync.report_issue("manual".into(), false));
        assert!(sync.report_issue("manual".into(), true));
        assert!(!sync.report_issue("manual".into(), true));
        assert!(sync.report_issue("new revision".into(), true));
        assert!(sync.report_issue("new revision".into(), false));
        assert!(!sync.report_issue("new revision".into(), true));
        sync.last_issue = None;
        assert!(sync.report_issue("new revision".into(), true));
    }

    #[test]
    fn automatic_sync_checks_at_startup_after_wow_closes_and_every_five_minutes() {
        let start = Instant::now();
        let mut schedule = AutoSync::default();
        assert!(schedule.due(false, start));
        schedule.last_attempt = Some(start);
        assert!(!schedule.due(false, start + Duration::from_secs(15)));
        assert!(!schedule.due(true, start + Duration::from_secs(30)));
        let closed = start + Duration::from_secs(45);
        assert!(schedule.due(false, closed));
        schedule.last_attempt = Some(closed);
        assert!(!schedule.due(false, closed + Duration::from_secs(299)));
        assert!(schedule.due(false, closed + Duration::from_secs(300)));
        assert!(!schedule.due(true, closed + Duration::from_secs(600)));
    }
}

fn resolve_account(root: &Path, selected: Option<String>) -> io::Result<Option<String>> {
    let accounts = crate::settings_backup::accounts(root)?;
    Ok(selected
        .filter(|name| accounts.contains(name))
        .or_else(|| (accounts.len() == 1).then(|| accounts[0].clone())))
}

fn load_history_scope(
    client: crate::Client,
    root: Option<PathBuf>,
    data: Option<PathBuf>,
    selected: Option<String>,
) -> io::Result<Option<HistoryScope>> {
    let (Some(root), Some(data)) = (root, data) else {
        return Ok(None);
    };
    let Some(account) = resolve_account(&root, selected)? else {
        return Ok(None);
    };
    let store = crate::settings_backup::Store::open(&data, &root, client, &account)?;
    let state = store.profile_state()?;
    Ok(Some(HistoryScope {
        client,
        root,
        account,
        profile: state.active,
    }))
}

fn unauthorized(error: &(dyn Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if error
            .downcast_ref::<cloud::Error>()
            .is_some_and(|error| error.status == Some(401))
            || error
                .downcast_ref::<io::Error>()
                .and_then(io::Error::get_ref)
                .and_then(|inner| inner.downcast_ref::<cloud::Error>())
                .is_some_and(|error| error.status == Some(401))
        {
            return true;
        }
        current = error.source();
    }
    false
}

impl Astral {
    fn begin_account_work(&mut self, label: &'static str) {
        self.account.begin_work(label);
    }

    fn account_context(&self) -> Option<AccountContext> {
        let client = self.preferences.client;
        let root = self.root(client).cloned();
        Some(AccountContext {
            client,
            account: root
                .as_deref()
                .and_then(|root| self.selected_local_account(client, root)),
            profile: root
                .as_deref()
                .and_then(|root| self.loaded_local_profile(client, root)),
            root,
            session: self.account.session.as_ref()?.fingerprint(),
        })
    }

    pub(crate) fn refresh_cloud_context(&mut self, cx: &mut Context<Self>) {
        if self.page == crate::Page::Account
            && self.account.session.is_some()
            && self.account.metadata_context != self.account_context()
        {
            self.refresh_account(None, cx);
        }
    }

    fn account_reply_current(
        &mut self,
        request: u64,
        context: &Option<AccountContext>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.account.metadata.current(request) {
            return false;
        }
        if &self.account_context() != context {
            self.refresh_account(None, cx);
            return false;
        }
        true
    }

    fn refresh_secondary_account_data(
        &mut self,
        request: u64,
        context: Option<AccountContext>,
        session: Session,
        client: crate::Client,
        cx: &mut Context<Self>,
    ) {
        let shared_session = session.clone();
        let shared_context = context.clone();
        let shared = cx
            .background_executor()
            .spawn(async move { Service::new().shared_library(&shared_session, client) });
        cx.spawn(async move |this, cx| {
            let result = shared.await;
            let _ = this.update(cx, |app, cx| {
                if !app.account_reply_current(request, &shared_context, cx) {
                    return;
                }
                match result {
                    Ok(library) => {
                        app.account.shared[client.index()] = Some(library);
                        app.account.shared_error = None;
                    }
                    Err(error) if error.status == Some(401) => {
                        app.expire_account_read(error.to_string(), cx)
                    }
                    Err(error) => app.account.shared_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        let profile_request = self.begin_profile_list_read();
        let profiles = cx
            .background_executor()
            .spawn(async move { Service::new().profiles(&session, client) });
        cx.spawn(async move |this, cx| {
            let result = profiles.await;
            let _ = this.update(cx, |app, cx| {
                if !app.profile_list_reply_current(profile_request) {
                    return;
                }
                if !app.account_reply_current(request, &context, cx) {
                    return;
                }
                match result {
                    Ok(rows) => app.remember_cloud_profiles(client, rows),
                    Err(error) if error.status == Some(401) => {
                        app.expire_account_read(error.to_string(), cx)
                    }
                    Err(error) => app.remember_cloud_profile_error(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn expire_account_read(&mut self, message: String, cx: &mut Context<Self>) {
        if self.credential_cleanup_pending {
            return;
        }
        let owns_busy = self.account.busy.is_none();
        if owns_busy {
            self.begin_account_work("Signing out…");
        } else {
            self.account.metadata.invalidate();
        }
        self.account.session = None;
        self.account.overview = None;
        self.account.revoke = None;
        self.account.shared = [None, None];
        self.account.history_scope = None;
        self.account.metadata_context = None;
        self.account.metadata_error = None;
        self.account.error = Some(message);
        if self.fixture {
            // Reference fixtures may exercise expiry but never a real secret store.
            if owns_busy {
                self.account.busy = None;
            }
            cx.notify();
            return;
        }
        self.credential_cleanup_pending = true;
        let cleanup = cx
            .background_executor()
            .spawn(async { credentials::remove_background() });
        cx.spawn(async move |this, cx| {
            let result = cleanup.await;
            let _ = this.update(cx, |app, cx| {
                app.credential_cleanup_pending = false;
                if owns_busy {
                    app.account.busy = None;
                }
                if let Err(error) = result {
                    app.account.error = Some(error.to_string());
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn open_signed_account_fixture(&mut self) {
        if !self.fixture {
            return;
        }
        self.account.loaded = true;
        self.account.machine = Some(cloud::device_label());
        self.account.session = Some(
            serde_json::from_value(serde_json::json!({
                "token": "fixture-only-not-a-real-session", "name": "AstralPlayer#1234"
            }))
            .expect("fixture session"),
        );
        self.account.overview = Some(serde_json::from_value(serde_json::json!({
            "devices": [
                { "id": "c".repeat(43), "label": "Desktop Mac", "current": true, "created": 1794600000_i64, "last_seen": 1794690000_i64 },
                { "id": "d".repeat(43), "label": "ROG Ally", "current": false, "created": 1794500000_i64, "last_seen": 1794680000_i64 }
            ],
            "history": [
                { "id": "a".repeat(43), "revision": 2, "created": "2026-11-14T20:15:00Z", "bytes": 68981, "files": 69, "device_label": "Desktop Mac", "current": true },
                { "id": "b".repeat(43), "revision": 1, "created": "2026-11-13T18:30:00Z", "bytes": 65536, "files": 68, "device_label": "ROG Ally", "current": false }
            ], "revision": 2, "usage": { "paused": false }
        })).expect("fixture overview"));
        self.account.shared[self.preferences.client.index()] = Some(cloud::SharedLibrary {
            addons: None,
            revision: 0,
        });
        let data = self
            .preferences_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let root = self.root(self.preferences.client).cloned();
        let selected = root
            .as_deref()
            .and_then(|root| self.selected_local_account(self.preferences.client, root));
        self.account.history_scope =
            load_history_scope(self.preferences.client, root, data, selected)
                .ok()
                .flatten();
        self.seed_cloud_profile_fixture();
    }

    pub(crate) fn open_sync_dialog_fixture(&mut self, flag: &str) {
        match flag {
            "--profile-review" => self.open_profile_review_fixture(),
            "--sync-conflict" => {
                self.account.sync_conflict = Some(crate::sync_engine::Conflict::fixture());
            }
            "--sync-missing" => {
                self.account.sync_missing = vec![crate::shared_library::MissingAddon {
                    folder: "ExampleAddon".into(),
                    name: "Example Addon".into(),
                    reason: "Manual install",
                }];
            }
            _ => {}
        }
    }

    pub(crate) fn load_last_sync(&mut self, cx: &mut Context<Self>) {
        self.account.last_sync_request = self.account.last_sync_request.wrapping_add(1);
        let request = self.account.last_sync_request;
        let client = self.preferences.client;
        let (Some(session), Some(root), Some(data)) = (
            self.account.session.clone(),
            self.root(client).cloned(),
            self.preferences_path
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
        ) else {
            self.account.last_sync = None;
            return;
        };
        let selected = self.selected_local_account(client, &root);
        let context = (
            client,
            root.clone(),
            selected.clone(),
            self.loaded_local_profile(client, &root),
        );
        if self.account.last_sync_context.as_ref() != Some(&context) {
            self.account.last_sync = None;
            self.account.last_sync_context = Some(context);
        }
        let root_for_result = root.clone();
        let fingerprint = session.fingerprint();
        let task = cx.background_executor().spawn(async move {
            let Some(account) = resolve_account(&root, selected)? else {
                return Ok::<_, io::Error>(None);
            };
            let store = crate::settings_backup::Store::open(&data, &root, client, &account)?;
            crate::sync_engine::last_sync(&store, store.profile_state()?.active, &session)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if app.account.last_sync_request == request
                    && app.preferences.client == client
                    && app.root(client) == Some(&root_for_result)
                    && app
                        .account
                        .session
                        .as_ref()
                        .is_some_and(|session| session.fingerprint() == fingerprint)
                {
                    app.account.last_sync = result.ok().flatten();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn last_sync_label(&self) -> String {
        let Some(date) = self
            .account
            .last_sync
            .and_then(|ts| time::OffsetDateTime::from_unix_timestamp(ts).ok())
        else {
            return "Not synced yet".into();
        };
        let local =
            crate::local_time::offset_at(date).and_then(|offset| date.checked_to_offset(offset));
        let date = local.unwrap_or(date);
        let month = date.month().to_string();
        format!(
            "{} {}, {:02}:{:02}{}",
            &month[..3],
            date.day(),
            date.hour(),
            date.minute(),
            if local.is_some() { "" } else { " UTC" },
        )
    }
    pub(crate) fn update_sync_dialog_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sync_dialog_open() {
            let opening = self.account.sync_dialog_focus.is_none();
            let focus = self
                .account
                .sync_dialog_focus
                .get_or_insert_with(|| SyncDialogFocus {
                    close: cx.focus_handle().tab_stop(true),
                    cloud: cx.focus_handle().tab_stop(true),
                    local: cx.focus_handle().tab_stop(true),
                    previous: window.focused(cx),
                });
            let cloud_confirmation = self.page == crate::Page::Account
                && (self.account.history_restore.is_some()
                    || self.account.revoke.is_some()
                    || self.account.delete_confirmation.is_some());
            if (opening || !cloud_confirmation)
                && !focus.cloud.is_focused(window)
                && !focus.local.is_focused(window)
                && !(cloud_confirmation && focus.close.is_focused(window))
                && !self
                    .account
                    .delete_input
                    .as_ref()
                    .is_some_and(|input| input.read(cx).focus.is_focused(window))
            {
                if cloud_confirmation {
                    focus.close.focus(window);
                } else {
                    focus.cloud.focus(window);
                }
            }
        } else if let Some(focus) = self.account.sync_dialog_focus.take() {
            focus
                .previous
                .as_ref()
                .unwrap_or(&self.root_focus)
                .focus(window);
        }
    }

    pub(crate) fn dismiss_sync_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        let cloud_confirmation = self.page == crate::Page::Account
            && (self.account.history_restore.is_some()
                || self.account.revoke.is_some()
                || self.account.delete_confirmation.is_some());
        if cloud_confirmation && self.account.busy.is_some() {
            return true;
        }
        let deletion =
            self.page == crate::Page::Account && self.account.delete_confirmation.take().is_some();
        if deletion {
            self.account.delete_input = None;
        }
        let revoke = self.page == crate::Page::Account && self.account.revoke.take().is_some();
        let restore =
            self.page == crate::Page::Account && self.account.history_restore.take().is_some();
        if self.account.sync_conflict.take().is_none()
            && self.account.sync_missing.is_empty()
            && !restore
            && !revoke
            && !deletion
        {
            return false;
        }
        self.account.sync_missing.clear();
        cx.notify();
        true
    }

    fn cloud_confirmation_open(&self) -> bool {
        self.page == crate::Page::Account
            && (self.account.history_restore.is_some()
                || self.account.revoke.is_some()
                || self.account.delete_confirmation.is_some())
    }

    pub(crate) fn cloud_confirmation_key(
        &self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> bool {
        if !self.cloud_confirmation_open() || event.keystroke.key != "tab" {
            return false;
        }
        self.cycle_cloud_confirmation_focus(event.keystroke.modifiers.shift, window, cx);
        true
    }

    pub(crate) fn dismiss_cloud_confirmation(&mut self, cx: &mut Context<Self>) -> bool {
        self.cloud_confirmation_open() && self.dismiss_sync_dialog(cx)
    }

    pub(crate) fn sync_dialog_open(&self) -> bool {
        self.account.sync_conflict.is_some()
            || !self.account.sync_missing.is_empty()
            || (self.page == crate::Page::Account
                && (self.account.history_restore.is_some()
                    || self.account.revoke.is_some()
                    || self.account.delete_confirmation.is_some()))
    }

    pub(crate) fn sync_missing_dialog(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<gpui::Div> {
        if self.account.sync_missing.is_empty() {
            return None;
        }
        let focus = self.account.sync_dialog_focus.as_ref()?;
        Some(
            div().absolute().size_full().top_0().left_0().flex().items_center().justify_center()
                .child(div().id("sync-missing-backdrop").absolute().top_0().left_0().right_0().bottom_0()
                    .bg(gpui::rgba(0x050b08b3)).occlude()
                    .on_click(cx.listener(|app, _, _, cx| { app.dismiss_sync_dialog(cx); })))
                .child(dialog_panel("sync-missing-dialog", palette)
                    .on_key_down(|event, _, cx| {
                        if event.keystroke.key == "tab" {
                            cx.stop_propagation();
                        }
                    })
                    .child(crate::icon("package", 26., palette.accent))
                    .child(sync_dialog_title("Some addons need a source"))
                    .child(palette.label("Your current UI and bindings were kept. Install these addons, then sync again.")
                        .mt(px(13.)).text_size(px(12.)).line_height(px(23.)))
                    .child(div().id("sync-missing-list").mt(px(25.)).max_h(px(250.)).overflow_y_scroll()
                        .children(self.account.sync_missing.iter().map(|addon| {
                            div().flex().items_center().justify_between().gap(px(13.)).py(px(16.))
                                .border_b_1().border_color(palette.line)
                                .child(div().min_w_0().text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(addon.name.clone()))
                                .child(div().text_size(px(14.)).child(addon.reason))
                        })))
                    .child(div().mt(px(25.)).flex().justify_end().child(crate::outlined_button(
                        palette.primary_button("view-shared-addons", "View shared addons")
                            .font_weight(gpui::FontWeight(550.)),
                        palette,
                        &focus.cloud,
                        cx.listener(|app, _: &(), window, cx| {
                            let names = crate::shared_library::missing_names(&app.account.sync_missing);
                            app.dismiss_sync_dialog(cx);
                            app.account.sync_dialog_focus = None;
                            app.navigate(crate::Page::Account, window, cx);
                            app.account.shared_error = Some(format!("Install these addons before syncing settings: {names}."));
                            cx.notify();
                        }),
                    )))),
        )
    }

    pub(crate) fn sync_conflict_dialog(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<gpui::Div> {
        self.account.sync_conflict.as_ref()?;
        let focus = self.account.sync_dialog_focus.as_ref()?;
        let cloud = focus.cloud.clone();
        let local = focus.local.clone();
        let blocked = self.profile_game_running() || self.account.busy.is_some();
        let choice = |button: gpui::Stateful<gpui::Div>, handle: &FocusHandle, value| {
            let button = button
                .font_weight(gpui::FontWeight(550.))
                .track_focus(handle);
            if blocked {
                button.opacity(0.45).cursor_default()
            } else {
                crate::outlined_button(
                    button,
                    palette,
                    handle,
                    cx.listener(move |app, _: &(), _, cx| {
                        if app.fixture {
                            app.dismiss_sync_dialog(cx);
                        } else {
                            app.sync_now(Some(value), cx);
                        }
                    }),
                )
            }
        };
        Some(div().absolute().size_full().top_0().left_0().flex().items_center().justify_center()
            .child(div().id("sync-conflict-backdrop").absolute().top_0().left_0().right_0().bottom_0()
                .bg(gpui::rgba(0x050b08b3)).occlude()
                .on_click(cx.listener(|app, _, _, cx| { app.dismiss_sync_dialog(cx); })))
            .child(dialog_panel("sync-conflict-dialog", palette)
                .on_key_down(move |event, window, cx| {
                    if event.keystroke.key == "tab" {
                        if !blocked && cloud.is_focused(window) {
                            local.focus(window);
                        } else {
                            cloud.focus(window);
                        }
                        cx.stop_propagation();
                    }
                })
                .child(crate::icon("layers", 26., palette.accent))
                .child(sync_dialog_title("Choose which setup to keep"))
                .child(palette.label("Choose what to sync with the selected profile. Astral saves a local backup before applying cloud settings.")
                    .mt(px(13.)).text_size(px(12.)).line_height(px(23.)))
                .when(self.profile_game_running(), |dialog| dialog.child(
                    palette.label("Sync waits for WoW to close.").mt(px(8.)).text_size(px(12.))
                ))
                .child(div().mt(px(25.)).flex().justify_end().gap(px(9.))
                    .child(choice(palette.button("sync-use-cloud", "Use cloud setup"), &focus.cloud, crate::sync_engine::Choice::Cloud))
                    .child(choice(palette.primary_button("sync-use-local", "Use this computer"), &focus.local, crate::sync_engine::Choice::Local)))))
    }
    pub(crate) fn account_name(&self) -> Option<&str> {
        self.account
            .session
            .as_ref()
            .map(|session| session.name.as_str())
    }

    pub(crate) fn close_account_overlays(&mut self) {
        if self.cloud_confirmation_open() {
            self.account.sync_dialog_focus = None;
        }
        self.account.history_restore = None;
        self.account.revoke = None;
        self.account.delete_confirmation = None;
        self.account.delete_input = None;
        self.account.manage_data = false;
    }

    pub fn account_work_in_progress(&self) -> bool {
        self.account.busy.is_some() || self.credential_cleanup_pending
    }

    pub fn start_auto_sync_timer(&self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            loop {
                if this
                    .update(cx, |app, cx| {
                        app.poll_profile_game_status(cx);
                        app.poll_auto_sync(cx);
                    })
                    .is_err()
                {
                    break;
                }
                executor.timer(Duration::from_secs(15)).await;
            }
        })
        .detach();
    }

    fn auto_sync_ready(&self) -> bool {
        !self.fixture
            && !self.root_choice_pending
            && !self.local_profile_sync_pending()
            && !self.app_updates.installing()
            && self.account.busy.is_none()
            && !self.sync_dialog_open()
            && self.account.history_restore.is_none()
            && self.account.delete_confirmation.is_none()
            && self.account.revoke.is_none()
            && !self.account.profiles.editing()
            && self.release_review.is_none()
            && self.removal.is_none()
            && self.batch.is_none()
            && self.project_review.is_none()
            && !self.activity.has_pending_operation()
            && !self.settings_write_in_progress()
            && !self.talent_refreshing
            && !self.daily_running
    }

    fn poll_auto_sync(&mut self, cx: &mut Context<Self>) {
        if !self.auto_sync_ready() || self.account.auto_sync.probing {
            return;
        }
        if self.root(self.preferences.client).is_none()
            || (self.account.loaded && self.account.session.is_none())
        {
            return;
        }
        self.account.auto_sync.probing = true;
        let probe = cx
            .background_executor()
            .spawn(async { crate::installer::game_running() });
        cx.spawn(async move |this, cx| {
            let result = probe.await;
            let _ = this.update(cx, |app, cx| {
                app.account.auto_sync.probing = false;
                if let Ok(running) = result
                    && app.auto_sync_ready()
                {
                    if !running && !app.account.loaded {
                        app.open_account(cx);
                    } else if app.account.session.is_some()
                        && app.account.auto_sync.due(running, Instant::now())
                    {
                        app.sync_settings(None, true, cx);
                    }
                }
            });
        })
        .detach();
    }

    pub fn dismiss_account_confirmation(&mut self, cx: &mut Context<Self>) -> bool {
        if self.page != crate::Page::Account || self.account.busy.is_some() {
            return false;
        }
        let dismissed = self.account.delete_confirmation.take().is_some()
            || self.account.history_restore.take().is_some()
            || self.account.revoke.take().is_some()
            || self.dismiss_cloud_profiles();
        if dismissed {
            cx.notify();
        }
        dismissed
    }

    pub fn open_account(&mut self, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        if self.account_work_in_progress() {
            return;
        }
        if self.account.loaded {
            if self.account.session.is_some() {
                self.refresh_account(None, cx);
                return;
            }
            if !self.account.credentials_unavailable {
                return;
            }
        }
        self.begin_account_work("Opening account…");
        self.account.error = None;
        let task = cx
            .background_executor()
            .spawn(async { credentials::load() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.account.loaded = true;
                app.account.credentials_unavailable = result.is_err();
                app.account.busy = None;
                match result {
                    Ok(session) => {
                        app.account.session = session;
                        if app.page == crate::Page::Account {
                            app.refresh_account(None, cx);
                        } else {
                            app.load_last_sync(cx);
                            app.poll_auto_sync(cx);
                        }
                    }
                    Err(error) => app.account.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        if self.account.session.is_some() || self.account_work_in_progress() {
            return;
        }
        self.account.error = None;
        self.begin_account_work("Starting Battle.net sign-in…");
        let service = Service::new();
        let worker = service.clone();
        let task = cx.background_executor().spawn(async move {
            worker
                .begin(&cloud::device_label())
                .map_err(|error| error.to_string())
        });
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result = async {
                let mut pair = task.await?;
                this.update(cx, |app, cx| {
                    cx.open_url(&pair.url);
                    app.begin_account_work("Finish signing in in your browser…");
                    cx.notify();
                })
                .map_err(|_| "Astral was closed.".to_owned())?;
                let deadline = Instant::now() + Duration::from_secs(245);
                let mut connection_failures = 0;
                loop {
                    executor.timer(Duration::from_secs(2)).await;
                    if Instant::now() >= deadline {
                        return Err("Sign-in expired. Try again.".to_owned());
                    }
                    let service = service.clone();
                    let poll = executor.spawn(async move {
                        let result = service.poll(&pair);
                        (pair, result)
                    });
                    let (returned, result) = poll.await;
                    let session = match result {
                        Ok(session) => {
                            connection_failures = 0;
                            session
                        }
                        Err(error) if error.status.is_none() && connection_failures < 2 => {
                            connection_failures += 1;
                            pair = returned;
                            continue;
                        }
                        Err(error) => return Err(error.to_string()),
                    };
                    if let Some(session) = session {
                        let worker = Service::new();
                        return executor
                            .spawn(async move {
                                if let Err(error) = credentials::save(&session) {
                                    let _ = worker.logout(&session);
                                    return Err(error.to_string());
                                }
                                Ok(session)
                            })
                            .await;
                    }
                    pair = returned;
                }
            }
            .await;
            let _ = this.update(cx, |app, cx| {
                app.account.busy = None;
                match result {
                    Ok(session) => {
                        app.account.session = Some(session);
                        app.account.credentials_unavailable = false;
                        app.refresh_account(None, cx);
                    }
                    Err(error) => app.account.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_account(&mut self, revoke: Option<String>, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        self.load_last_sync(cx);
        self.account.machine.get_or_insert_with(cloud::device_label);
        if self.account.busy.is_some() {
            return;
        }
        let Some(session) = self.account.session.clone() else {
            return;
        };
        let removed_device = revoke.is_some();
        let context = self.account_context();
        if !removed_device
            && self.account.metadata.loading
            && self.account.metadata_context == context
        {
            return;
        }
        if removed_device {
            self.begin_account_work("Removing device…");
        }
        let request = self.account.metadata.begin();
        if self.account.metadata_context != context {
            self.account.overview = None;
            self.account.history_scope = None;
            self.account.history_error = None;
            self.account.metadata_context = context.clone();
        }
        self.account.error = None;
        self.account.metadata_error = None;
        let client = self.preferences.client;
        let root = self.root(client).cloned();
        let selected = root
            .as_deref()
            .and_then(|root| self.selected_local_account(client, root));
        let data = self
            .preferences_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        self.refresh_secondary_account_data(request, context.clone(), session.clone(), client, cx);
        let task = cx.background_executor().spawn(async move {
            let service = Service::new();
            let (scope, mut history_error) = match load_history_scope(client, root, data, selected)
            {
                Ok(scope) => (scope, None),
                Err(error) => (None, Some(error.to_string())),
            };
            let result = revoke
                .as_deref()
                .map_or(Ok(()), |id| service.revoke_device(&session, id))
                .and_then(|()| {
                    let profile = scope
                        .as_ref()
                        .map(|scope| scope.profile)
                        .unwrap_or_default();
                    match service.overview(&session, client, profile) {
                        Err(error)
                            if error.status == Some(404)
                                && profile != crate::settings_backup::Profile::Regular =>
                        {
                            history_error = Some(error.to_string());
                            service
                                .overview(
                                    &session,
                                    client,
                                    crate::settings_backup::Profile::Regular,
                                )
                                .map(|mut overview| {
                                    overview.history.clear();
                                    overview.revision = 0;
                                    overview
                                })
                        }
                        result => result,
                    }
                });
            (result, scope, history_error)
        });
        cx.spawn(async move |this, cx| {
            let (result, scope, history_error) = task.await;
            let _ = this.update(cx, |app, cx| {
                if !app.account.metadata.finish(request) {
                    return;
                }
                if removed_device {
                    app.account.busy = None;
                }
                if app.account_context() != context {
                    app.refresh_account(None, cx);
                    cx.notify();
                    return;
                }
                match result {
                    Ok(overview) => {
                        if removed_device {
                            app.show_notification("Device signed out.", false, cx);
                        }
                        app.account.overview = Some(overview);
                        app.account.history_scope = scope;
                        app.account.history_error = history_error;
                        app.account.revoke = None;
                    }
                    Err(error) => {
                        if removed_device {
                            app.show_notification(error.to_string(), true, cx);
                        }
                        if error.status == Some(401) {
                            app.expire_account_read(error.to_string(), cx);
                        } else if !removed_device {
                            app.account.metadata_error = Some(error.to_string());
                        } else {
                            app.account.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn sign_out(&mut self, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        if self.account_work_in_progress() {
            return;
        }
        let Some(session) = self.account.session.clone() else {
            return;
        };
        let removed_device = self.account.revoke.is_some();
        self.begin_account_work("Signing out…");
        self.account.error = None;
        let task = cx.background_executor().spawn(async move {
            let revoked = Service::new().logout(&session);
            let local = credentials::remove();
            (revoked, local)
        });
        cx.spawn(async move |this, cx| {
            let (revoked, local) = task.await;
            let _ = this.update(cx, |app, cx| {
                app.account.busy = None;
                match local {
                    Ok(()) => {
                        if removed_device && (revoked.is_ok() || revoked.as_ref().is_err_and(|error| error.status == Some(401))) {
                            app.show_notification("Device signed out.", false, cx);
                        }
                        app.account.session = None;
                        app.account.overview = None;
                        app.account.revoke = None;
                        app.account.sync_conflict = None;
                        app.account.sync_message = None;
                        app.account.shared = [None, None];
                        app.account.shared_error = None;
                        app.account.shared_message = None;
                        app.account.history_scope = None;
                        app.account.history_restore = None;
                        app.account.history_error = None;
                        app.account.delete_confirmation = None;
                        app.account.delete_input = None;
                        if revoked.is_err_and(|error| error.status != Some(401)) {
                            app.account.error = Some("Signed out on this computer. Astral couldn’t confirm that the session was revoked.".into());
                        }
                    }
                    Err(error) => app.account.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn sync_now(&mut self, choice: Option<crate::sync_engine::Choice>, cx: &mut Context<Self>) {
        self.sync_settings(choice, false, cx);
    }

    fn sync_settings(
        &mut self,
        choice: Option<crate::sync_engine::Choice>,
        automatic: bool,
        cx: &mut Context<Self>,
    ) {
        if self.fixture {
            return;
        }
        if self.account.busy.is_some()
            || self.addon_batch_running()
            || (choice.is_some() && self.profile_game_running())
        {
            return;
        }
        let Some(session) = self.account.session.clone() else {
            return;
        };
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            self.account.error = Some("Choose a WoW folder in Settings first.".into());
            cx.notify();
            return;
        };
        let Some(data) = self
            .preferences_path
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        let resolve = choice.and_then(|choice| {
            self.account
                .sync_conflict
                .clone()
                .map(|conflict| (conflict, choice))
        });
        if choice.is_some() && resolve.is_none() {
            return;
        }
        let selected = self.selected_local_account(client, &root);
        let other_client = if client == crate::Client::Retail {
            crate::Client::Forever
        } else {
            crate::Client::Retail
        };
        let other = self.root(other_client).map(|root| {
            (
                root.clone(),
                self.selected_local_account(other_client, root),
            )
        });
        let catalog = self.catalog.clone();
        self.account.auto_sync.last_attempt = Some(Instant::now());
        self.begin_account_work(if automatic {
            "Checking connected WoW settings…"
        } else {
            "Syncing WoW settings…"
        });
        if !automatic {
            self.account.error = None;
            self.account.sync_message = None;
            self.account.sync_conflict = None;
        }
        let operation_root = root.clone();
        let task = cx.background_executor().spawn(async move {
            let root = operation_root;
            let service = Service::new();
            let sync_other = || {
                other.as_ref().map(|(root, selected)| {
                    crate::shared_library::sync_installation(
                        &service, &session, root, other_client, &catalog, &data, selected.clone(),
                    )
                })
            };
            let mut other_result = None;
            let result = (|| -> Result<crate::sync_engine::Outcome, Box<dyn std::error::Error + Send + Sync>> {
                let account = resolve_account(&root, selected)?;
                let store = account.as_deref().map(|account| crate::settings_backup::Store::open(&data, &root, client, account)).transpose()?;
                if automatic && store.as_ref().map(|store| store.profile_state()).transpose()?.is_some_and(|state| state.sync_pending) {
                    return Ok(crate::sync_engine::Outcome::ProfilePending);
                }
                if crate::installer::game_running().map_err(io::Error::other)? {
                    return Err(io::Error::other("Settings sync waits until WoW is closed.").into());
                }
                other_result = match sync_other() {
                    Some(Err(error)) => return Err(error), result => result,
                };
                if let Some(store) = store {
                    crate::sync_engine::run(&store, &root, client, &catalog, &session, resolve, automatic)
                } else if resolve.is_some() {
                    Err(io::Error::other("Your local WoW account changed. Sync again to review it.").into())
                } else {
                    let has_accounts = !crate::settings_backup::accounts(&root)?.is_empty();
                    let collection = crate::shared_library::sync(&service, &session, &root, client, &catalog, None, None)?;
                    Ok(crate::sync_engine::Outcome::LibraryOnly { collection, has_accounts })
                }
            })();
            let expired = result.as_ref().err().and_then(|error| error.downcast_ref::<cloud::Error>())
                .is_some_and(|error| error.status == Some(401))
                || other_result.as_ref().and_then(|result| result.as_ref().err())
                    .and_then(|error| error.downcast_ref::<cloud::Error>())
                    .is_some_and(|error| error.status == Some(401));
            if expired { let _ = credentials::remove_background(); }
            (result, other_result, expired)
        });
        cx.spawn(async move |this, cx| {
            let (result, other_result, expired) = task.await;
            let _ = this.update(cx, |app, cx| {
                app.account.busy = None;
                if expired {
                    app.account.session = None;
                    app.account.overview = None;
                    app.account.shared = [None, None];
                    app.account.error = Some("Please sign in with Battle.net.".into());
                    cx.notify();
                    return;
                }
                let other_issue = match other_result {
                    Some(Ok(result)) => {
                        app.account.shared[other_client.index()] = Some(result.library);
                        (!result.manual.is_empty()).then(|| {
                            format!(
                                "{} needs {}",
                                other_client.label(),
                                crate::shared_library::missing_names(&result.manual)
                            )
                        })
                    }
                    Some(Err(error)) => Some(format!("{}: {error}", other_client.label())),
                    None => None,
                };
                if app.preferences.client != client || app.root(client) != Some(&root) {
                    cx.notify();
                    return;
                }
                if !matches!(result, Ok(crate::sync_engine::Outcome::ProfilePending)) {
                    app.account.error = other_issue;
                }
                match result {
                    Ok(crate::sync_engine::Outcome::Synced { message, installed }) => {
                        app.account.auto_sync.last_issue = None;
                        app.load_last_sync(cx);
                        if app.page == crate::Page::Account { app.reload_current_profile(cx); }
                        app.account.sync_message = Some(message);
                        if app.page == crate::Page::Account && !automatic {
                            app.refresh_after_change(cx);
                            if app.account.error.is_none() {
                                app.refresh_account(None, cx);
                            }
                        } else if installed > 0 && app.selected.is_none() && app.auto_sync_ready() {
                            app.refresh_after_change(cx);
                        }
                    }
                    Ok(crate::sync_engine::Outcome::LibraryOnly { collection, has_accounts }) => {
                        app.account.shared[client.index()] = Some(collection.library);
                        app.account.auto_sync.last_issue = None;
                        let message = if has_accounts {
                            "Addon collection shared. Choose a WoW account in Settings to sync its settings."
                        } else { "Addon collection shared. Open WoW once, then close it to sync this client’s settings." };
                        app.account.sync_message = Some(message.into());
                        if !automatic { app.show_notification(message, false, cx); }
                    }
                    Ok(crate::sync_engine::Outcome::Conflict(conflict)) => {
                        if app.account.auto_sync.report_issue(conflict.issue_key(), automatic) {
                            app.account.sync_conflict = Some(conflict);
                            app.poll_profile_game_status(cx);
                            if automatic { app.show_notification("Both setups changed. Choose which one to keep.", true, cx); }
                        }
                    }
                    Ok(crate::sync_engine::Outcome::Incomplete(collection)) => {
                        app.account.shared[client.index()] = Some(collection.library);
                        if collection.installed > 0 { app.refresh_after_change(cx); }
                        let key = serde_json::json!([client.label(), root, app.loaded_local_profile(client, &root).map(|p| p.cloud_id()),
                            collection.manual.iter().map(|addon| (&addon.folder, &addon.name, &addon.reason)).collect::<Vec<_>>()]).to_string();
                        if app.account.auto_sync.report_issue(key, automatic) {
                            app.account.sync_missing = collection.manual;
                            if automatic { app.show_notification("Some addons need a connected source. Your current setup was kept.", true, cx); }
                        }
                    }
                    Ok(crate::sync_engine::Outcome::ProfilePending) => {}
                    Err(error) => { let message = error.to_string(); app.account.error = Some(message.clone()); app.show_notification(message, true, cx); }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn install_missing_catalog_addon(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.fixture || !self.can_start_update_batch() {
            return;
        }
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            return;
        };
        let Some(source) = self.catalog.iter().position(|source| {
            source.id == id
                && source.supports(client)
                && (source.bundled == Some(true) || source.repo.is_some())
        }) else {
            return;
        };
        let catalog = self.catalog.clone();
        let preferences = self.preferences_path.clone();
        self.begin_account_work("Installing addon…");
        self.account.shared_error = None;
        self.account.shared_message = None;
        let task = cx.background_executor().spawn(async move {
            let result = crate::batch::install_missing(
                source,
                &root,
                client,
                &catalog,
                preferences.as_deref(),
            );
            crate::activity::record(
                preferences.as_deref(),
                crate::activity::Action {
                    root,
                    client,
                    name: catalog[source].name.clone(),
                    retry_source: Some(catalog[source].id.clone()),
                    from: None,
                    to: None,
                    verb: match &result {
                        Ok(true) => "Installed",
                        Ok(false) => "Already installed",
                        Err(_) => "Failed",
                    }
                    .into(),
                    error: result.as_ref().err().cloned(),
                    time: 0,
                },
            );
            result
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.account.busy = None;
                if let Err(error) = result {
                    app.show_notification(error, true, cx);
                }
                app.refresh_after_change(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn sync_shared_addons(&mut self, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        if self.account.busy.is_some() || self.addon_batch_running() {
            return;
        }
        let Some(session) = self.account.session.clone() else {
            return;
        };
        let Some(data) = self
            .preferences_path
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        let selected_client = self.preferences.client;
        let targets: Vec<_> = [
            selected_client,
            if selected_client == crate::Client::Retail {
                crate::Client::Forever
            } else {
                crate::Client::Retail
            },
        ]
        .into_iter()
        .filter_map(|client| {
            let root = self.root(client)?.clone();
            let account = self.selected_local_account(client, &root);
            Some((client, root, account))
        })
        .collect();
        if targets.is_empty() {
            self.account.shared_error = Some("Choose a WoW folder in Settings first.".into());
            cx.notify();
            return;
        }
        let catalog = self.catalog.clone();
        self.begin_account_work("Syncing shared addons…");
        self.account.shared_error = None;
        self.account.shared_message = None;
        let task = cx.background_executor().spawn(async move {
            let service = Service::new();
            let mut results = Vec::new();
            for (client, root, selected) in targets {
                let result = crate::shared_library::sync_installation(
                    &service, &session, &root, client, &catalog, &data, selected,
                );
                let expired = result
                    .as_ref()
                    .err()
                    .and_then(|error| error.downcast_ref::<cloud::Error>())
                    .is_some_and(|error| error.status == Some(401));
                results.push((client, result));
                if expired {
                    let _ = credentials::remove_background();
                    return (results, true);
                }
            }
            (results, false)
        });
        cx.spawn(async move |this, cx| {
            let (results, expired) = task.await;
            let _ = this.update(cx, |app, cx| {
                app.account.busy = None;
                if expired {
                    app.account.session = None;
                    app.account.overview = None;
                    app.account.shared = [None, None];
                    app.account.error = Some("Please sign in with Battle.net.".into());
                    cx.notify();
                    return;
                }
                let mut issues = Vec::new();
                let mut installed = 0;
                for (client, result) in results {
                    match result {
                        Ok(result) => {
                            installed += result.installed;
                            app.account.shared[client.index()] = Some(result.library);
                            if !result.manual.is_empty() {
                                issues.push(format!(
                                    "{} needs {}",
                                    client.label(),
                                    crate::shared_library::missing_names(&result.manual)
                                ));
                            }
                        }
                        Err(error) => issues.push(format!("{}: {error}", client.label())),
                    }
                }
                if app.page == crate::Page::Account {
                    app.refresh_after_change(cx);
                }
                app.account.shared_error = (!issues.is_empty()).then(|| issues.join(" "));
                app.account.shared_message =
                    (installed > 0).then(|| format!("Installed {installed} shared addons."));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn restore_cloud_history(&mut self, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        if self.addon_batch_running() || self.profile_game_running() {
            return;
        }
        if self.account.busy.is_some() {
            return;
        }
        let (Some(session), Some(request), Some(data)) = (
            self.account.session.clone(),
            self.account.history_restore.clone(),
            self.preferences_path
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
        ) else {
            return;
        };
        let scope = &request.scope;
        if self.preferences.client != scope.client || self.root(scope.client) != Some(&scope.root) {
            self.account.history_restore = None;
            self.account.history_error =
                Some("The WoW folder changed. Refresh cloud backups.".into());
            cx.notify();
            return;
        }
        let selected = self.selected_local_account(scope.client, &scope.root);
        let catalog = self.catalog.clone();
        self.begin_account_work("Restoring cloud backup…");
        self.account.error = None;
        self.account.history_error = None;
        self.account.history_restore = None;
        self.account.sync_conflict = None;
        let task = cx.background_executor().spawn(async move {
            let scope = request.scope;
            let result = (|| -> crate::shared_library::Result<String> {
                if resolve_account(&scope.root, selected)?.as_deref() != Some(&scope.account) {
                    return Err(io::Error::other(
                        "The local WoW account changed. Refresh cloud backups.",
                    )
                    .into());
                }
                let store = crate::settings_backup::Store::open(
                    &data,
                    &scope.root,
                    scope.client,
                    &scope.account,
                )?;
                crate::sync_engine::restore_history(
                    &store,
                    &scope.root,
                    scope.client,
                    &catalog,
                    &session,
                    &request.point,
                )
            })();
            let expired = result
                .as_ref()
                .err()
                .is_some_and(|error| unauthorized(error.as_ref()));
            if expired {
                let _ = credentials::remove_background();
            }
            (result, expired)
        });
        cx.spawn(async move |this, cx| {
            let (result, expired) = task.await;
            let _ = this.update(cx, |app, cx| {
                app.account.busy = None;
                if expired {
                    app.account = State {
                        loaded: true,
                        error: Some("Please sign in with Battle.net.".into()),
                        ..State::default()
                    };
                } else {
                    match result {
                        Ok(message) => {
                            app.show_notification(
                                "Cloud backup restored. Local backup saved.",
                                false,
                                cx,
                            );
                            app.account.sync_message = Some(message);
                            app.reload_current_profile(cx);
                            app.refresh_after_change(cx);
                            app.refresh_account(None, cx);
                        }
                        Err(error) => {
                            let message = error.to_string();
                            app.account.history_error = Some(message.clone());
                            app.show_notification(message, true, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn delete_cloud_data(&mut self, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        if self.account.busy.is_some()
            || self.account.delete_confirmation.as_deref() != Some("DELETE")
        {
            return;
        }
        let Some(session) = self.account.session.clone() else {
            return;
        };
        self.begin_account_work("Deleting cloud data…");
        self.account.error = None;
        let task = cx.background_executor().spawn(async move {
            let result = Service::new().delete_account_data(&session);
            let remove = if result.is_ok()
                || result
                    .as_ref()
                    .is_err_and(|error| error.status == Some(401))
            {
                credentials::remove().err()
            } else {
                None
            };
            (result, remove)
        });
        cx.spawn(async move |this, cx| {
            let (result, credential_error) = task.await;
            let _ = this.update(cx, |app, cx| {
                app.account.busy = None;
                match result {
                    Ok(()) => {
                        app.show_notification(
                            "Cloud data deleted. Your files remain on this computer.",
                            false,
                            cx,
                        );
                        app.account = State {
                            loaded: true,
                            sync_message: Some("Cloud data deleted.".into()),
                            error: credential_error.map(|error| error.to_string()),
                            ..State::default()
                        };
                    }
                    Err(error) if error.status == Some(401) => {
                        app.show_notification(error.to_string(), true, cx);
                        app.account = State {
                            loaded: true,
                            error: Some(error.to_string()),
                            ..State::default()
                        };
                    }
                    Err(error) => {
                        let message = error.to_string();
                        app.account.error = Some(message.clone());
                        app.show_notification(message, true, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn cloud_restore_dialog(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<gpui::Div> {
        if self.page != crate::Page::Account {
            return None;
        }
        let request = self.account.history_restore.as_ref()?;
        let focus = self.account.sync_dialog_focus.as_ref()?;
        let busy = self.account.busy.is_some();
        let blocked = self.profile_game_running() || busy || self.addon_batch_running();
        let restore = palette
            .primary_button("confirm-cloud-restore", "Restore backup")
            .when(blocked, |button| button.opacity(0.45).cursor_default())
            .when(!blocked, |button| {
                crate::outlined_button(
                    button,
                    palette,
                    &focus.local,
                    cx.listener(|app, _: &(), _, cx| app.restore_cloud_history(cx)),
                )
            });
        Some(div().absolute().size_full().top_0().left_0().flex().items_center().justify_center()
            .child(div().id("cloud-restore-backdrop").absolute().top_0().left_0().right_0().bottom_0()
                .bg(gpui::rgba(0x050b08b3)).occlude()
                .on_click(cx.listener(|app, _, _, cx| { app.dismiss_sync_dialog(cx); })))
            .child(dialog_panel("cloud-restore-dialog", palette).relative()
                .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "tab" { app.cycle_cloud_confirmation_focus(event.keystroke.modifiers.shift, window, cx); cx.stop_propagation(); }
                }))
                .child(self.cloud_confirmation_close(palette, cx))
                .child(crate::icon("history", 26., palette.accent))
                .child(sync_dialog_title(format!("Restore version {}?", request.version)))
                .child(palette.label(format!(
                    "Make the backup from {} your current cloud setup and apply it here. Astral saves a local backup first. Other computers will see this restored version on their next sync.", request.label
                )).mt(px(13.)).mb(px(25.)).text_size(px(12.)).line_height(px(22.8)).text_color(rgb(0x8da898)))
                .child(div().flex().justify_end().gap(px(9.))
                    .child(palette.button("cancel-cloud-restore", "Cancel")
                        .when(busy, |button| button.opacity(0.45).cursor_default())
                        .when(!busy, |button| crate::outlined_button(button,
                        palette,
                        &focus.cloud,
                            cx.listener(|app, _: &(), _, cx| { app.dismiss_sync_dialog(cx); }))))
                    .child(restore))))
    }

    fn cloud_history_panel(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let state = &self.account;
        let can_restore =
            !self.profile_game_running() && !self.addon_batch_running() && state.busy.is_none();
        let scope = state.history_scope.as_ref().filter(|scope| {
            scope.client == self.preferences.client && self.root(scope.client) == Some(&scope.root)
        });
        let count = state
            .overview
            .as_ref()
            .map_or(0, |overview| overview.history.len());
        let mut panel = div().flex().flex_col().child(cloud_section_heading(
            "history",
            format!("{} backups ({count})", self.preferences.client.label()),
            palette,
        ));
        if let Some(error) = &state.history_error {
            return panel.child(
                palette
                    .label(error.clone())
                    .text_size(px(11.))
                    .line_height(px(17.6)),
            );
        }
        let Some(overview) = &state.overview else {
            return panel.child(
                palette
                    .label("Loading your cloud setup…")
                    .text_size(px(14.)),
            );
        };
        if overview.history.is_empty() {
            panel = panel.child(
                palette
                    .label("Your first sync will appear here.")
                    .text_size(px(14.)),
            );
        }
        for (index, backup) in overview.history.iter().enumerate() {
            let date = cloud_backup_date(&backup.created);
            let metadata = format!("{date} · {}", cloud_backup_size(backup.bytes.unwrap_or(0)));
            let mut details = backup
                .device_label
                .clone()
                .unwrap_or_else(|| "Earlier backup".into());
            if let Some(count) = backup.files {
                details.push_str(&format!(" · {count} items"));
            }
            let enabled = can_restore && scope.is_some();
            let mut restore = palette
                .text_button(("cloud-history-restore", index), "Restore")
                .text_size(px(11.))
                .line_height(px(13.2))
                .py(px(5.))
                .border_0();
            if let Some(scope) = scope.filter(|_| enabled) {
                let request = HistoryRestore {
                    scope: scope.clone(),
                    point: crate::sync_engine::RestorePoint {
                        profile: scope.profile,
                        revision: overview.revision,
                        id: backup.id.clone(),
                    },
                    label: date,
                    version: backup.revision,
                };
                restore = crate::outlined_button(
                    restore,
                    palette,
                    &self.account_focus.row("history", &backup.id, cx),
                    cx.listener(move |app, _: &(), _, cx| {
                        app.account.history_error = None;
                        app.account.delete_confirmation = None;
                        app.account.delete_input = None;
                        app.account.history_restore = Some(request.clone());
                        cx.notify();
                    }),
                );
            } else {
                restore = restore.opacity(0.45).cursor_default();
            }
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(13.))
                    .py(px(18.))
                    .border_b_1()
                    .border_color(palette.line)
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .min_h(px(16.8))
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .line_height(px(15.6))
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .child(format!("Version {}", backup.revision)),
                                    )
                                    .when(backup.current, |title| {
                                        title.child(cloud_tag("Current", palette))
                                    }),
                            )
                            .child(
                                palette
                                    .label(metadata)
                                    .text_size(px(11.))
                                    .line_height(px(17.6))
                                    .mt(px(5.)),
                            )
                            .child(
                                palette
                                    .label(details)
                                    .text_size(px(11.))
                                    .line_height(px(17.6))
                                    .mt(px(5.)),
                            ),
                    )
                    .child(restore),
            );
        }
        panel.when(self.profile_game_running(), |panel| {
            panel.child(
                palette
                    .label("Restore becomes available when WoW is closed.")
                    .text_size(px(11.))
                    .line_height(px(18.7)),
            )
        })
    }

    fn cloud_devices_panel(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let count = self
            .account
            .overview
            .as_ref()
            .map_or(0, |overview| overview.devices.len());
        let mut panel = div().flex().flex_col().child(cloud_section_heading(
            "monitor",
            format!("Computers ({count})"),
            palette,
        ));
        let busy = self.account.busy.is_some();
        if let Some(overview) = &self.account.overview {
            for (index, device) in overview.devices.iter().enumerate() {
                let id = device.id.clone();
                let label = if device.current {
                    self.account.machine.as_deref().unwrap_or(&device.label)
                } else {
                    &device.label
                };
                panel = panel.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(13.))
                        .py(px(18.))
                        .border_b_1()
                        .border_color(palette.line)
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .min_h(px(16.8))
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .line_height(px(15.6))
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .child(label.to_owned()),
                                        )
                                        .when(device.current, |title| {
                                            title.child(cloud_tag("This computer", palette))
                                        }),
                                )
                                .child(
                                    palette
                                        .label(format!(
                                            "Last seen {}",
                                            cloud_device_date(device.last_seen.or(device.created))
                                        ))
                                        .text_size(px(11.))
                                        .line_height(px(17.6))
                                        .mt(px(5.)),
                                ),
                        )
                        .child(
                            palette
                                .text_button(("revoke-device", index), "Sign out")
                                .debug_selector({
                                    let id = device.id.clone();
                                    move || format!("revoke-device:{id}")
                                })
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(11.))
                                .line_height(px(13.2))
                                .py(px(5.))
                                .border_0()
                                .flex_row_reverse()
                                .child(crate::icon("logout", 13., palette.muted))
                                .when(busy, |button| button.opacity(0.45).cursor_default())
                                .when(!busy, |button| {
                                    crate::outlined_button(
                                        button,
                                        palette,
                                        &self.account_focus.row("device", &device.id, cx),
                                        cx.listener(move |app, _: &(), _, cx| {
                                            app.account.revoke = Some(id.clone());
                                            app.account.history_restore = None;
                                            app.account.delete_confirmation = None;
                                            app.account.delete_input = None;
                                            cx.notify();
                                        }),
                                    )
                                }),
                        ),
                );
            }
        }
        panel
    }

    pub(crate) fn cloud_device_dialog(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<gpui::Div> {
        if self.page != crate::Page::Account {
            return None;
        }
        let id = self.account.revoke.as_ref()?;
        let device = self
            .account
            .overview
            .as_ref()?
            .devices
            .iter()
            .find(|device| &device.id == id)?;
        let current = device.current;
        let id = id.clone();
        let focus = self.account.sync_dialog_focus.as_ref()?;
        let busy = self.account.busy.is_some();
        let message = if current {
            "This computer will need to sign in again. Its local game files remain.".into()
        } else {
            format!(
                "{} will lose access to your cloud setup until it signs in again. Its local game files remain.",
                device.label
            )
        };
        let mut confirm = palette.primary_button("confirm-revoke-device", "Sign out computer");
        if busy {
            confirm = confirm.opacity(0.45).cursor_default();
        } else {
            confirm = crate::outlined_button(
                confirm,
                palette,
                &focus.local,
                cx.listener(move |app, _: &(), _, cx| {
                    if current {
                        app.sign_out(cx);
                    } else {
                        app.refresh_account(Some(id.clone()), cx);
                    }
                }),
            );
        }
        Some(
            div()
                .absolute()
                .size_full()
                .top_0()
                .left_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("device-confirmation-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .bg(gpui::rgba(0x050b08b3))
                        .occlude()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.dismiss_sync_dialog(cx);
                        })),
                )
                .child(
                    dialog_panel("device-confirmation", palette)
                        .relative()
                        .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, window, cx| {
                            if event.keystroke.key == "tab" {
                                app.cycle_cloud_confirmation_focus(
                                    event.keystroke.modifiers.shift,
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                            }
                        }))
                        .child(self.cloud_confirmation_close(palette, cx))
                        .child(crate::icon("monitor", 26., palette.accent))
                        .child(sync_dialog_title("Sign out this computer?"))
                        .child(
                            palette
                                .label(message)
                                .text_color(rgb(0x8da898))
                                .mt(px(13.))
                                .mb(px(25.))
                                .text_size(px(12.))
                                .line_height(px(22.8)),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(px(9.))
                                .child(
                                    crate::outlined_button(
                                        palette.button("cancel-revoke-device", "Cancel"),
                                        palette,
                                        &focus.cloud,
                                        cx.listener(|app, _: &(), _, cx| {
                                            app.dismiss_sync_dialog(cx);
                                        }),
                                    )
                                    .when(busy, |button| button.opacity(0.45).cursor_default()),
                                )
                                .child(confirm),
                        ),
                ),
        )
    }

    fn cloud_confirmation_close(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let button = palette
            .icon_button("close-cloud-confirmation", "close")
            .absolute()
            .top(px(14.))
            .right(px(14.));
        let Some(focus) = &self.account.sync_dialog_focus else {
            return button;
        };
        if self.account.busy.is_some() {
            return button.opacity(0.45).cursor_default();
        }
        crate::outlined_button(
            button,
            palette,
            &focus.close,
            cx.listener(|app, _: &(), _, cx| {
                app.dismiss_sync_dialog(cx);
            }),
        )
    }

    fn cycle_cloud_confirmation_focus(
        &self,
        backwards: bool,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        let Some(focus) = &self.account.sync_dialog_focus else {
            return;
        };
        let busy = self.account.busy.is_some();
        let mut controls = Vec::new();
        if !busy {
            controls.push(focus.close.clone());
        }
        if self.account.delete_confirmation.is_some() {
            if let Some(input) = &self.account.delete_input {
                controls.push(input.read(cx).focus.clone());
            }
        }
        if !busy {
            controls.push(focus.cloud.clone());
        }
        let enabled = if let Some(value) = &self.account.delete_confirmation {
            value == "DELETE"
        } else {
            self.account.revoke.is_some()
                || (!self.profile_game_running() && !self.addon_batch_running())
        };
        if enabled && !busy {
            controls.push(focus.local.clone());
        }
        if controls.is_empty() {
            return;
        }
        crate::focus_visible::dialog_tab(controls.iter(), backwards, window, cx);
        let current = controls.iter().position(|focus| focus.is_focused(window));
        let index = match (current, backwards) {
            (Some(0), true) | (None, true) => controls.len() - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % controls.len(),
            (None, false) => 0,
        };
        controls[index].focus(window);
    }

    fn start_cloud_deletion(&mut self, cx: &mut Context<Self>) {
        if self.account.busy.is_some() {
            return;
        }
        let input = cx.new(crate::search_input::SearchInput::new);
        cx.subscribe(
            &input,
            |app, _, change: &crate::search_input::Changed, cx| {
                if let Some(value) = app.account.delete_confirmation.as_mut() {
                    *value = change.0.clone();
                    cx.notify();
                }
            },
        )
        .detach();
        self.account.delete_input = Some(input);
        self.account.delete_confirmation = Some(String::new());
        self.account.history_restore = None;
        self.account.revoke = None;
        cx.notify();
    }

    fn cloud_data_controls(&self, palette: Palette, stack: bool, cx: &Context<Self>) -> gpui::Div {
        let busy = self.account.busy.is_some();
        let label = if self.account.manage_data {
            "▾ Manage cloud data"
        } else {
            "▸ Manage cloud data"
        };
        div().mt(px(35.)).pt(px(20.)).border_t_1().border_color(palette.line)
            .child(crate::outlined_button(
                palette.text_button("manage-cloud-data", label).debug_selector(|| "manage-cloud-data".into()).line_height(px(13.2)).border_0(), palette,
                    &self.account_focus.manage_data,
                cx.listener(|app, _: &(), _, cx| { app.account.manage_data = !app.account.manage_data; cx.notify(); }),
            ))
            .when(self.account.manage_data, |panel| panel.child(div().py(px(25.)).flex().items_start().justify_between().gap(px(22.))
                .when(stack, |body| body.flex_col())
                .child(palette.label("Remove every saved setup and sign out every computer. Local game files stay in place.")
                    .min_w_0().text_size(px(11.)).line_height(px(19.8)))
                .child(palette.text_button("delete-cloud-data", "Delete cloud data").flex().items_center().gap(px(6.))
                    .text_size(px(10.)).line_height(px(12.)).py(px(5.)).text_color(rgb(0xe6a5ae)).border_0()
                    .flex_row_reverse().child(crate::icon("trash", 14., 0xe6a5ae))
                    .when(busy, |button| button.opacity(0.45).cursor_default())
                    .when(!busy, |button| crate::outlined_button(button, palette, &self.account_focus.delete_data,
                        cx.listener(|app, _: &(), _, cx| app.start_cloud_deletion(cx)))))))
    }

    pub(crate) fn cloud_delete_dialog(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Div> {
        if self.page != crate::Page::Account {
            return None;
        }
        let value = self.account.delete_confirmation.as_ref()?;
        let input = self.account.delete_input.as_ref()?;
        let focus = self.account.sync_dialog_focus.as_ref()?;
        let busy = self.account.busy.is_some();
        let enabled = !busy && value == "DELETE";
        Some(div().absolute().size_full().top_0().left_0().flex().items_center().justify_center()
            .child(div().id("cloud-delete-backdrop").absolute().top_0().left_0().right_0().bottom_0()
                .bg(gpui::rgba(0x050b08b3)).occlude().on_click(cx.listener(|app, _, _, cx| { app.dismiss_sync_dialog(cx); })))
            .child(dialog_panel("cloud-delete-dialog", palette).relative()
                .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "tab" { app.cycle_cloud_confirmation_focus(event.keystroke.modifiers.shift, window, cx); cx.stop_propagation(); }
                }))
                .child(self.cloud_confirmation_close(palette, cx))
                .child(crate::icon("alert", 26., palette.accent))
                .child(sync_dialog_title("Delete your cloud data?"))
                .child(palette.label("This permanently removes all Retail and Forever backups and signs out every computer. Your local files remain. Type DELETE to confirm.")
                    .mt(px(13.)).mb(px(25.)).text_size(px(12.)).line_height(px(22.8)).text_color(rgb(0x8da898)))
                .child(div().mt(px(10.)).p(px(12.)).border_1().border_color(palette.line).rounded(px(5.))
                    .bg(rgb(palette.background)).text_size(px(14.))
                    .child(crate::search_input::field(input, "", palette.muted, cx)))
                .child(div().flex().justify_end().gap(px(9.))
                    .child(palette.button("cancel-cloud-deletion", "Cancel")
                        .when(busy, |button| button.opacity(0.45).cursor_default())
                        .when(!busy, |button| crate::outlined_button(button,
                        palette,
                        &focus.cloud,
                            cx.listener(|app, _: &(), _, cx| { app.dismiss_sync_dialog(cx); }))))
                    .child(palette.danger_button("confirm-cloud-deletion", "Delete cloud data")
                        .when(!enabled, |button| button.opacity(0.45).cursor_default())
                        .when(enabled, |button| crate::outlined_button(button,
                        palette,
                        &focus.local,
                            cx.listener(|app, _: &(), _, cx| app.delete_cloud_data(cx))))))))
    }

    fn shared_addons_panel(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let state = &self.account;
        let client = self.preferences.client;
        let busy = state.busy.is_some();
        let running = self.profile_game_running();
        let action = |button: gpui::Stateful<gpui::Div>| {
            button
                .text_size(px(10.))
                .line_height(px(12.))
                .py(px(5.))
                .border_0()
        };
        let mut panel = div().flex().flex_col().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb(px(20.))
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(15.6))
                        .font_weight(gpui::FontWeight(550.))
                        .child("Shared addons"),
                )
                .child(
                    action(palette.text_button("sync-shared-addons", "Sync addons"))
                        .when(busy || running, |button| {
                            button.opacity(0.45).cursor_default()
                        })
                        .when(!busy && !running, |button| {
                            crate::outlined_button(
                                button,
                                palette,
                                &self.account_focus.shared_sync,
                                cx.listener(|app, _: &(), _, cx| app.sync_shared_addons(cx)),
                            )
                        }),
                )
                .child(
                    action(palette.text_button("reload-shared-addons", "Refresh"))
                        .when(busy, |button| button.opacity(0.45).cursor_default())
                        .when(!busy, |button| {
                            crate::outlined_button(
                                button,
                                palette,
                                &self.account_focus.shared_refresh,
                                cx.listener(|app, _: &(), _, cx| app.refresh_account(None, cx)),
                            )
                        }),
                ),
        );
        if let Some(error) = &state.shared_error {
            return panel.child(
                palette
                    .label(error.clone())
                    .text_size(px(14.))
                    .line_height(px(17.)),
            );
        }
        if let Some(message) = &state.shared_message {
            panel = panel.child(
                palette
                    .label(message.clone())
                    .text_color(rgb(palette.success)),
            );
        }
        let Some(library) = state.shared[client.index()].as_ref() else {
            return panel.child(
                palette
                    .label("Loading library…")
                    .text_size(px(14.))
                    .text_color(rgb(palette.text)),
            );
        };
        let Some(shared) = library.addons.as_deref() else {
            return panel.child(
                palette
                    .label("Addons sync across profiles for this game client.")
                    .text_size(px(14.))
                    .text_color(rgb(palette.text))
                    .line_height(px(17.)),
            );
        };
        let installed: std::collections::HashSet<_> = self
            .addons
            .iter()
            .flat_map(|addon| addon.modules.iter().map(|folder| folder.to_lowercase()))
            .collect();
        let missing: Vec<_> = shared
            .iter()
            .filter(|addon| !installed.contains(&addon.folder.to_lowercase()))
            .collect();
        panel = panel.child(
            palette
                .label(format!(
                    "{} shared · {} missing on this computer",
                    shared.len(),
                    missing.len()
                ))
                .text_size(px(14.))
                .text_color(rgb(palette.text))
                .line_height(px(17.)),
        );
        if running && !missing.is_empty() {
            panel = panel.child(
                palette
                    .label("Close WoW to install addons.")
                    .text_size(px(14.))
                    .text_color(rgb(palette.text)),
            );
        }
        for (index, addon) in missing.into_iter().enumerate() {
            let source = self.catalog.iter().find(|source| {
                source.supports(client) && source.folder.eq_ignore_ascii_case(&addon.folder)
            });
            let mut row = div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(13.))
                .py(px(18.))
                .border_b_1()
                .border_color(palette.line)
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(15.6))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(addon.name.clone()),
                );
            if let Some(source) = source {
                let id = source.id.clone();
                if source.repo.is_some() || source.bundled == Some(true) {
                    let button = palette
                        .text_button(("install-shared-addon", index), "Install")
                        .text_size(px(11.))
                        .py(px(5.))
                        .line_height(px(13.2))
                        .border_0();
                    row = row.child(if self.can_start_update_batch() {
                        crate::outlined_button(
                            button,
                            palette,
                            &self.account_focus.row("install", &addon.folder, cx),
                            cx.listener(move |app, _: &(), _, cx| {
                                app.install_missing_catalog_addon(&id, cx);
                            }),
                        )
                    } else {
                        button.opacity(0.45).cursor_default()
                    });
                } else {
                    let mut actions = div().flex().flex_wrap().gap_2();
                    if let Some(url) = source.source_url.clone() {
                        actions = actions.child(
                            palette
                                .text_button(("source-shared-addon", index), "Download page")
                                .text_size(px(11.))
                                .py(px(5.))
                                .line_height(px(13.2))
                                .border_0()
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    if !app.account_work_in_progress() {
                                        cx.open_url(&url);
                                    }
                                }))
                                .when(busy, |button| button.opacity(0.45).cursor_default())
                                .when(!busy, |button| {
                                    crate::button_outline(
                                        button,
                                        palette,
                                        &self.account_focus.row("source", &addon.folder, cx),
                                    )
                                }),
                        );
                    }
                    row = row.child(
                        actions.child(
                            palette
                                .text_button(("zip-shared-addon", index), "Install from ZIP")
                                .text_size(px(11.))
                                .py(px(5.))
                                .line_height(px(13.2))
                                .border_0()
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    if !app.profile_game_running()
                                        && !app.account_work_in_progress()
                                    {
                                        app.choose_addon_zip(&id, window, cx);
                                    }
                                }))
                                .when(busy || running, |button| {
                                    button.opacity(0.45).cursor_default()
                                })
                                .when(!busy && !running, |button| {
                                    crate::button_outline(
                                        button,
                                        palette,
                                        &self.account_focus.row("zip", &addon.folder, cx),
                                    )
                                }),
                        ),
                    );
                }
            } else {
                row = row.child(palette.label("Manual install"));
            }
            panel = panel.child(row);
        }
        panel
    }

    pub fn account_page(
        &self,
        palette: Palette,
        compact: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let account = &self.account;
        self.account_focus.retain_rows(account);
        let name = self.account_name();
        let signed_in = name.is_some();
        // Fixture sign-in is blocked at the handler; keep its appearance equal
        // to the normal signed-out state for visual comparisons.
        let ready = !self.account_work_in_progress() && !self.settings_write_in_progress();
        let game_running = self.profile_game_running();
        let can_activate = ready && !(signed_in && game_running);
        let identity =
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(35.))
                .py(px(23.))
                .border_t_1()
                .border_color(rgb(palette.text))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(20.))
                        .min_w_0()
                        .child(
                            div()
                                .size(px(70.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(palette.line)
                                .bg(rgb(palette.surface))
                                .text_size(px(33.))
                                .font_weight(gpui::FontWeight(650.))
                                .when_some(name, |avatar, name| {
                                    avatar.child(name.chars().next().unwrap_or('A').to_string())
                                })
                                .when(!signed_in, |avatar| {
                                    avatar.child(crate::icon("cloud", 30., palette.text))
                                }),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .text_size(px(if compact { 25. } else { 29. }))
                                        .line_height(px(if compact { 30. } else { 35. }))
                                        .font_weight(gpui::FontWeight(690.))
                                        .child(crate::typography::tracked(
                                            name.unwrap_or("Battle.net").to_owned(),
                                            if compact { 25. } else { 29. },
                                            1.2,
                                            -1.,
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(7.))
                                        .text_size(px(12.))
                                        .text_color(rgb(palette.muted))
                                        .child(div().size(px(6.)).rounded_full().bg(rgb(
                                            if signed_in { 0x19c587 } else { palette.muted },
                                        )))
                                        .child(if signed_in {
                                            "Connected with Battle.net"
                                        } else {
                                            "Not connected"
                                        }),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap_3()
                        .flex_shrink_0()
                        .child(
                            palette
                                .primary_button(
                                    "account-primary-action",
                                    if signed_in {
                                        "Sync now"
                                    } else if account.busy.is_some() {
                                        "Connecting…"
                                    } else {
                                        "Connect Battle.net"
                                    },
                                )
                                .min_w(px(135.))
                                .when(signed_in, |button| {
                                    button
                                        .flex_row_reverse()
                                        .child(crate::icon("refresh", 16., 0xffffff))
                                })
                                .when(!signed_in, |button| {
                                    button.child(crate::icon("arrow-up-right", 16., 0xffffff))
                                })
                                .when(!can_activate, |button| {
                                    button.opacity(0.45).cursor_default()
                                })
                                .when(can_activate, |button| {
                                    let activate = move |app: &mut Self, cx: &mut Context<Self>| {
                                        if signed_in {
                                            app.sync_now(None, cx);
                                        } else {
                                            app.sign_in(cx);
                                        }
                                    };
                                    button
                                        .focusable()
                                        .tab_stop(true)
                                        .focus(move |style| style.border_color(rgb(palette.text)))
                                        .on_click(
                                            cx.listener(move |app, _, _, cx| activate(app, cx)),
                                        )
                                }),
                        )
                        .when(signed_in, |actions| {
                            actions.child(
                                palette
                                    .label(if game_running {
                                        "Sync waits for WoW to close.".to_owned()
                                    } else {
                                        format!("Last sync · {}", self.last_sync_label())
                                    })
                                    .text_size(px(11.)),
                            )
                        }),
                );
        let mut view = div()
            .flex()
            .flex_col()
            .child(identity)
            .child(div().h(px(1.)).flex_shrink_0().bg(palette.line));
        if let Some(error) = &account.error
            && !self.cloud_profile_manager_open()
        {
            view = view.child(
                palette
                    .label(error.clone())
                    .mt_3()
                    .text_color(rgb(palette.error)),
            );
        }
        if let Some(busy) = account.busy
            && !self.cloud_profile_manager_open()
        {
            view = view.child(palette.label(busy).mt_3());
        }
        let scope = div()
            .w(px(if compact { 225. } else { 265. }))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_0()
            .child(self.account_profile_picker(palette, window, cx).mb(px(18.)))
            .when(signed_in, |scope| {
                scope.child(self.cloud_profiles_panel(palette, cx))
            })
            .when(self.local_profile_sync_pending(), |scope| {
                scope.child(
                    palette
                        .label(if signed_in {
                            "Local profile applied. Select Sync now to review cloud settings."
                        } else {
                            "Local profile applied. Connect Battle.net to sync it."
                        })
                        .text_size(px(11.))
                        .line_height(px(18.7)),
                )
            })
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(15.6))
                    .mb(px(5.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Profile settings"),
            )
            .children(
                [
                    ("settings", "Addon profiles"),
                    ("command", "Keybindings"),
                    ("layers", "Blizzard layouts"),
                ]
                .map(|(glyph, title)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .py(px(17.))
                        .border_b_1()
                        .border_color(palette.line)
                        .text_size(px(13.))
                        .line_height(px(15.6))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(crate::icon(glyph, 19., palette.muted).mt(px(2.)))
                        .child(title)
                }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(13.))
                    .my(px(25.))
                    .child(crate::icon("monitor", 18., palette.muted).mt(px(3.)))
                    .child(
                        palette
                            .label("Graphics and display settings stay on this computer.")
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.))
                            .line_height(px(19.8)),
                    ),
            )
            .child(self.local_inspection_controls(palette, cx));
        let mut cloud = div()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .gap_6()
            .when(signed_in, |cloud| cloud.gap_0())
            .child(self.profile_addon_controls(palette, cx));
        if !signed_in {
            cloud = cloud.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .border_t_1()
                    .border_color(palette.line)
                    .pt_4()
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Cloud backups"),
                    )
                    .child(palette.label(
                        "Sign in to save your settings and restore them on another computer.",
                    )),
            );
            return view.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(44.))
                    .pt(px(25.))
                    .child(scope)
                    .child(cloud),
            );
        }
        let paused = account
            .overview
            .as_ref()
            .and_then(|overview| overview.usage.as_ref())
            .is_some_and(|usage| usage.paused);
        cloud = cloud.child(self.shared_addons_panel(palette, cx));
        cloud = cloud.child(
            div()
                .flex()
                .flex_col()
                .child(
                    div().flex().justify_end().mb(px(12.)).child(
                        palette
                            .text_button("refresh-devices", "Refresh")
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .py(px(5.))
                            .line_height(px(12.))
                            .flex_row_reverse()
                            .child(crate::icon("refresh", 13., palette.muted))
                            .when(!ready, |button| button.opacity(0.45).cursor_default())
                            .when(ready, |button| {
                                crate::outlined_button(
                                    button,
                                    palette,
                                    &self.account_focus.devices_refresh,
                                    cx.listener(|app, _: &(), _, cx| app.refresh_account(None, cx)),
                                )
                            }),
                    ),
                )
                .when_some(account.metadata_error.as_ref(), |panel, error| {
                    panel.child(div().p(px(14.)).mb(px(20.)).rounded(px(8.)).border_1()
                        .border_color(palette.line).bg(rgb(palette.surface))
                        .text_color(rgb(0xd18b35)).text_size(px(11.)).line_height(px(18.7))
                        .child(error.clone()))
                })
                .when(account.overview.is_none() && account.error.is_none() && account.metadata_error.is_none(), |panel| {
                    panel.child(palette.label("Loading your cloud setup…").text_size(px(10.)).line_height(px(12.)))
                })
                .when(account.overview.is_some() && account.metadata_error.is_none(), |panel| {
                    panel
                        .when(paused, |panel| panel.child(palette.label("Cloud sync is temporarily paused. Your saved backups are still available.").text_size(px(14.))))
                        .child(self.cloud_devices_panel(palette, cx))
                        .child(div().mt(px(34.)).child(self.cloud_history_panel(palette, cx)))
                        .child(self.cloud_data_controls(palette, window.viewport_size().width <= px(1000.), cx))
                }),
        );
        view.child(
            div()
                .flex()
                .items_start()
                .gap(px(44.))
                .pt(px(25.))
                .child(scope)
                .child(cloud),
        )
    }
}

fn cloud_section_heading(glyph: &'static str, title: String, palette: Palette) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(9.))
        .mb(px(10.))
        .text_size(px(12.))
        .line_height(px(14.4))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(crate::icon(glyph, 15., palette.text))
        .child(title)
}

fn cloud_tag(title: &'static str, palette: Palette) -> gpui::Div {
    div()
        .ml(px(8.))
        .px(px(5.))
        .py(px(2.))
        .border_1()
        .border_color(palette.line)
        .bg(rgb(palette.surface))
        .rounded(px(4.))
        .text_size(px(9.))
        .line_height(px(10.8))
        .text_color(rgb(palette.muted))
        .font_weight(gpui::FontWeight::NORMAL)
        .child(title)
}

fn cloud_device_date(timestamp: Option<i64>) -> String {
    timestamp
        .and_then(|timestamp| time::OffsetDateTime::from_unix_timestamp(timestamp).ok())
        .and_then(|date| {
            date.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .map(|text| cloud_backup_date(&text))
        .unwrap_or_else(|| "Before this update".into())
}
