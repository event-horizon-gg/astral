use crate::sync_addons::{ProfileAddon, ProfileEdits};
use crate::{
    Astral, Palette,
    settings_backup::{self, Backup, Profile, ProfileReview, ProfileState, Store},
};
use gpui::{Context, FocusHandle, Window, div, prelude::*, px, rgb};

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
        app.page = Page::Settings;
        app.open_profile_review_fixture();
        app.profile_review_focus.return_to = Some(app.brand_focus.clone());
        app
    }

    fn account_fixture(
        directory: &std::path::Path,
        light: bool,
        cx: &mut Context<Astral>,
    ) -> Astral {
        let root = directory.join("_retail_");
        for name in ["ALPHA", "ALPINE", "BETA"] {
            std::fs::create_dir_all(root.join("WTF/Account").join(name).join("SavedVariables"))
                .unwrap();
        }
        let mut app = Astral::new(
            [None, None],
            Some(directory),
            Some(Client::Retail),
            light,
            cx,
        );
        app.page = Page::Settings;
        app.preferences.folders[0] = Some(root);
        app.load_settings_backups(None, cx);
        app
    }

    #[gpui::test]
    fn local_account_prefix_survives_refresh_and_placeholder_cannot_be_selected(
        cx: &mut TestAppContext,
    ) {
        for light in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (view, cx) =
                cx.add_window_view(|_, cx| account_fixture(directory.path(), light, cx));
            cx.simulate_resize(size(px(1250.), px(900.)));
            cx.run_until_parked();
            view.update_in(cx, |app, window, _| app.local_account_focus.focus(window));
            press(cx, "a");
            assert_eq!(
                view.read_with(cx, |app, _| app.settings_backups.account.clone())
                    .as_deref(),
                Some("ALPHA")
            );
            press(cx, "l");
            press(cx, "p");
            press(cx, "i");
            view.update_in(cx, |app, window, _| {
                assert_eq!(app.settings_backups.account.as_deref(), Some("ALPINE"));
                assert_eq!(app.settings_backups.accounts.len(), 3);
                assert!(app.local_account_focus.is_focused(window));
            });
            for choice in [Some(0), None, Some(usize::MAX)] {
                view.update_in(cx, |app, window, cx| {
                    app.await_local_account(std::future::ready(choice), window, cx)
                });
                cx.run_until_parked();
                view.update_in(cx, |app, window, _| {
                    assert_eq!(app.settings_backups.account.as_deref(), Some("ALPINE"));
                    assert!(app.local_account_focus.is_focused(window));
                });
            }
        }
    }

    #[gpui::test]
    fn account_popup_cannot_restore_a_selection_after_the_settings_context_changes(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| account_fixture(directory.path(), false, cx));
        cx.run_until_parked();
        let (choice, finish) = crate::ui_tests::deferred();
        view.update_in(cx, |app, window, cx| {
            app.await_local_account(choice, window, cx);
            app.load_settings_backups(Some("BETA".into()), cx);
            app.brand_focus.focus(window);
        });
        cx.run_until_parked();
        finish(Some(1));
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            assert_eq!(app.settings_backups.account.as_deref(), Some("BETA"));
            assert!(app.settings_backups.account_choice.is_none());
            assert!(app.brand_focus.is_focused(window));
        });
    }

    #[gpui::test]
    fn signed_out_cached_cloud_only_profile_requests_sign_in_instead_of_creating_a_local_profile(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("_retail_");
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = Astral::new(
                [None, None],
                Some(directory.path()),
                Some(Client::Retail),
                false,
                cx,
            );
            app.page = Page::Account;
            app.preferences.folders[0] = Some(root.clone());
            app.settings_backups.account_context = Some((Client::Retail, root.clone()));
            app.settings_backups.account = Some("Fixture".into());
            app.preferences.remember_profile_names(
                crate::preferences::ProfileNamesScope {
                    root: Some(root.clone()),
                    account: Some("Fixture".into()),
                },
                &[
                    crate::cloud::CloudProfile {
                        id: "regular".into(),
                        name: "Keyboard".into(),
                    },
                    crate::cloud::CloudProfile {
                        id: "0123456789abcdef0123456789abcdef".into(),
                        name: "Raid".into(),
                    },
                ],
            );
            app
        });
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            app.profile_review_focus.picker.focus(window)
        });
        press(cx, "enter");
        assert!(view.read_with(cx, |app, _| app.settings_backups.choosing_profile));
        view.update_in(cx, |app, window, _| {
            app.profile_review_focus.picker_options[1].focus(window)
        });
        press(cx, "enter");
        assert!(cx.debug_bounds("notification").is_some());
        view.read_with(cx, |app, _| {
            assert!(!app.settings_backups.choosing_profile);
            assert!(!app.settings_backups.reviewing);
            assert!(app.settings_backups.review.is_none());
            assert_eq!(app.settings_backups.profile.profiles().count(), 2);
        });
        assert!(!root.exists());
    }

    #[gpui::test]
    fn empty_cached_profiles_keep_keyboard_focus_on_the_picker(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("_retail_");
        let (view, cx) = cx.add_window_view(|_, cx| {
            cx.bind_keys([KeyBinding::new("escape", crate::DismissOverlay, None)]);
            let mut app = Astral::new(
                [None, None],
                Some(directory.path()),
                Some(Client::Retail),
                false,
                cx,
            );
            app.page = Page::Account;
            app.preferences.folders[0] = Some(root.clone());
            app.settings_backups.account_context = Some((Client::Retail, root.clone()));
            app.settings_backups.account = Some("Fixture".into());
            app.preferences.remember_profile_names(
                crate::preferences::ProfileNamesScope {
                    root: Some(root.clone()),
                    account: Some("Fixture".into()),
                },
                &[],
            );
            app
        });
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            app.profile_review_focus.picker.focus(window)
        });
        press(cx, "enter");
        let unavailable = cx
            .debug_bounds("unavailable-active-profile")
            .unwrap()
            .center();
        cx.simulate_click(unavailable, Modifiers::default());
        for key in ["down", "up", "a"] {
            press(cx, key);
            view.update_in(cx, |app, window, _| {
                assert!(app.settings_backups.choosing_profile);
                assert!(app.profile_review_focus.picker.is_focused(window));
                assert_eq!(app.settings_backups.profile.active, Profile::Regular);
            });
        }
        press(cx, "escape");
        assert!(!view.read_with(cx, |app, _| app.settings_backups.choosing_profile));
        assert!(!root.exists());
    }

    #[gpui::test]
    fn profile_review_focus_runs_once_and_captures_header_tab_and_escape(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            assert!(app.profile_review_focus.cancel.is_focused(window))
        });
        let appearance = cx.debug_bounds("appearance-trigger").unwrap().center();
        cx.simulate_click(appearance, Modifiers::default());
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(app.header_focus.contains_focused(window, cx));
        });
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.profile_review_focus.cancel.is_focused(window))
        });
        press(cx, "shift-tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.profile_review_focus.apply.is_focused(window))
        });
        let tools = cx.debug_bounds("tools-trigger").unwrap().center();
        cx.simulate_click(tools, Modifiers::default());
        press(cx, "escape");
        assert!(view.read_with(cx, |app, _| !app.local_profile_review_open()));
        assert!(view.read_with(cx, |app, _| app.menu == Some(ToolbarMenu::Tools)));
        view.update_in(cx, |app, window, _| {
            assert!(app.brand_focus.is_focused(window))
        });
    }

    #[gpui::test]
    fn busy_profile_review_ignores_cancel_and_escape(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        view.update(cx, |app, cx| {
            app.settings_backups.writing = true;
            cx.notify();
        });
        cx.run_until_parked();
        let cancel = cx.debug_bounds("cancel-profile-review").unwrap().center();
        cx.simulate_click(cancel, Modifiers::default());
        assert!(view.read_with(cx, |app, _| app.local_profile_review_open()));
        press(cx, "escape");
        press(cx, "tab");
        assert!(view.read_with(cx, |app, _| app.local_profile_review_open()));
    }

    #[gpui::test]
    fn leaving_profile_review_keeps_the_clicked_header_focused(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        let brand = cx.debug_bounds("astral-brand").unwrap().center();
        cx.simulate_click(brand, Modifiers::default());
        cx.run_until_parked();
        assert!(view.read_with(cx, |app, _| app.page == Page::Library
            && !app.local_profile_review_open()));
        view.update_in(cx, |app, window, _| {
            assert!(app.brand_focus.is_focused(window))
        });
    }

    #[gpui::test]
    fn profile_review_stays_open_until_the_write_reports_its_result(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("_retail_");
        std::fs::create_dir_all(root.join("Interface/AddOns")).unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = fixture(directory.path(), cx);
            app.detected_roots[Client::Retail.index()] = Some(root.clone());
            app.settings_backups.account = Some("MISSING_ACCOUNT".into());
            app
        });
        view.update(cx, |app, cx| {
            app.apply_local_profile_review(cx);
            assert!(app.settings_backups.writing);
            assert!(app.settings_backups.review.is_some());
        });
        cx.run_until_parked();
        view.read_with(cx, |app, _| {
            assert!(!app.settings_backups.writing);
            assert!(app.settings_backups.review.is_none());
            assert!(matches!(&app.settings_backups.status, Some(Err(error)) if error.contains("account was not found")));
        });
        // Store::open fails before process queries, downloads or any game write.
        assert!(!root.join("WTF").exists());
    }

    #[gpui::test]
    fn a_profile_error_is_reported_after_leaving_its_original_context(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("_retail_");
        std::fs::create_dir_all(root.join("Interface/AddOns")).unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = fixture(directory.path(), cx);
            app.detected_roots[Client::Retail.index()] = Some(root.clone());
            app.settings_backups.account = Some("MISSING_ACCOUNT".into());
            app
        });
        cx.simulate_resize(size(px(1250.), px(700.)));
        view.update(cx, |app, cx| {
            app.apply_local_profile_review(cx);
            app.detected_roots = [None, None];
            app.load_settings_backups(None, cx);
            // Changing the displayed context must not release the active write.
            assert!(app.settings_backups.writing);
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |app, _| !app.settings_backups.writing
            && app.settings_backups.status.is_none()));
        assert!(cx.debug_bounds("notification").is_some());
        assert!(!root.join("WTF").exists());
    }
}

pub(crate) struct ProfileReviewFocus {
    pub(crate) picker: FocusHandle,
    picker_options: [FocusHandle; 16],
    files: FocusHandle,
    cancel: FocusHandle,
    apply: FocusHandle,
    pub(crate) return_to: Option<FocusHandle>,
    pub(crate) open: bool,
}

impl ProfileReviewFocus {
    pub(crate) fn new(cx: &mut Context<Astral>) -> Self {
        Self {
            picker: cx.focus_handle().tab_stop(true),
            picker_options: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            files: cx.focus_handle().tab_stop(true),
            cancel: cx.focus_handle().tab_stop(true),
            apply: cx.focus_handle().tab_stop(true),
            return_to: None,
            open: false,
        }
    }

    pub(crate) fn enter(&mut self, window: &mut Window, cx: &Context<Astral>) {
        if !self.open {
            self.open = true;
            if self.return_to.is_none() {
                self.return_to = window.focused(cx);
            }
            // useDialog's selector includes buttons, not the details summary.
            // Its initial-focus effect runs once when the dialog opens.
            self.cancel.focus(window);
        }
    }

    pub(crate) fn leave(&mut self, window: &mut Window, fallback: &FocusHandle) {
        if std::mem::take(&mut self.open) {
            self.return_to
                .take()
                .as_ref()
                .unwrap_or(fallback)
                .focus(window);
        }
    }
}

#[derive(Default)]
pub struct State {
    request: u64,
    accounts: Vec<String>,
    account_choice: Option<uuid::Uuid>,
    account_type_ahead: crate::type_ahead::TypeAhead,
    choosing_profile: bool,
    profile_type_ahead: crate::type_ahead::TypeAhead,
    game_running: bool,
    checking_game: bool,
    account: Option<String>,
    account_context: Option<(crate::Client, std::path::PathBuf)>,
    backups: Vec<Backup>,
    confirmation: Option<Backup>,
    profile: ProfileState,
    review: Option<ProfileReview>,
    reviewing: bool,
    show_review_files: bool,
    addon_rows: Option<Result<Vec<ProfileAddon>, String>>,
    addon_edits: ProfileEdits,
    inspection: Option<Result<(usize, usize), String>>,
    inspecting: bool,
    manage_profiles: bool,
    profile_edit: Option<ProfileEdit>,
    loading: bool,
    writing: bool,
    status: Option<Result<String, String>>,
}

enum Action {
    Backup,
    Restore(String),
    ApplyReview(ProfileReview),
    SaveName(Option<Profile>, String),
    DeleteProfile(Profile),
    Recover,
}

enum ProfileEdit {
    Name(Option<Profile>, String),
    Delete(Profile),
}

fn choice_label(enabled: Option<bool>) -> &'static str {
    match enabled {
        Some(true) => "On",
        Some(false) => "Off",
        None => "Varies by character",
    }
}

impl Astral {
    pub(crate) fn reload_current_profile(&mut self, cx: &mut Context<Self>) {
        let client = self.preferences.client;
        let selected = self
            .root(client)
            .and_then(|root| self.selected_local_account(client, root));
        self.load_settings_backups(selected, cx);
    }

    pub(crate) fn local_profile_sync_pending(&self) -> bool {
        self.settings_backups
            .account_context
            .as_ref()
            .is_some_and(|(client, root)| {
                *client == self.preferences.client && self.root(*client) == Some(root)
            })
            && self.settings_backups.profile.sync_pending
    }

    pub(crate) fn profile_game_running(&self) -> bool {
        self.settings_backups.game_running
    }

    pub(crate) fn poll_profile_game_status(&mut self, cx: &mut Context<Self>) {
        if self.fixture
            || (!matches!(
                self.page,
                crate::Page::Account | crate::Page::Settings | crate::Page::Discover
            ) && self.menu != Some(crate::ToolbarMenu::Account)
                && self.selected.is_none()
                && self.project_review.is_none()
                && self.batch_update_count() == 0
                && !self.sync_dialog_open())
            || self.settings_backups.checking_game
        {
            return;
        }
        self.settings_backups.checking_game = true;
        let task = cx
            .background_executor()
            .spawn(async { crate::installer::game_running() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                let state = &mut app.settings_backups;
                state.checking_game = false;
                if let Ok(running) = result
                    && running != state.game_running
                {
                    state.game_running = running;
                    if running {
                        state.choosing_profile = false;
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn local_account_picker(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let trigger = palette
            .button_base(
                "local-wow-account",
                self.settings_backups
                    .account
                    .clone()
                    .unwrap_or_else(|| "Select account".into()),
            )
            .debug_selector(|| "local-wow-account-trigger".into())
            .relative()
            .h_auto()
            .p(px(8.))
            .cursor_default()
            .font_weight(gpui::FontWeight::NORMAL)
            .bg(rgb(palette.surface))
            .text_color(rgb(palette.text))
            .child(crate::remember_bounds(self.local_account_bounds.clone()))
            .child(crate::icon("chevron", 13., palette.muted));
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(25.))
            .text_size(px(12.))
            .py(px(17.))
            .border_b_1()
            .border_color(palette.line)
            .child("Local WoW account")
            .child(
                crate::outlined_button(
                    trigger,
                    palette,
                    &self.local_account_focus,
                    cx.listener(|app, _: &(), window, cx| app.choose_local_account(window, cx)),
                )
                .capture_key_up(crate::native_select::suppress_button_keys),
            )
    }

    fn choose_local_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture
            || self.page != crate::Page::Settings
            || self.settings_backups.account_choice.is_some()
        {
            return;
        }
        let Some(bounds) = self.local_account_bounds.get() else {
            return;
        };
        let labels = std::iter::once("Select account".to_owned())
            .chain(self.settings_backups.accounts.iter().cloned())
            .collect();
        let selected = self
            .settings_backups
            .account
            .as_ref()
            .and_then(|name| {
                self.settings_backups
                    .accounts
                    .iter()
                    .position(|item| item == name)
            })
            .map_or(0, |index| index + 1);
        match crate::native_select::choose_with_disabled(
            window,
            cx,
            labels,
            selected,
            gpui::point(bounds.left(), bounds.bottom()),
            vec![0],
        ) {
            Ok(choice) => self.await_local_account(choice, window, cx),
            Err(error) => self.show_notification(error, true, cx),
        }
    }

    fn await_local_account(
        &mut self,
        choice: impl std::future::Future<Output = Option<usize>> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.page != crate::Page::Settings || self.settings_backups.account_choice.is_some() {
            return;
        }
        let request = self.settings_backups.request;
        let token = uuid::Uuid::new_v4();
        let client = self.preferences.client;
        let root = self.root(client).cloned();
        let accounts = self.settings_backups.accounts.clone();
        self.settings_backups.account_choice = Some(token);
        cx.spawn_in(window, async move |this, cx| {
            let choice = choice.await;
            let _ = this.update_in(cx, |app, window, cx| {
                if app.settings_backups.account_choice != Some(token) {
                    return;
                }
                app.settings_backups.account_choice = None;
                if app.settings_backups.request == request
                    && app.page == crate::Page::Settings
                    && app.preferences.client == client
                    && app.root(client) == root.as_ref()
                {
                    if let Some(name) = choice
                        .and_then(|index| index.checked_sub(1))
                        .and_then(|index| accounts.get(index))
                    {
                        app.select_local_account(name, cx);
                    }
                    app.local_account_focus.focus(window);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn invalidate_local_account_picker(&mut self) {
        self.settings_backups.account_choice = None;
        self.settings_backups.account_type_ahead.reset();
    }

    fn select_local_account(&mut self, name: &str, cx: &mut Context<Self>) {
        if self
            .settings_backups
            .accounts
            .iter()
            .any(|item| item == name)
            && self.settings_backups.account.as_deref() != Some(name)
        {
            self.load_settings_backups(Some(name.to_owned()), cx);
        }
    }

    pub(crate) fn local_account_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.page != crate::Page::Settings
            || !self.local_account_focus.is_focused(window)
            || self.settings_backups.account_choice.is_some()
        {
            return false;
        }
        let state = &mut self.settings_backups;
        let labels: Vec<_> = std::iter::once("Select account")
            .chain(state.accounts.iter().map(String::as_str))
            .collect();
        let selected = state
            .account
            .as_ref()
            .and_then(|name| state.accounts.iter().position(|item| item == name))
            .map_or(0, |index| index + 1);
        match crate::native_select::closed_key_with_disabled(
            &event.keystroke,
            cfg!(target_os = "macos"),
            &labels,
            selected,
            &mut state.account_type_ahead,
            std::time::Instant::now(),
            &[0],
        ) {
            crate::native_select::KeyResult::Open => self.choose_local_account(window, cx),
            crate::native_select::KeyResult::Select(index) => {
                if let Some(name) = index
                    .checked_sub(1)
                    .and_then(|index| state.accounts.get(index))
                    .cloned()
                {
                    self.select_local_account(&name, cx);
                }
            }
            crate::native_select::KeyResult::Handled => {}
            crate::native_select::KeyResult::Ignore => return false,
        }
        true
    }

    pub(crate) fn inspect_local_settings(&mut self, cx: &mut Context<Self>) {
        let state = &self.settings_backups;
        if state.inspecting
            || state.loading
            || state.game_running
            || self.settings_write_in_progress()
        {
            return;
        }
        let client = self.preferences.client;
        let (Some(root), Some(data), Some(account)) = (
            self.root(client).cloned(),
            self.preferences_path
                .as_deref()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf),
            state.account.clone(),
        ) else {
            return;
        };
        let request = state.request;
        let catalog = self.catalog.clone();
        self.settings_backups.inspecting = true;
        self.settings_backups.inspection = None;
        let task = cx.background_executor().spawn(async move {
            Store::open(&data, &root, client, &account)?.inspect_settings(&catalog)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                let state = &mut app.settings_backups;
                if state.request == request {
                    state.inspecting = false;
                    state.inspection = Some(result.map_err(|error| error.to_string()));
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn local_inspection_controls(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let state = &self.settings_backups;
        let available = state.account.is_some()
            && !state.inspecting
            && !state.loading
            && !state.game_running
            && !self.account_work_in_progress()
            && !self.settings_write_in_progress();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("check-local-settings")
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.))
                    .text_color(rgb(palette.muted))
                    .child(if state.inspecting {
                        "Checking local settings…"
                    } else {
                        "Check local settings"
                    })
                    .child(crate::icon("arrow-right", 14., palette.muted))
                    .when(!available, |link| link.opacity(0.55))
                    .when(available, |link| {
                        link.cursor_pointer()
                            .focusable()
                            .tab_stop(true)
                            .hover(move |style| style.text_color(rgb(palette.accent)))
                            .focus(move |style| style.text_color(rgb(palette.accent)))
                            .on_click(cx.listener(|app, _, _, cx| app.inspect_local_settings(cx)))
                    }),
            )
            .when(state.game_running, |panel| {
                panel.child(
                    palette
                        .label("Close WoW to check saved settings.")
                        .text_size(px(11.)),
                )
            })
            .when_some(state.inspection.as_ref(), |view, result| {
                view.child(match result {
                    Ok((files, bytes)) => palette.label(format!(
                        "{files} settings items · {:.0} KB",
                        *bytes as f64 / 1024.
                    )),
                    Err(error) => palette.label(error.clone()).text_color(rgb(palette.error)),
                })
            })
    }

    pub(crate) fn profile_addon_controls(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let state = &self.settings_backups;
        let count = state.addon_edits.len();
        let busy = self.settings_write_in_progress() || self.account_work_in_progress();
        let blocked = busy || state.game_running;
        let mut panel = div().flex().flex_col().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb(px(20.))
                .gap_3()
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(15.6))
                        .font_weight(gpui::FontWeight(550.))
                        .child("Addons in this profile"),
                )
                .when(count > 0, |row| {
                    row.child(
                        palette
                            .text_button("discard-profile-addons", "Discard changes")
                            .when(busy, |button| button.opacity(0.45).cursor_default())
                            .when(!busy, |button| {
                                button
                                    .focusable()
                                    .tab_stop(true)
                                    .focus(move |style| style.border_color(rgb(palette.accent)))
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.settings_backups.addon_edits.clear();
                                        cx.notify();
                                    }))
                            }),
                    )
                }),
        );
        let Some(rows) = &state.addon_rows else {
            return panel.child(palette.label(if state.loading {
                "Loading addon choices…"
            } else {
                "Open WoW once, then close it and select your local account."
            }));
        };
        let rows = match rows {
            Ok(rows) if !rows.is_empty() => rows,
            Ok(_) => return panel.child(palette.label("No installed addons found.")),
            Err(error) => {
                return panel.child(palette.label(error.clone()).text_color(rgb(palette.error)));
            }
        };
        let mut ordered_rows: Vec<_> = rows.iter().collect();
        ordered_rows.sort_by_key(|choice| {
            self.addons
                .iter()
                .position(|addon| addon.folder == choice.folder)
                .unwrap_or(usize::MAX)
        });
        panel = panel
            .when(state.game_running, |panel| {
                panel.child(
                    palette
                        .label("Close WoW before changing addon choices.")
                        .text_size(px(12.)),
                )
            })
            .child(
                div()
                    .id("profile-addon-list")
                    .max_h(px(360.))
                    .border_t_1()
                    .border_color(palette.line)
                    .overflow_y_scroll()
                    .children(ordered_rows.into_iter().enumerate().map(|(index, addon)| {
                        let value = state
                            .addon_edits
                            .get(&addon.folder)
                            .copied()
                            .or(addon.enabled);
                        let original = addon.enabled;
                        let folder = addon.folder.clone();
                        let editable = !addon.managed && !blocked;
                        let toggle = move |app: &mut Self, cx: &mut Context<Self>| {
                            let next = value != Some(true);
                            if Some(next) == original {
                                app.settings_backups.addon_edits.remove(&folder);
                            } else {
                                app.settings_backups
                                    .addon_edits
                                    .insert(folder.clone(), next);
                            }
                            cx.notify();
                        };
                        div()
                            .id(("profile-addon-choice", index))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(20.))
                            .py(px(12.))
                            .text_size(px(13.))
                            .line_height(px(15.))
                            .border_b_1()
                            .border_color(palette.line)
                            .child(div().child(addon.name.clone()).when(
                                addon.managed || value.is_none(),
                                |label| {
                                    label.child(
                                        palette
                                            .label(if addon.managed {
                                                "Enabled in Handheld only"
                                            } else {
                                                "Varies by character"
                                            })
                                            .mt_1()
                                            .text_size(px(11.))
                                            .line_height(px(13.)),
                                    )
                                },
                            ))
                            .child(
                                palette
                                    .checkbox_mark(value, 17.)
                                    .mt(px(3.))
                                    .mb(px(3.))
                                    .ml(px(4.))
                                    .mr(px(3.))
                                    .when(!editable, |mark| {
                                        if value == Some(false) {
                                            mark.opacity(0.55)
                                        } else {
                                            let gray = if self.preferences.light_theme {
                                                0xcfcfcf
                                            } else {
                                                0x555555
                                            };
                                            mark.bg(rgb(gray)).border_color(rgb(gray))
                                        }
                                    }),
                            )
                            .when(editable, |row| {
                                row.focusable()
                                    .tab_stop(true)
                                    .cursor_pointer()
                                    .focus(move |style| style.border_color(rgb(palette.accent)))
                                    .on_click(cx.listener(move |app, _, _, cx| toggle(app, cx)))
                            })
                    })),
            );
        panel.child(
            div().flex().justify_end().child(
                palette
                    .button(
                        "review-addon-choices",
                        if count == 0 {
                            "Review changes".into()
                        } else {
                            format!("Review changes ({count})")
                        },
                    )
                    .when(count == 0 || blocked, |button| {
                        button.opacity(0.45).cursor_default()
                    })
                    .when(count > 0 && !blocked, |button| {
                        let review =
                            |app: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                                let target = app.settings_backups.profile.active;
                                let edits = app.settings_backups.addon_edits.clone();
                                app.review_profile_changes(target, false, edits, window, cx);
                            };
                        button
                            .focusable()
                            .tab_stop(true)
                            .focus(move |style| style.border_color(rgb(palette.accent)))
                            .on_click(
                                cx.listener(move |app, _, window, cx| review(app, window, cx)),
                            )
                    }),
            ),
        )
    }

    pub(crate) fn review_cached_profile(
        &mut self,
        target: Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .settings_backups
            .profile
            .profiles()
            .any(|profile| profile == target)
        {
            self.show_notification("Sign in with Battle.net first.", true, cx);
            return;
        }
        self.review_local_profile(target, target == Profile::Handheld, window, cx);
    }

    fn review_local_profile(
        &mut self,
        target: Profile,
        preset: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.review_profile_changes(target, preset, ProfileEdits::new(), window, cx);
    }

    fn review_profile_changes(
        &mut self,
        target: Profile,
        preset: bool,
        edits: ProfileEdits,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = &self.settings_backups;
        let client = self.preferences.client;
        let (Some(root), Some(data), Some(account)) = (
            self.root(client).cloned(),
            self.preferences_path
                .as_deref()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf),
            state.account.clone(),
        ) else {
            return;
        };
        if state.loading || state.writing || state.reviewing || state.game_running {
            return;
        }
        let request = state.request;
        let catalog = self.catalog.clone();
        self.settings_backups.reviewing = true;
        self.settings_backups.status = None;
        self.profile_review_focus.return_to = window.focused(cx);
        self.root_focus.focus(window);
        let task = cx.background_executor().spawn(async move {
            Store::open(&data, &root, client, &account)?
                .review_profile(target, preset, &catalog, edits)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                let state = &mut app.settings_backups;
                if state.request != request {
                    return;
                }
                state.reviewing = false;
                state.show_review_files = false;
                match result {
                    Ok(review) => state.review = Some(review),
                    Err(error) => {
                        state.status = Some(Err(error.to_string()));
                        app.profile_review_focus.return_to = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn dismiss_local_profile_review(&mut self, cx: &mut Context<Self>) -> bool {
        let dismissed = self.settings_backups.review.take().is_some();
        if dismissed {
            cx.notify();
        }
        dismissed
    }

    pub(crate) fn open_profile_review_fixture(&mut self) {
        if self.fixture {
            self.settings_backups.review = Some(ProfileReview::fixture(self.preferences.client));
            self.settings_backups.show_review_files = false;
        }
    }

    pub fn local_profile_review(&self, palette: Palette, cx: &Context<Self>) -> Option<gpui::Div> {
        if !matches!(self.page, crate::Page::Settings | crate::Page::Account) {
            return None;
        }
        let state = &self.settings_backups;
        let review = state.review.as_ref()?;
        Some(self.profile_review_dialog(review, state.show_review_files, false, palette, cx))
    }

    pub(crate) fn local_profile_review_open(&self) -> bool {
        matches!(self.page, crate::Page::Settings | crate::Page::Account)
            && self.settings_backups.review.is_some()
    }

    fn profile_review_busy(&self) -> bool {
        self.settings_backups.writing
            || self.account_work_in_progress()
            || self.addon_batch_running()
            || self.root_choice_pending
            || self.daily_running
            || self.scanning
            || self.checking
    }

    fn profile_review_blocked(&self, review: &ProfileReview) -> bool {
        review.blocker.is_some() || self.profile_game_running() || self.profile_review_busy()
    }

    pub(crate) fn profile_review_key(
        &self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &gpui::App,
    ) -> bool {
        if event.keystroke.key != "tab" {
            return false;
        }
        let review = if self.local_profile_review_open() {
            self.settings_backups.review.as_ref()
        } else {
            self.current_cloud_profile_review()
        };
        let Some(review) = review else {
            return false;
        };
        let mut controls = Vec::new();
        if !self.profile_review_busy() {
            controls.push(&self.profile_review_focus.cancel);
            if !self.profile_review_blocked(review) {
                controls.push(&self.profile_review_focus.apply);
            }
        }
        if !controls.is_empty() {
            crate::focus_visible::dialog_tab(
                controls.iter().copied(),
                event.keystroke.modifiers.shift,
                window,
                cx,
            );
            let current = controls.iter().position(|item| item.is_focused(window));
            let index = match (current, event.keystroke.modifiers.shift) {
                (Some(index), false) => (index + 1) % controls.len(),
                (Some(index), true) => (index + controls.len() - 1) % controls.len(),
                (None, false) => 0,
                (None, true) => controls.len() - 1,
            };
            controls[index].focus(window);
        }
        true
    }

    pub(crate) fn dismiss_active_profile_review(&mut self, cx: &mut Context<Self>) -> bool {
        let cloud = self.cloud_profile_review_open();
        if !cloud && !self.local_profile_review_open() {
            return false;
        }
        self.cancel_profile_review(cloud, cx);
        true
    }

    fn apply_profile_review(&mut self, cloud: bool, cx: &mut Context<Self>) {
        if self.fixture {
            self.show_notification(
                "This action is unavailable in the reference fixture.",
                true,
                cx,
            );
            return;
        }
        if self.profile_game_running() || self.profile_review_busy() {
            return;
        }
        if cloud {
            self.apply_cloud_profile_review(cx);
        } else {
            self.apply_local_profile_review(cx);
        }
    }

    fn apply_local_profile_review(&mut self, cx: &mut Context<Self>) {
        if let Some(review) = self.settings_backups.review.clone() {
            self.write_settings_backup(Action::ApplyReview(review), cx);
        }
    }

    pub(crate) fn profile_review_dialog(
        &self,
        review: &ProfileReview,
        show_files: bool,
        cloud: bool,
        palette: Palette,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let retail = self.preferences.client == crate::Client::Retail;
        let has_files = !review.changed.is_empty();
        let busy = self.profile_review_busy();
        let blocked = self.profile_review_blocked(review);
        let files = self.profile_review_focus.files.clone();
        let cancel = self.profile_review_focus.cancel.clone();
        let apply = self.profile_review_focus.apply.clone();
        let toggle_files = move |app: &mut Self, cx: &mut Context<Self>| {
            if cloud {
                app.toggle_cloud_profile_review_files();
            } else {
                app.settings_backups.show_review_files = !app.settings_backups.show_review_files;
            }
            cx.notify();
        };
        let caption = |text: String| {
            palette
                .label(text)
                .text_size(px(12.))
                .line_height(px(22.8))
                .text_color(rgb(0x8da898))
                .mt(px(13.))
                .mb(px(25.))
        };
        div().absolute().size_full().top_0().left_0().flex().items_center().justify_center()
            .child(div().id("profile-review-backdrop").absolute().top_0().left_0().right_0().bottom_0()
                .bg(gpui::rgba(0x050b08b3)).occlude()
                .on_click(cx.listener(move |app, _, _, cx| { app.cancel_profile_review(cloud, cx); })))
            .child(crate::account::dialog_panel("profile-review-dialog", palette)
                .max_h(gpui::relative(0.85)).overflow_y_scroll().block().gap_0()
                .child(crate::account::sync_dialog_title(if review.editing_addons {
                    "Apply addon choices?".into()
                } else { format!("Switch to {}?", review.name) }))
                .when(review.initialized && !review.editing_addons, |dialog| dialog.child(caption("This profile starts with your current setup.".into())))
                .child(caption(format!("{} UI or binding files will change. Your current setup is saved before applying.", review.changed.len())))
                .when(!review.packages.is_empty(), |dialog| dialog.child(caption(format!("Install: {}.", review.packages.join(", ")))))
                .when(review.handheld, |dialog| dialog.child(caption(if retail {
                    "ConsolePort will be enabled and the four desktop action bars hidden."
                } else {
                    "The four desktop action bars will be hidden. The native controller HUD stays visible."
                }.into())))
                .when_some(review.blocker.as_ref(), |dialog, error| dialog.child(caption(error.clone())))
                .when(self.profile_game_running(), |dialog| dialog.child(caption("Close WoW before applying changes.".into())))
                .when(!review.addon_changes.is_empty(), |dialog| dialog.child(
                    div().id("profile-review-addons").block().max_h(px(280.)).overflow_y_scroll()
                        .children(review.addon_changes.iter().map(|change| {
                            div().flex().items_center().justify_between().gap(px(20.)).py(px(16.)).text_size(px(12.)).line_height(px(14.)).border_b_1().border_color(palette.line)
                                .child(div().font_weight(gpui::FontWeight::BOLD).child(change.name.clone()))
                                .child(palette.label(format!("{} → {}", choice_label(change.before), choice_label(change.after))).text_size(px(12.)))
                        }))
                ))
                .when(review.addon_changes.is_empty(), |dialog| dialog.child(
                    caption(if review.packages.is_empty() {
                        "Addon choices stay the same."
                    } else {
                        "Existing addon choices stay the same."
                    }.into()),
                ))
                .when(has_files, |dialog| dialog.child(div().block().mt(px(16.)).child(
                    div().id("toggle-profile-review-files").cursor_pointer().flex().items_center().gap(px(5.))
                        .text_size(px(14.)).line_height(px(17.))
                        .child(if show_files { "▾" } else { "▸" })
                        .child("Changed settings")
                        .track_focus(&files)
                        .focus(move |style| style.bg(rgb(palette.panel)).shadow(vec![gpui::BoxShadow {
                            color: rgb(if cfg!(target_os = "macos") { 0x99c8ff } else { palette.accent }).into(),
                            offset: gpui::point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(2.),
                        }]))
                        .on_click(cx.listener(move |app, _, _, cx| toggle_files(app, cx)))
                )))
                .when(show_files, |dialog| dialog.child(
                    div().id("profile-review-files").block().my(px(14.)).pl(px(24.))
                        .children(review.changed.iter().map(|file| {
                            div().flex().gap(px(12.)).text_size(px(12.)).line_height(px(14.4)).child("•").child(div().min_w_0().child(file.clone()))
                        }))
                ))
                .child(div().flex().justify_end().gap(px(9.))
                    .child(palette.button("cancel-profile-review", "Cancel")
                        .debug_selector(|| "cancel-profile-review".into())
                        .h(px(38.)).font_weight(gpui::FontWeight(550.))
                        .map(|button| crate::button_outline(button, palette, &cancel))
                        .when(busy, |button| button.opacity(0.45).cursor_default())
                        .on_click(cx.listener(move |app, _, _, cx| app.cancel_profile_review(cloud, cx))))
                    .child(palette.primary_button("apply-profile-review", if review.editing_addons { "Apply choices" } else { "Save and switch" })
                        .h(px(38.)).font_weight(gpui::FontWeight(550.))
                        .track_focus(&apply)
                        .when(blocked, |button| button.opacity(0.45).cursor_default())
                        .when(!blocked, |button| crate::button_outline(button, palette, &apply)
                            .on_click(cx.listener(move |app, _, _, cx| app.apply_profile_review(cloud, cx)))))))
    }

    fn cancel_profile_review(&mut self, cloud: bool, cx: &mut Context<Self>) {
        if self.profile_review_busy() {
            return;
        }
        if cloud {
            self.dismiss_cloud_profile_review(cx);
        } else {
            self.dismiss_local_profile_review(cx);
        }
    }

    pub fn settings_write_in_progress(&self) -> bool {
        self.settings_backups.writing
            || self.settings_backups.reviewing
            || (matches!(self.page, crate::Page::Settings | crate::Page::Account)
                && self.settings_backups.review.is_some())
    }

    pub fn dismiss_profile_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let state = &mut self.settings_backups;
        if self.page != crate::Page::Settings || !state.manage_profiles || state.writing {
            return false;
        }
        if state.profile_edit.take().is_none() {
            state.manage_profiles = false;
        }
        state.status = None;
        self.root_focus.focus(window);
        cx.notify();
        true
    }

    pub fn load_settings_backups(&mut self, account: Option<String>, cx: &mut Context<Self>) {
        let same_scope =
            self.settings_backups
                .account_context
                .as_ref()
                .is_some_and(|(client, root)| {
                    *client == self.preferences.client && self.root(*client) == Some(root)
                });
        self.settings_backups.request += 1;
        let request = self.settings_backups.request;
        self.settings_backups.confirmation = None;
        self.settings_backups.review = None;
        self.settings_backups.addon_rows = None;
        self.settings_backups.addon_edits.clear();
        self.settings_backups.inspection = None;
        self.settings_backups.inspecting = false;
        self.settings_backups.reviewing = false;
        self.settings_backups.manage_profiles = false;
        self.settings_backups.profile_edit = None;
        self.settings_backups.profile = ProfileState::default();
        self.settings_backups.backups.clear();
        self.settings_backups.account_choice = None;
        if !same_scope {
            self.settings_backups.accounts.clear();
            self.settings_backups.account_type_ahead.reset();
            self.settings_backups.account = None;
            self.settings_backups.account_context = None;
        }
        self.settings_backups.choosing_profile = false;
        self.settings_backups.loading = false;
        self.settings_backups.status = None;
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            return;
        };
        let account = account.or_else(|| {
            self.preferences
                .local_account(client, &root)
                .map(str::to_owned)
        });
        let Some(data) = self
            .preferences_path
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        self.settings_backups.loading = true;
        let account_context = (client, root.clone());
        let root_for_preferences = root.clone();
        let catalog = self.catalog.clone();
        let task = cx.background_executor().spawn(async move {
            let accounts = settings_backup::accounts(&root)?;
            let account = account
                .filter(|name| accounts.contains(name))
                .or_else(|| (accounts.len() == 1).then(|| accounts[0].clone()));
            let (backups, profile, addons) = match &account {
                Some(name) => {
                    let store = Store::open(&data, &root, client, name)?;
                    (
                        store.list()?,
                        store.profile_state()?,
                        Some(
                            store
                                .profile_addons(&catalog)
                                .map_err(|error| error.to_string()),
                        ),
                    )
                }
                None => (Vec::new(), ProfileState::default(), None),
            };
            Ok::<_, std::io::Error>((accounts, account, backups, profile, addons))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                let state = &mut app.settings_backups;
                if state.request != request {
                    return;
                }
                state.loading = false;
                match result {
                    Ok((accounts, account, backups, profile, addons)) => {
                        state.accounts = accounts;
                        state.account = account.clone();
                        state.account_context = Some(account_context);
                        state.backups = backups;
                        state.profile = profile;
                        state.addon_rows = addons;
                        if let Some(account) = account
                            && app.preferences.local_account(client, &root_for_preferences)
                                != Some(&account)
                        {
                            let previous = app.preferences.local_accounts[client.index()]
                                .replace((root_for_preferences, account));
                            app.save_preferences();
                            if app.preferences_error.is_some() {
                                app.preferences.local_accounts[client.index()] = previous;
                            }
                        }
                    }
                    Err(error) => state.status = Some(Err(error.to_string())),
                }
                app.load_last_sync(cx);
                app.refresh_cloud_context(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn selected_local_account(
        &self,
        client: crate::Client,
        root: &std::path::Path,
    ) -> Option<String> {
        let state = &self.settings_backups;
        state
            .account_context
            .as_ref()
            .filter(|(selected_client, selected_root)| {
                *selected_client == client && selected_root == root
            })
            .and(state.account.clone())
            .or_else(|| {
                self.preferences
                    .local_account(client, root)
                    .map(str::to_owned)
            })
    }

    pub(crate) fn loaded_local_profile(
        &self,
        client: crate::Client,
        root: &std::path::Path,
    ) -> Option<Profile> {
        let state = &self.settings_backups;
        let (loaded_client, loaded_root) = state.account_context.as_ref()?;
        (*loaded_client == client && loaded_root == root && state.account.is_some())
            .then_some(state.profile.active)
    }

    fn write_settings_backup(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.root_choice_pending || self.addon_batch_running() {
            return;
        }
        let state = &self.settings_backups;
        let client = self.preferences.client;
        let (Some(root), Some(data), Some(account)) = (
            self.root(client).cloned(),
            self.preferences_path
                .as_deref()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf),
            state.account.clone(),
        ) else {
            return;
        };
        if state.loading || state.writing {
            return;
        }
        let request = state.request;
        let applying_review = matches!(&action, Action::ApplyReview(_));
        let installing_addons =
            matches!(&action, Action::ApplyReview(review) if !review.packages.is_empty());
        let catalog = self.catalog.clone();
        self.settings_backups.writing = true;
        self.settings_backups.status = None;
        let task = cx.background_executor().spawn(async move {
            let store = Store::open(&data, &root, client, &account)?;
            let result = match action {
                Action::Restore(id) => store
                    .restore(&id, &catalog)
                    .map(|count| format!("Restored {count} settings files.")),
                Action::Backup => store
                    .capture(&catalog)
                    .map(|count| format!("Backed up {count} settings files.")),
                Action::ApplyReview(review) => store.apply_review(&review, &catalog).map(|()| {
                    if review.editing_addons {
                        "Addon choices saved for this profile.".into()
                    } else {
                        format!("{} is active.", review.name)
                    }
                }),
                Action::SaveName(profile, name) => {
                    store.save_profile_name(profile, &name).map(|_| {
                        if profile.is_some() {
                            "Profile renamed.".into()
                        } else {
                            "Profile created.".into()
                        }
                    })
                }
                Action::DeleteProfile(profile) => store
                    .delete_profile(profile)
                    .map(|()| "Profile deleted.".into()),
                Action::Recover => store
                    .recover_profile(&catalog)
                    .map(|()| "Previous profile restored.".into()),
            };
            Ok::<_, std::io::Error>((
                result,
                store.list(),
                store.profile_state(),
                store
                    .profile_addons(&catalog)
                    .map_err(|error| error.to_string()),
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                let feedback = match &result {
                    Ok((Err(error), _, _, _)) => Some(error.to_string()),
                    Err(error) => Some(error.to_string()),
                    _ => None,
                };
                let state = &mut app.settings_backups;
                state.writing = false;
                if state.request == request {
                    if applying_review {
                        state.review = None;
                        state.show_review_files = false;
                    }
                    match result {
                        Ok((outcome, backups, profile, addons)) => {
                            if outcome.is_ok() {
                                state.confirmation = None;
                                state.review = None;
                                state.show_review_files = false;
                                state.profile_edit = None;
                                state.addon_edits.clear();
                            }
                            state.status = Some(outcome.map_err(|error| error.to_string()));
                            if let Ok(backups) = backups {
                                state.backups = backups;
                            }
                            if let Ok(profile) = profile {
                                state.profile = profile;
                            }
                            state.addon_rows = Some(addons);
                        }
                        Err(error) => state.status = Some(Err(error.to_string())),
                    }
                }
                if installing_addons && app.preferences.client == client {
                    app.refresh_after_change(cx);
                }
                if let Some(message) = feedback {
                    app.show_notification(message, true, cx);
                }
                app.load_last_sync(cx);
                app.refresh_cloud_context(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn dismiss_account_profile_picker(&mut self, cx: &mut Context<Self>) -> bool {
        let was_open = std::mem::take(&mut self.settings_backups.choosing_profile);
        if was_open {
            self.settings_backups.profile_type_ahead.reset();
            cx.notify();
        }
        was_open
    }

    pub(crate) fn account_profile_picker_open(&self) -> bool {
        self.settings_backups.choosing_profile
    }

    fn account_profile_picker_available(&self) -> bool {
        let state = &self.settings_backups;
        state.account.is_some()
            && !state.loading
            && !state.game_running
            && !state.profile.interrupted
            && !self.settings_write_in_progress()
            && !self.account_work_in_progress()
    }

    pub(crate) fn validate_account_profile_picker(&mut self, window: &mut Window) {
        if self.settings_backups.choosing_profile && !self.account_profile_picker_available() {
            self.settings_backups.choosing_profile = false;
            self.settings_backups.profile_type_ahead.reset();
            self.root_focus.focus(window);
        }
    }

    fn toggle_account_profile_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_backups.profile_type_ahead.reset();
        self.settings_backups.choosing_profile = !self.settings_backups.choosing_profile;
        if self.settings_backups.choosing_profile {
            let options = profile_picker_choices(self.cloud_picker_profiles());
            let index = options
                .iter()
                .position(|(profile, _, _)| *profile == self.settings_backups.profile.active)
                .unwrap_or(0);
            if !options.is_empty()
                && let Some(focus) = self.profile_review_focus.picker_options.get(index).cloned()
            {
                window.on_next_frame(move |window, _| focus.focus(window));
            }
        } else {
            self.profile_review_focus.picker.focus(window);
        }
        cx.notify();
    }

    fn profile_picker_type_ahead(
        &mut self,
        key: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(text) = crate::type_ahead::printable(key) else {
            return false;
        };
        if !self.account_profile_picker_available() {
            return false;
        }
        let options = profile_picker_choices(self.cloud_picker_profiles());
        let active = self.settings_backups.profile.active;
        let open = self.settings_backups.choosing_profile;
        let selected = if open {
            self.profile_review_focus
                .picker_options
                .iter()
                .position(|handle| handle.is_focused(window))
        } else {
            options
                .iter()
                .position(|(profile, _, _)| *profile == active)
        }
        .unwrap_or(0);
        let names = options
            .iter()
            .take(self.profile_review_focus.picker_options.len())
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>();
        if let Some(index) = self.settings_backups.profile_type_ahead.find(
            text,
            &names,
            selected,
            std::time::Instant::now(),
        ) {
            if open {
                self.profile_review_focus.picker_options[index].focus(window);
            } else if let Some((profile, _, cloud)) = options.get(index) {
                if *profile != active {
                    if let Some(cloud) = cloud.clone() {
                        self.review_cloud_picker(cloud, window, cx);
                    } else {
                        self.review_local_profile(
                            *profile,
                            *profile == Profile::Handheld,
                            window,
                            cx,
                        );
                    }
                }
            }
        }
        true
    }

    pub(crate) fn account_profile_picker(
        &self,
        palette: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let state = &self.settings_backups;
        let active = state.profile.active;
        let options = profile_picker_choices(self.cloud_picker_profiles());
        let options_len = options
            .len()
            .min(self.profile_review_focus.picker_options.len());
        let mac_popup = cfg!(target_os = "macos");
        let dark_menu = matches!(
            window.appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        let menu_background = match (dark_menu, self.preferences.light_theme) {
            (true, false) => 0x35363a,
            (true, true) => 0x9e9e9d,
            (false, true) => 0xf0f0f0,
            (false, false) => 0xaaaaad,
        };
        let menu_text = if dark_menu { 0xf2f1ed } else { 0x101114 };
        let selected_index = options
            .iter()
            .position(|(profile, _, _)| *profile == active)
            .unwrap_or(0);
        let selected_name = options
            .iter()
            .find(|(profile, _, _)| *profile == active)
            .map(|(_, name, _)| name.clone())
            .unwrap_or_else(|| {
                if self.cloud_picker_profiles().is_some() {
                    "Profile unavailable"
                } else {
                    "Selected profile"
                }
                .into()
            });
        let available = self.account_profile_picker_available();
        let picker_focus = self.profile_review_focus.picker.clone().tab_stop(available);
        // A browser select reserves room for its widest option, not just the
        // currently selected name. Keep custom profile names from resizing it.
        let mut font = window.text_style().font();
        font.weight = gpui::FontWeight(650.);
        let select_width = options
            .iter()
            .map(|(_, name, _)| name)
            .chain(std::iter::once(&selected_name))
            .map(|name| {
                let line = window.text_system().shape_line(
                    name.clone().into(),
                    px(14.),
                    &[gpui::TextRun {
                        len: name.len(),
                        font: font.clone(),
                        color: rgb(palette.text).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                f32::from(line.width) + 40.
            })
            .fold(0_f32, f32::max)
            .ceil();
        let missing_active = !options.iter().any(|(profile, _, _)| *profile == active);
        let selected = palette
            .button("account-active-profile", selected_name.clone())
            .h(px(34.))
            .px(px(9.))
            .w(px(select_width))
            .justify_between()
            .text_size(px(14.))
            .font_weight(gpui::FontWeight(650.))
            .bg(rgb(palette.panel))
            .track_focus(&picker_focus)
            .when(state.choosing_profile, |button| {
                button.shadow(palette.focus_ring())
            })
            .child(crate::icon("chevron", 13., palette.muted))
            .when(!available, |button| button.opacity(0.55).cursor_default())
            .when(available, |button| {
                button
                    .tab_stop(true)
                    .focus(move |style| style.shadow(palette.focus_ring()))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.toggle_account_profile_picker(window, cx)
                    }))
                    .on_key_down(
                        cx.listener(move |app, event: &gpui::KeyDownEvent, window, cx| {
                            match event.keystroke.key.as_str() {
                                "up" if !app.settings_backups.choosing_profile => {
                                    app.toggle_account_profile_picker(window, cx)
                                }
                                "down" if !app.settings_backups.choosing_profile => {
                                    app.toggle_account_profile_picker(window, cx)
                                }
                                "down" if options_len > 0 => {
                                    app.profile_review_focus.picker_options[0].focus(window)
                                }
                                "up" if app.settings_backups.choosing_profile
                                    && options_len > 0 =>
                                {
                                    app.profile_review_focus.picker_options
                                        [options_len.saturating_sub(1)]
                                    .focus(window)
                                }
                                _ => {
                                    if app.profile_picker_type_ahead(&event.keystroke, window, cx) {
                                        cx.stop_propagation();
                                    }
                                    return;
                                }
                            }
                            app.settings_backups.profile_type_ahead.reset();
                            cx.stop_propagation();
                        }),
                    )
            });
        div().relative().flex().flex_col()
            .child(div().flex().items_center().justify_between().gap_3()
                .text_size(px(14.)).font_weight(gpui::FontWeight(650.))
                .child("Profile").child(selected))
            .when(state.choosing_profile && available, |picker| picker.child(
                gpui::deferred(div().absolute()
                    .top(px(if mac_popup { -6. - selected_index as f32 * 24. } else { 34. }))
                    .right(px(if mac_popup { 7. } else { 0. }))
                    .w(px(select_width + if mac_popup { 5. } else { 0. }))
                    .occlude().flex().flex_col().gap(px(if mac_popup { 0. } else { 4. })).p_1()
                    .bg(rgb(if mac_popup { menu_background } else { palette.panel }))
                    .border_1().border_color(palette.line).rounded(px(if mac_popup { 10. } else { 4. }))
                    .shadow(vec![gpui::BoxShadow { color: gpui::rgba(0x00000040).into(), offset: gpui::point(px(0.), px(6.)), blur_radius: px(16.), spread_radius: px(0.) }])
                    .when(missing_active, |menu| menu.child(
                        div().id("unavailable-active-profile")
                            .debug_selector(|| "unavailable-active-profile".into())
                            .flex().items_center().justify_between().gap(px(6.))
                            .px(px(if mac_popup { 5. } else { 15. }))
                            .h(px(if mac_popup { 24. } else { 34. }))
                            .text_size(px(14.)).font_weight(gpui::FontWeight::NORMAL)
                            .text_color(rgb(if mac_popup { menu_text } else { palette.text })).opacity(0.45).cursor_default()
                            .when(mac_popup, |row| row.justify_start().child(
                                div().w(px(14.)).flex_shrink_0().child(crate::icon("check", 12., menu_text))))
                            .child(selected_name)
                            .when(!mac_popup, |row| row.child(crate::icon("check", 14., palette.muted)))
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(|_, _, cx| cx.stop_propagation())
                    ))
                    .children(options.into_iter().take(options_len).enumerate().map(|(index, (profile, name, cloud))| {
                        let focus = self.profile_review_focus.picker_options[index].clone();
                        let next = self.profile_review_focus.picker_options[(index + 1) % options_len].clone();
                        let previous = self.profile_review_focus.picker_options[(index + options_len - 1) % options_len].clone();
                        let first = self.profile_review_focus.picker_options[0].clone();
                        let last = self.profile_review_focus.picker_options[options_len - 1].clone();
                        let apply = move |app: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                            app.dismiss_account_profile_picker(cx);
                            app.profile_review_focus.picker.focus(window);
                            if profile != active {
                                if let Some(cloud) = cloud.clone() { app.review_cloud_picker(cloud, window, cx); }
                                else { app.review_local_profile(profile, profile == Profile::Handheld, window, cx); }
                            }
                        };
                        let hover_focus = focus.clone();
                        let button = if mac_popup {
                            div().id(("account-profile-option", index)).flex().items_center().gap(px(6.)).px(px(5.)).h(px(24.))
                                .rounded(px(5.)).text_size(px(14.)).font_weight(gpui::FontWeight::NORMAL).text_color(rgb(menu_text))
                                .child(div().w(px(14.)).flex_shrink_0().when(profile == active, |mark| mark.child(crate::icon("check", 12., if focus.is_focused(window) { 0xffffff } else { menu_text }))))
                                .child(name)
                        } else {
                            palette.button(("account-profile-option", index), name).h(px(34.)).justify_between()
                                .when(profile == active, |button| button.child(crate::icon("check", 14., palette.text)))
                        };
                        crate::keyboard_button(button.track_focus(&focus), palette,
                            cx.listener(move |app, _: &(), window, cx| apply(app, window, cx)))
                            .when(mac_popup, |button| button.border_0()
                                .focus(|style| style.border_0().bg(rgb(0x0a84ff)).text_color(rgb(0xffffff)))
                                .on_hover(move |hovered, window, _| { if *hovered { hover_focus.focus(window); } }))
                            .on_key_down(cx.listener(move |app, event: &gpui::KeyDownEvent, window, cx| {
                                match event.keystroke.key.as_str() {
                                    "down" => next.focus(window),
                                    "up" => previous.focus(window),
                                    "home" => first.focus(window),
                                    "end" => last.focus(window),
                                    _ => {
                                        if app.profile_picker_type_ahead(&event.keystroke, window, cx) {
                                            cx.stop_propagation();
                                        }
                                        return;
                                    }
                                }
                                app.settings_backups.profile_type_ahead.reset();
                                cx.stop_propagation();
                            }))
                    }))).with_priority(crate::NATIVE_PICKER_LAYER),
            ))
            .when(active == Profile::Handheld && available, |picker| {
                picker.child(div().child(palette.text_button("review-handheld-setup", "Review Handheld setup")
                    .focusable().tab_stop(true).focus(move |style| style.border_color(rgb(palette.accent)))
                    .on_click(cx.listener(|app, _, window, cx| app.review_local_profile(Profile::Handheld, true, window, cx)))))
            })
            .when(state.account.is_none() && !state.loading, |picker| picker.child(
                palette.label("Open WoW once, then close it to set up profiles.").text_size(px(11.)),
            ))
            .when(state.profile.interrupted, |picker| picker.child(self.settings_backup_controls(palette, cx)))
            .when_some(state.status.as_ref(), |picker, status| {
                let (message, color) = match status {
                    Ok(message) => (message, palette.success),
                    Err(message) => (message, palette.error),
                };
                picker.child(palette.label(message.clone()).text_color(rgb(color)))
            })
    }

    pub(crate) fn local_profile_picker(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let state = &self.settings_backups;
        let mut controls = div().flex().flex_col().gap_2();
        if self.page == crate::Page::Account {
            controls = controls.when_some(state.status.as_ref(), |controls, status| {
                let (message, color) = match status {
                    Ok(message) => (message, palette.success),
                    Err(message) => (message, palette.error),
                };
                controls.child(palette.label(message.clone()).text_color(rgb(color)))
            });
        }
        if state.loading {
            return controls.child(palette.label("Loading profiles…"));
        }
        if state.account.is_none() {
            return controls.child(palette.label("Choose a local WoW account in Settings."));
        }
        if state.writing || state.reviewing {
            return controls.child(palette.label("Working on WoW settings…"));
        }
        if state.profile.interrupted {
            return controls.child(palette.label("Recover the interrupted profile in Settings."));
        }
        let active = state.profile.active;
        controls =
            controls.child(
                div().flex().flex_wrap().gap_2().children(
                    state
                        .profile
                        .profiles()
                        .enumerate()
                        .map(|(index, profile)| {
                            let button = palette.button(
                                ("local-profile", index),
                                if profile == active {
                                    format!("{} · Active", state.profile.label(profile))
                                } else {
                                    state.profile.label(profile).to_owned()
                                },
                            );
                            if profile == active {
                                button.border_color(rgb(palette.accent)).cursor_default()
                            } else {
                                crate::keyboard_button(
                                    button,
                                    palette,
                                    cx.listener(move |app, _: &(), window, cx| {
                                        app.review_local_profile(
                                            profile,
                                            profile == Profile::Handheld,
                                            window,
                                            cx,
                                        );
                                    }),
                                )
                            }
                        }),
                ),
            );
        controls = controls.when(self.page == crate::Page::Settings, |controls| {
            controls.child(div().child(crate::keyboard_button(
                palette.button("manage-local-profiles", "Manage profiles"),
                palette,
                cx.listener(|app, _: &(), _, cx| {
                    app.settings_backups.manage_profiles = true;
                    app.settings_backups.status = None;
                    cx.notify();
                }),
            )))
        });
        if !state.profile.handheld_preset || active == Profile::Handheld {
            controls = controls.child(div().child(crate::keyboard_button(
                palette.button("configure-handheld", "Apply Handheld preset…"),
                palette,
                cx.listener(|app, _: &(), window, cx| {
                    app.review_local_profile(Profile::Handheld, true, window, cx);
                }),
            )));
        }
        controls
    }

    pub fn settings_backup_controls(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let state = &self.settings_backups;
        let mut controls = div().flex().flex_col().gap_2();
        if let Some(status) = &state.status {
            let (message, color) = match status {
                Ok(message) => (message, palette.success),
                Err(message) => (message, palette.error),
            };
            controls = controls.child(palette.label(message.clone()).text_color(rgb(color)));
        }
        if self.preferences_path.is_none() {
            return controls.child(palette.label("Astral storage is unavailable."));
        }
        if self.root(self.preferences.client).is_none() {
            return controls.child(palette.label("Choose a WoW folder first."));
        }
        if state.loading {
            return controls.child(palette.label("Loading settings backups…"));
        }
        if state.accounts.is_empty() {
            return if state.status.is_some() {
                controls
            } else {
                controls.child(palette.label("No local WoW accounts found."))
            };
        }
        let busy = state.writing;
        if state.account.is_none() {
            return controls.child(palette.label("Choose a WoW account."));
        }
        if busy {
            return controls.child(palette.label("Working on WoW settings…"));
        }
        if state.profile.interrupted {
            return controls
                .child(palette.label(
                    "A profile switch was interrupted. Close WoW and recover your previous setup.",
                ))
                .child(
                    div().child(
                        palette
                            .button("recover-profile", "Recover previous setup")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.write_settings_backup(Action::Recover, cx)
                            })),
                    ),
                );
        }
        if state.manage_profiles {
            return controls.child(self.profile_manager(palette, cx));
        }
        if state.reviewing {
            return controls.child(palette.label("Reviewing profile…"));
        }
        if let Some(backup) = &state.confirmation {
            let id = backup.id.clone();
            return controls
                .child(palette.label(format!(
                    "Restore the backup from {}? Astral saves a copy of the files it replaces.",
                    backup_date(backup.time)
                )))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            palette
                                .button("cancel-settings-restore", "Cancel")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.settings_backups.confirmation = None;
                                    app.settings_backups.status = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            palette
                                .button("confirm-settings-restore", "Restore settings")
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.write_settings_backup(Action::Restore(id.clone()), cx);
                                })),
                        ),
                );
        }
        controls = controls.child(self.local_profile_picker(palette, cx));
        controls = controls.child(self.profile_addon_controls(palette, cx));
        let backup = palette.button("back-up-settings", "Back up settings");
        controls = controls.child(div().child(if state.game_running {
            backup.opacity(0.45).cursor_default()
        } else {
            crate::keyboard_button(
                backup,
                palette,
                cx.listener(|app, _: &(), _, cx| app.write_settings_backup(Action::Backup, cx)),
            )
        }));
        if state.backups.is_empty() {
            controls = controls.child(palette.label("No settings backups yet."));
        }
        for (index, backup) in state.backups.iter().enumerate() {
            let backup = backup.clone();
            controls = controls.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(palette.label(backup_date(backup.time)))
                    .child({
                        let restore = palette.button(("restore-settings", index), "Restore…");
                        if state.game_running {
                            restore.opacity(0.45).cursor_default()
                        } else {
                            crate::keyboard_button(
                                restore,
                                palette,
                                cx.listener(move |app, _: &(), _, cx| {
                                    app.settings_backups.confirmation = Some(backup.clone());
                                    app.settings_backups.status = None;
                                    cx.notify();
                                }),
                            )
                        }
                    }),
            );
        }
        controls
    }

    fn edit_profile_name(
        &mut self,
        profile: Option<Profile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = &mut self.settings_backups;
        let name = profile
            .map(|id| state.profile.label(id).to_owned())
            .unwrap_or_default();
        state.profile_edit = Some(ProfileEdit::Name(profile, name));
        state.status = None;
        self.profile_name_focus.focus(window);
        cx.notify();
    }

    fn profile_manager(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let state = &self.settings_backups;
        let rows = div().flex().flex_col().gap_2();
        let cancel = || {
            palette
                .button("cancel-profile-edit", "Cancel")
                .on_click(cx.listener(|app, _, window, cx| {
                    app.settings_backups.profile_edit = None;
                    app.settings_backups.status = None;
                    app.root_focus.focus(window);
                    cx.notify();
                }))
        };
        match &state.profile_edit {
            Some(ProfileEdit::Name(profile, name)) => {
                let profile = *profile;
                let submitted_name = name.clone();
                return rows
                    .child(palette.label(if profile.is_some() {
                        "Rename profile"
                    } else {
                        "New profile"
                    }))
                    .child(
                        div()
                            .id("profile-name-input")
                            .track_focus(&self.profile_name_focus)
                            .border_1()
                            .border_color(rgb(palette.accent))
                            .rounded_md()
                            .min_h(px(36.))
                            .px_3()
                            .py_2()
                            .child(if name.is_empty() {
                                palette.label("Profile name")
                            } else {
                                palette.label(name.clone()).text_color(rgb(palette.text))
                            })
                            .on_click(
                                cx.listener(|app, _, window, _| {
                                    app.profile_name_focus.focus(window)
                                }),
                            )
                            .on_key_down(cx.listener(
                                |app, event: &gpui::KeyDownEvent, window, cx| {
                                    let Some(ProfileEdit::Name(profile, name)) =
                                        &mut app.settings_backups.profile_edit
                                    else {
                                        return;
                                    };
                                    let key = &event.keystroke;
                                    let command = key.modifiers.platform || key.modifiers.control;
                                    match key.key.as_str() {
                                        "escape" => {
                                            app.settings_backups.profile_edit = None;
                                            app.settings_backups.status = None;
                                            app.root_focus.focus(window);
                                        }
                                        "enter" => {
                                            let action = Action::SaveName(*profile, name.clone());
                                            app.write_settings_backup(action, cx);
                                        }
                                        "backspace" if command => name.clear(),
                                        "backspace" => {
                                            name.pop();
                                        }
                                        _ => {
                                            let input = if command && key.key == "v" {
                                                cx.read_from_clipboard()
                                                    .and_then(|item| item.text())
                                            } else if !command {
                                                key.key_char.clone()
                                            } else {
                                                None
                                            };
                                            let Some(input) = input else { return };
                                            for ch in input.chars().filter(|ch| !ch.is_control()) {
                                                if name.encode_utf16().count() + ch.len_utf16() > 40
                                                {
                                                    break;
                                                }
                                                name.push(ch);
                                            }
                                        }
                                    }
                                    cx.stop_propagation();
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div().flex().gap_2().child(cancel()).child(
                            palette
                                .button("save-profile-name", "Save profile")
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.write_settings_backup(
                                        Action::SaveName(profile, submitted_name.clone()),
                                        cx,
                                    );
                                })),
                        ),
                    );
            }
            Some(ProfileEdit::Delete(profile)) => {
                let profile = *profile;
                return rows
                    .child(palette.label(format!(
                        "Delete {}? Its saved setup will move to backups.",
                        state.profile.label(profile)
                    )))
                    .child(
                        div().flex().gap_2().child(cancel()).child(
                            palette
                                .button("confirm-delete-profile", "Delete profile")
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.write_settings_backup(Action::DeleteProfile(profile), cx);
                                })),
                        ),
                    );
            }
            None => {}
        }
        rows.children(
            state
                .profile
                .profiles()
                .enumerate()
                .map(|(index, profile)| {
                    let name = state.profile.label(profile);
                    let active = state.profile.active == profile;
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(palette.label(if active {
                            format!("{name} · Active")
                        } else {
                            name.to_owned()
                        }))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    palette
                                        .button(("rename-profile", index), "Rename")
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.edit_profile_name(Some(profile), window, cx);
                                        })),
                                )
                                .when(!active && matches!(profile, Profile::Custom(_)), |row| {
                                    row.child(
                                        palette
                                            .button(("delete-profile", index), "Delete…")
                                            .on_click(cx.listener(move |app, _, _, cx| {
                                                app.settings_backups.profile_edit =
                                                    Some(ProfileEdit::Delete(profile));
                                                app.settings_backups.status = None;
                                                cx.notify();
                                            })),
                                    )
                                }),
                        )
                }),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    palette
                        .button("close-profile-manager", "Done")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.settings_backups.manage_profiles = false;
                            app.settings_backups.status = None;
                            cx.notify();
                        })),
                )
                .when(state.profile.profiles().count() < 8, |row| {
                    row.child(
                        palette
                            .button("new-custom-profile", "New profile")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.edit_profile_name(None, window, cx);
                            })),
                    )
                }),
        )
    }
}

fn backup_date(millis: u64) -> String {
    let seconds = (millis / 1000) as i64;
    if let Ok(date) = time::OffsetDateTime::from_unix_timestamp(seconds) {
        format!(
            "{} {:02}:{:02}:{:02} UTC",
            date.date(),
            date.hour(),
            date.minute(),
            date.second()
        )
    } else {
        "Unknown date".into()
    }
}

/// Cloud profiles may not exist in this computer's local store yet. Keep them
/// reachable in the main picker; management only creates/renames/deletes them.
fn profile_picker_choices(
    cloud: Option<&[crate::cloud::CloudProfile]>,
) -> Vec<(Profile, String, Option<crate::cloud::CloudProfile>)> {
    if let Some(rows) = cloud {
        rows.iter()
            .filter_map(|row| {
                row.profile()
                    .ok()
                    .map(|id| (id, row.name.clone(), Some(row.clone())))
            })
            .collect()
    } else {
        Profile::ALL
            .into_iter()
            .map(|id| (id, id.label().to_owned(), None))
            .collect()
    }
}

#[cfg(test)]
mod picker_tests {
    use super::*;
    #[test]
    fn uncached_options_are_defaults_but_an_empty_cached_list_stays_empty() {
        let defaults = profile_picker_choices(None);
        assert_eq!(
            defaults
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["Regular", "Handheld"]
        );
        assert!(defaults.iter().all(|(_, _, cloud)| cloud.is_none()));
        assert!(profile_picker_choices(Some(&[])).is_empty());
    }

    #[test]
    fn a_cloud_only_profile_is_reachable_without_importing_it_first() {
        let local = ProfileState::default();
        let id = "0123456789abcdef0123456789abcdef";
        let cloud = [crate::cloud::CloudProfile {
            id: id.into(),
            name: "Raid setup".into(),
        }];
        let custom = Profile::from_cloud_id(id).unwrap();
        assert!(!local.profiles().any(|profile| profile == custom));
        let choices = profile_picker_choices(Some(&cloud));
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].0, custom);
        assert_eq!(choices[0].1, "Raid setup");
        assert_eq!(choices[0].2.as_ref().unwrap().id, id);
        assert!(
            profile_picker_choices(None)
                .iter()
                .any(|(id, _, _)| *id == Profile::Handheld)
        );
    }
}
