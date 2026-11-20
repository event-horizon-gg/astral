use super::{HistoryScope, State as AccountState, load_history_scope, unauthorized};
use crate::{
    Astral, Palette,
    cloud::{CloudProfile, Service},
    credentials,
    settings_backup::{ProfileReview, Store},
};
use gpui::{Context, Entity, FocusHandle, Window, div, prelude::*, px, rgb};
use std::{
    io,
    path::{Path, PathBuf},
};

#[cfg(all(test, feature = "headless-ui-tests"))]
mod ui_tests {
    use super::*;
    use crate::{Client, Page, ToolbarMenu, ui_tests::press};
    use gpui::{KeyBinding, Modifiers, TestAppContext, size};

    fn fixture(directory: &Path, cx: &mut Context<Astral>) -> Astral {
        cx.bind_keys([KeyBinding::new("escape", crate::DismissOverlay, None)]);
        let mut app = Astral::new(
            [None, None],
            Some(directory),
            Some(Client::Retail),
            false,
            cx,
        );
        app.page = Page::Account;
        app.seed_cloud_profile_fixture();
        app.account.profiles.manage = true;
        app
    }

    #[gpui::test]
    fn manager_keeps_header_focus_and_captures_tab_and_escape(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        let appearance = cx.debug_bounds("appearance-trigger").unwrap().center();
        cx.simulate_click(appearance, Modifiers::default());
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(app.header_focus.contains_focused(window, cx))
        });
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.account.profiles.focus.as_ref().unwrap().controls[0].is_focused(window))
        });
        press(cx, "shift-tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.account.profiles.focus.as_ref().unwrap().controls[17].is_focused(window))
        });
        let tools = cx.debug_bounds("tools-trigger").unwrap().center();
        cx.simulate_click(tools, Modifiers::default());
        press(cx, "escape");
        assert!(
            view.read_with(cx, |app, _| !app.cloud_profile_manager_open()
                && app.menu == Some(ToolbarMenu::Tools))
        );
    }

    #[gpui::test]
    fn busy_name_editor_keeps_only_the_input_in_its_tab_loop(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            app.start_cloud_name(None, window, cx);
            app.account.busy = Some("Fixture operation");
        });
        cx.run_until_parked();
        let appearance = cx.debug_bounds("appearance-trigger").unwrap().center();
        cx.simulate_click(appearance, Modifiers::default());
        press(cx, "tab");
        view.update_in(cx, |app, window, cx| {
            assert!(
                app.account
                    .profiles
                    .name_input
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .focus
                    .is_focused(window)
            )
        });
        cx.simulate_input("Raid");
        press(cx, "shift-tab");
        view.update_in(cx, |app, window, cx| {
            assert!(
                app.account
                    .profiles
                    .name_input
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .focus
                    .is_focused(window)
            )
        });
        press(cx, "escape");
        assert!(view.read_with(cx, |app, _| app.cloud_profile_manager_open()));
        // No profile is saved or deleted; all edits remain in the fixture model.
    }

    #[gpui::test]
    fn leaving_manager_keeps_the_clicked_navigation_focused(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        let brand = cx.debug_bounds("astral-brand").unwrap().center();
        cx.simulate_click(brand, Modifiers::default());
        cx.run_until_parked();
        assert!(view.read_with(cx, |app, _| app.page == Page::Library
            && !app.cloud_profile_manager_open()));
        view.update_in(cx, |app, window, _| {
            assert!(app.brand_focus.is_focused(window))
        });
    }
    #[gpui::test]
    fn switching_manager_forms_does_not_repeat_initial_focus(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        let create = cx.debug_bounds("new-cloud-profile").unwrap().center();
        cx.simulate_click(create, Modifiers::default());
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(app.root_focus.is_focused(window));
            assert!(
                !app.account
                    .profiles
                    .name_input
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .focus
                    .is_focused(window)
            );
        });
        press(cx, "tab");
        view.update_in(cx, |app, window, cx| {
            assert!(
                app.account
                    .profiles
                    .name_input
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .focus
                    .is_focused(window)
            )
        });
        let cancel = cx
            .debug_bounds("cancel-cloud-profile-edit")
            .unwrap()
            .center();
        cx.simulate_click(cancel, Modifiers::default());
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            assert!(app.root_focus.is_focused(window))
        });
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.account.profiles.focus.as_ref().unwrap().controls[0].is_focused(window))
        });
    }

    type ListResult = std::result::Result<Vec<CloudProfile>, ListError>;
    fn deferred_list() -> (
        impl std::future::Future<Output = ListResult>,
        impl FnOnce(ListResult),
    ) {
        use std::{
            cell::RefCell,
            rc::Rc,
            task::{Poll, Waker},
        };
        let response = Rc::new(RefCell::new(None));
        let wake = Rc::new(RefCell::new(None::<Waker>));
        let future = std::future::poll_fn({
            let response = response.clone();
            let wake = wake.clone();
            move |cx| {
                *wake.borrow_mut() = Some(cx.waker().clone());
                response
                    .borrow_mut()
                    .take()
                    .map_or(Poll::Pending, Poll::Ready)
            }
        });
        (future, move |result| {
            *response.borrow_mut() = Some(result);
            if let Some(wake) = wake.borrow_mut().take() {
                wake.wake();
            }
        })
    }
    fn session(token: char) -> crate::cloud::Session {
        serde_json::from_value(
            serde_json::json!({"token": token.to_string().repeat(43), "name": "Fixture#1234"}),
        )
        .unwrap()
    }
    fn rows(name: &str) -> Vec<CloudProfile> {
        vec![CloudProfile {
            id: "regular".into(),
            name: name.into(),
        }]
    }

    #[gpui::test]
    fn cached_names_follow_root_and_account_without_closing_the_name_editor(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let first_root = directory.path().join("first/_retail_");
        let second_root = directory.path().join("second/_retail_");
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = fixture(directory.path(), cx);
            for (root, account, name) in [
                (&first_root, "One", "First"),
                (&first_root, "Two", "Second account"),
                (&second_root, "One", "Second folder"),
            ] {
                app.preferences.remember_profile_names(
                    crate::preferences::ProfileNamesScope {
                        root: Some(root.clone()),
                        account: Some(account.into()),
                    },
                    &rows(name),
                );
            }
            app.preferences.folders[0] = Some(first_root.clone());
            app.preferences.local_accounts[0] = Some((first_root.clone(), "One".into()));
            app
        });
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(app.account.session.is_none());
            assert_eq!(app.cloud_picker_profiles().unwrap()[0].name, "First");
            app.start_cloud_name(None, window, cx);
        });
        cx.run_until_parked();
        for (root, account, expected) in [
            (&first_root, "Two", "Second account"),
            (&second_root, "One", "Second folder"),
        ] {
            view.update(cx, |app, cx| {
                app.preferences.folders[0] = Some(root.clone());
                app.preferences.local_accounts[0] = Some((root.clone(), account.into()));
                // Even before the next render, the picker cannot leak old rows.
                assert_eq!(app.cloud_picker_profiles().unwrap()[0].name, expected);
                cx.notify();
            });
            cx.run_until_parked();
            view.read_with(cx, |app, _| {
                assert!(matches!(app.account.profiles.edit, Some(Edit::Name { .. })));
                assert!(app.cloud_profile_manager_open());
                assert_eq!(app.account.profiles.rows[0].name, expected);
            });
        }
        view.update(cx, |app, _| {
            app.remember_cloud_profiles(Client::Retail, rows("Fresh name"))
        });
        let preferences =
            crate::preferences::Preferences::load(&directory.path().join("preferences.json"))
                .unwrap();
        let names = preferences
            .cached_profile_names(&crate::preferences::ProfileNamesScope {
                root: Some(second_root),
                account: Some("One".into()),
            })
            .unwrap();
        assert_eq!(names[0].name, "Fresh name");
    }

    #[gpui::test]
    fn an_old_account_reply_cannot_repopulate_an_uncached_account(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("_retail_");
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = fixture(directory.path(), cx);
            app.preferences.folders[0] = Some(root.clone());
            app.preferences.local_accounts[0] = Some((root.clone(), "One".into()));
            app.account.session = Some(session('f'));
            app.remember_cloud_profiles(Client::Retail, rows("Account one"));
            app
        });
        let (future, finish) = deferred_list();
        view.update(cx, |app, cx| app.await_profile_list(future, cx));
        cx.run_until_parked();
        view.update(cx, |app, cx| {
            app.preferences.local_accounts[0] = Some((root.clone(), "Two".into()));
            assert!(app.cloud_picker_profiles().is_none());
            cx.notify();
        });
        cx.run_until_parked();
        finish(Ok(rows("Late account one")));
        cx.run_until_parked();
        view.read_with(cx, |app, _| {
            assert!(app.cloud_picker_profiles().is_none());
            assert!(app.account.profiles.rows.is_empty());
            assert!(app.account.profiles.client.is_none());
            assert_eq!(
                app.preferences
                    .cached_profile_names(&crate::preferences::ProfileNamesScope {
                        root: Some(root),
                        account: Some("One".into()),
                    })
                    .unwrap()[0]
                    .name,
                "Account one"
            );
        });
    }

    #[gpui::test]
    fn profile_reads_do_not_block_the_manager_and_the_latest_reply_wins(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        let (old, finish_old) = deferred_list();
        let (new, finish_new) = deferred_list();
        view.update(cx, |app, cx| {
            app.account.session = Some(session('f'));
            app.await_profile_list(old, cx);
            app.await_profile_list(new, cx);
            assert!(app.account.busy.is_none());
        });
        cx.run_until_parked();
        let appearance = cx.debug_bounds("appearance-trigger").unwrap().center();
        cx.simulate_click(appearance, Modifiers::default());
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.account.profiles.focus.as_ref().unwrap().controls[0].is_focused(window))
        });
        finish_new(Ok(rows("Newest")));
        cx.run_until_parked();
        finish_old(Err("Obsolete failure".into()));
        cx.run_until_parked();
        view.read_with(cx, |app, _| {
            assert_eq!(app.account.profiles.rows[0].name, "Newest");
            assert!(app.account.profiles.error.is_none());
            assert!(app.account.busy.is_none());
        });
    }

    #[gpui::test]
    fn profile_read_replies_cannot_cross_account_root_client_session_or_unmount(
        cx: &mut TestAppContext,
    ) {
        for changed in ["account", "root", "client", "session", "unmount"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("_retail_");
            std::fs::create_dir_all(root.join("Interface/AddOns")).unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
            let (future, finish) = deferred_list();
            view.update(cx, |app, cx| {
                app.account.session = Some(session('f'));
                app.detected_roots[Client::Retail.index()] = Some(root.clone());
                app.preferences.local_accounts[Client::Retail.index()] =
                    Some((root.clone(), "A".into()));
                app.await_profile_list(future, cx);
            });
            cx.run_until_parked();
            view.update_in(cx, |app, window, cx| match changed {
                "account" => {
                    app.preferences.local_accounts[Client::Retail.index()] =
                        Some((root.clone(), "B".into()))
                }
                "root" => app.detected_roots[Client::Retail.index()] = None,
                "client" => app.preferences.client = Client::Forever,
                "session" => app.account.session = Some(session('e')),
                "unmount" => {
                    app.navigate(Page::Library, window, cx);
                    app.navigate(Page::Account, window, cx);
                }
                _ => unreachable!(),
            });
            finish(Ok(rows("Stale")));
            cx.run_until_parked();
            assert!(
                view.read_with(cx, |app, _| !app
                    .account
                    .profiles
                    .rows
                    .iter()
                    .any(|row| row.name == "Stale")),
                "{changed}"
            );
        }
    }

    #[gpui::test]
    fn retry_remains_available_during_other_work_and_reads_do_not_release_that_work(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        cx.simulate_resize(size(px(1250.), px(700.)));
        view.update(cx, |app, cx| {
            app.await_profile_list(std::future::ready(Err("Fixture read failed".into())), cx)
        });
        cx.run_until_parked();
        view.update(cx, |app, cx| {
            app.account.busy = Some("Fixture write");
            cx.notify();
        });
        cx.run_until_parked();
        let appearance = cx.debug_bounds("appearance-trigger").unwrap().center();
        cx.simulate_click(appearance, Modifiers::default());
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.account.profiles.focus.as_ref().unwrap().controls[17].is_focused(window))
        });
        let (future, finish) = deferred_list();
        view.update(cx, |app, cx| app.await_profile_list(future, cx));
        cx.run_until_parked();
        finish(Ok(rows("Read result")));
        cx.run_until_parked();
        view.read_with(cx, |app, _| {
            assert_eq!(app.account.busy, Some("Fixture write"));
            assert!(app.account.error.is_none());
            assert!(app.account.profiles.error.is_none());
            assert_eq!(app.account.profiles.rows[0].name, "Read result");
        });
    }

    #[gpui::test]
    fn uncached_profile_loading_keeps_done_enabled_and_finishes_after_manager_closes(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (future, finish) = deferred_list();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = fixture(directory.path(), cx);
            app.account.profiles.client = None;
            app.account.profiles.rows.clear();
            app.await_profile_list(future, cx);
            app
        });
        cx.simulate_resize(size(px(1250.), px(700.)));
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            assert!(app.account.busy.is_none());
            assert!(app.account.profiles.focus.as_ref().unwrap().controls[16].is_focused(window));
        });
        press(cx, "enter");
        assert!(view.read_with(cx, |app, _| !app.cloud_profile_manager_open()));
        finish(Ok(rows("Loaded after close")));
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |app, _| !app.cloud_profile_manager_open()
                && app.account.profiles.rows[0].name == "Loaded after close")
        );
    }
    #[gpui::test]
    fn expired_list_read_clears_the_fixture_session_without_releasing_a_write(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), cx));
        view.update(cx, |app, cx| {
            app.account.session = Some(session('f'));
            app.account.busy = Some("Fixture write");
            app.await_profile_list(
                std::future::ready(Err(ListError {
                    message: "Expired fixture session".into(),
                    expired: true,
                })),
                cx,
            );
        });
        cx.run_until_parked();
        view.read_with(cx, |app, _| {
            assert!(app.fixture);
            assert!(app.account.session.is_none());
            assert_eq!(app.account.busy, Some("Fixture write"));
            assert_eq!(
                app.account.profiles.error.as_deref(),
                Some("Expired fixture session")
            );
        });
    }
}

#[derive(Default)]
pub(super) struct State {
    list_request: Option<uuid::Uuid>,
    error: Option<String>,
    client: Option<crate::Client>,
    names_context: Option<(crate::Client, crate::preferences::ProfileNamesScope)>,
    rows: Vec<CloudProfile>,
    manage: bool,
    edit: Option<Edit>,
    message: Option<String>,
    review: Option<(CloudProfile, ProfileReview)>,
    show_review_files: bool,
    name_input: Option<Entity<crate::search_input::SearchInput>>,
    focus: Option<ManagerFocus>,
}

impl State {
    pub(super) fn editing(&self) -> bool {
        self.manage || self.edit.is_some() || self.review.is_some()
    }
}

struct ManagerFocus {
    previous: Option<FocusHandle>,
    controls: Vec<FocusHandle>,
    input: Option<FocusHandle>,
    mode: u8,
}

#[derive(Clone)]
enum Edit {
    Name {
        profile: Option<CloudProfile>,
        value: String,
    },
    Delete(CloudProfile),
}

enum Work {
    Save(Option<CloudProfile>, String),
    Delete(CloudProfile),
    Review(CloudProfile),
    Apply(CloudProfile, ProfileReview),
}

struct ListError {
    message: String,
    expired: bool,
}

impl From<String> for ListError {
    fn from(message: String) -> Self {
        Self {
            message,
            expired: false,
        }
    }
}

impl From<&str> for ListError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

impl Astral {
    fn profile_names_scope(&self) -> crate::preferences::ProfileNamesScope {
        let client = self.preferences.client;
        let root = self.root(client).cloned();
        let account = root
            .as_deref()
            .and_then(|root| self.selected_local_account(client, root));
        crate::preferences::ProfileNamesScope { root, account }
    }

    pub(crate) fn sync_cloud_profile_names(&mut self) {
        let client = self.preferences.client;
        let scope = self.profile_names_scope();
        let state = &mut self.account.profiles;
        if state.names_context.as_ref() == Some(&(client, scope.clone())) {
            return;
        }
        let cached = self.preferences.cached_profile_names(&scope);
        state.rows = cached.unwrap_or_default().to_vec();
        state.client = cached.map(|_| client);
        state.names_context = Some((client, scope));
        state.error = None;
        state.list_request = None;
        // Root/account changes reload names but preserve the open editor, as
        // SyncProfiles' effect does. Game-client changes remount the component.
    }

    pub(super) fn begin_profile_list_read(&mut self) -> uuid::Uuid {
        self.sync_cloud_profile_names();
        let state = &mut self.account.profiles;
        let request = uuid::Uuid::new_v4();
        state.list_request = Some(request);
        state.error = None;
        request
    }

    pub(super) fn profile_list_reply_current(&self, request: uuid::Uuid) -> bool {
        self.account.profiles.list_request == Some(request)
    }

    fn invalidate_profile_list_read(&mut self) {
        let state = &mut self.account.profiles;
        state.list_request = None;
    }

    fn profile_list_context(&self) -> Option<super::AccountContext> {
        self.account_context().map(|mut context| {
            context.profile = None;
            context
        })
    }

    fn refresh_cloud_profile_list(&mut self, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        let Some(session) = self.account.session.clone() else {
            return;
        };
        let client = self.preferences.client;
        let task = cx.background_executor().spawn(async move {
            Service::new()
                .profiles(&session, client)
                .map_err(|error| ListError {
                    expired: error.status == Some(401),
                    message: error.to_string(),
                })
        });
        self.await_profile_list(task, cx);
    }

    fn await_profile_list(
        &mut self,
        task: impl std::future::Future<Output = std::result::Result<Vec<CloudProfile>, ListError>>
        + 'static,
        cx: &mut Context<Self>,
    ) {
        let request = self.begin_profile_list_read();
        let context = self.profile_list_context();
        let client = self.preferences.client;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if !app.profile_list_reply_current(request)
                    || app.page != crate::Page::Account
                    || app.preferences.client != client
                    || app.profile_list_context() != context
                {
                    return;
                }
                match result {
                    Ok(rows) => app.remember_cloud_profiles(client, rows),
                    Err(error) => {
                        app.remember_cloud_profile_error(error.message.clone());
                        if error.expired {
                            app.expire_account_read(error.message, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn remember_cloud_profiles(
        &mut self,
        client: crate::Client,
        rows: Vec<CloudProfile>,
    ) {
        if self.preferences.client == client {
            let scope = self.profile_names_scope();
            self.preferences
                .remember_profile_names(scope.clone(), &rows);
            self.save_preferences();
            self.account.profiles.names_context = Some((client, scope));
            self.account.profiles.client = Some(client);
            self.account.profiles.rows = rows;
            self.account.profiles.error = None;
        }
    }

    pub(super) fn remember_cloud_profile_error(&mut self, error: String) {
        self.account.profiles.error = Some(error);
    }

    pub(crate) fn cloud_picker_profiles(&self) -> Option<&[CloudProfile]> {
        let scope = self.profile_names_scope();
        let state = &self.account.profiles;
        if state.client == Some(self.preferences.client)
            && state.names_context.as_ref() == Some(&(self.preferences.client, scope.clone()))
        {
            Some(state.rows.as_slice())
        } else {
            self.preferences.cached_profile_names(&scope)
        }
    }

    pub(crate) fn review_cloud_picker(
        &mut self,
        profile: CloudProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.account.session.is_none() {
            if let Ok(id) = profile.profile() {
                self.review_cached_profile(id, window, cx);
            }
            return;
        }
        self.profile_review_focus.return_to = window.focused(cx);
        self.cloud_profiles_work(Work::Review(profile), cx);
    }

    pub(super) fn seed_cloud_profile_fixture(&mut self) {
        self.account.profiles.names_context =
            Some((self.preferences.client, self.profile_names_scope()));
        self.account.profiles.client = Some(self.preferences.client);
        self.account.profiles.rows = [
            ("regular", "Regular"),
            ("handheld", "Handheld"),
            ("0123456789abcdef0123456789abcdef", "Raid setup"),
        ]
        .into_iter()
        .map(|(id, name)| CloudProfile {
            id: id.into(),
            name: name.into(),
        })
        .collect();
    }

    pub(crate) fn cloud_profile_review_open(&self) -> bool {
        let state = &self.account.profiles;
        self.page == crate::Page::Account
            && state.client == Some(self.preferences.client)
            && state.review.is_some()
    }

    pub(crate) fn current_cloud_profile_review(&self) -> Option<&ProfileReview> {
        if !self.cloud_profile_review_open() {
            return None;
        }
        self.account
            .profiles
            .review
            .as_ref()
            .map(|(_, review)| review)
    }

    pub(crate) fn cloud_profile_review_dialog(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<gpui::Div> {
        let state = &self.account.profiles;
        if self.page != crate::Page::Account || state.client != Some(self.preferences.client) {
            return None;
        }
        let (_, review) = state.review.as_ref()?;
        Some(self.profile_review_dialog(review, state.show_review_files, true, palette, cx))
    }

    pub(crate) fn toggle_cloud_profile_review_files(&mut self) {
        let state = &mut self.account.profiles;
        state.show_review_files = !state.show_review_files;
    }

    pub(crate) fn dismiss_cloud_profile_review(&mut self, cx: &mut Context<Self>) {
        self.account.profiles.review = None;
        cx.notify();
    }

    pub(crate) fn apply_cloud_profile_review(&mut self, cx: &mut Context<Self>) {
        if let Some((profile, review)) = self.account.profiles.review.clone() {
            self.cloud_profiles_work(Work::Apply(profile, review), cx);
        }
    }

    fn cloud_profiles_work(&mut self, work: Work, cx: &mut Context<Self>) {
        if self.fixture {
            // Signed-in comparison data is read-only and never reaches the service.
            return;
        }
        if self.account.busy.is_some() {
            return;
        }
        let Some(session) = self.account.session.clone() else {
            return;
        };
        let client = self.preferences.client;
        let applying_review = matches!(&work, Work::Apply(_, _));
        let installing_addons =
            matches!(&work, Work::Apply(_, review) if !review.packages.is_empty());
        if self.account.profiles.client != Some(client) {
            self.account.profiles.edit = None;
            self.account.profiles.review = None;
            self.account.error = Some("The game client changed. Refresh cloud profiles.".into());
            cx.notify();
            return;
        }
        let root = self.root(client).cloned();
        let data = self
            .preferences_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let selected = root
            .as_deref()
            .and_then(|root| self.selected_local_account(client, root));
        self.account.error = None;
        self.invalidate_profile_list_read();
        self.account.profiles.message = None;
        self.begin_account_work("Updating cloud profiles…");
        let catalog = self.catalog.clone();
        let task = cx.background_executor().spawn(async move {
            let service = Service::new();
            let local = |root: &Option<PathBuf>,
                         data: &Option<PathBuf>,
                         selected: &Option<String>|
             -> Result<HistoryScope> {
                load_history_scope(client, root.clone(), data.clone(), selected.clone())?
                    .ok_or_else(|| {
                        io::Error::other("Choose a local WoW account in Settings first.").into()
                    })
            };
            let result = (|| -> Result<_> {
                let (work, reviewed) = match work {
                    Work::Apply(profile, review) => (Work::Review(profile), Some(review)),
                    work => (work, None),
                };
                let (message, switched) = match work {
                    Work::Save(profile, name) => {
                        let id = profile.as_ref().map(CloudProfile::profile).transpose()?;
                        let saved = service.save_profile(&session, client, id, &name)?;
                        if let (Some(id), Some(data), Ok(scope)) =
                            (id, data.as_deref(), local(&root, &data, &selected))
                        {
                            let store = Store::open(data, &scope.root, client, &scope.account)?;
                            if store
                                .profile_state()?
                                .profiles()
                                .any(|profile| profile == id)
                            {
                                store.import_cloud_profile(id, &saved.name)?;
                            }
                        }
                        (
                            Some(if id.is_some() {
                                "Cloud profile renamed.".into()
                            } else {
                                "Cloud profile created.".into()
                            }),
                            false,
                        )
                    }
                    Work::Delete(profile) => {
                        let id = profile.profile()?;
                        let scope = load_history_scope(
                            client,
                            root.clone(),
                            data.clone(),
                            selected.clone(),
                        )?;
                        let store = scope
                            .as_ref()
                            .map(|scope| {
                                Store::open(
                                    data.as_deref().expect("local scope has a data directory"),
                                    &scope.root,
                                    client,
                                    &scope.account,
                                )
                            })
                            .transpose()?;
                        let _lock = store.as_ref().map(Store::lock).transpose()?;
                        if store
                            .as_ref()
                            .map(Store::profile_state)
                            .transpose()?
                            .is_some_and(|state| state.active == id)
                        {
                            return Err(io::Error::other(
                                "Switch to another profile before deleting this one.",
                            )
                            .into());
                        }
                        service.delete_profile(&session, client, id)?;
                        (
                            Some(
                                "Cloud profile deleted. Local backups are still available.".into(),
                            ),
                            false,
                        )
                    }
                    Work::Review(profile) | Work::Apply(profile, _) => {
                        let id = profile.profile()?;
                        let scope = local(&root, &data, &selected)?;
                        let store = Store::open(
                            data.as_deref().unwrap(),
                            &scope.root,
                            client,
                            &scope.account,
                        )?;
                        let rows = service.profiles(&session, client)?;
                        let current = rows
                            .iter()
                            .find(|profile| profile.id == id.cloud_id())
                            .ok_or_else(|| {
                                io::Error::other(
                                    "This cloud profile was deleted. Refresh the list.",
                                )
                            })?;
                        if let Some(review) = reviewed {
                            if current.name != review.name {
                                return Err(io::Error::other(
                                    "The cloud profile changed. Review it again.",
                                )
                                .into());
                            }
                            store.apply_review(&review, &catalog)?;
                            return Ok((
                                Ok(rows),
                                Some(format!(
                                    "{} is active. Choose Sync now to get its cloud settings.",
                                    review.name
                                )),
                                true,
                                None,
                            ));
                        }
                        let review = store.review_cloud_profile(id, &current.name, &catalog)?;
                        let profile = current.clone();
                        return Ok((Ok(rows), None, false, Some((profile, review))));
                    }
                };
                let rows = service.profiles(&session, client);
                Ok((rows, message, switched, None))
            })();
            let expired = result
                .as_ref()
                .err()
                .is_some_and(|error| unauthorized(error.as_ref()))
                || matches!(&result, Ok((Err(error), _, _, _)) if error.status == Some(401));
            if expired {
                let _ = credentials::remove_background();
            }
            (result, expired)
        });
        cx.spawn(async move |this, cx| {
            let (result, expired) = task.await;
            let _ = this.update(cx, |app, cx| {
                app.account.busy = None;
                app.invalidate_profile_list_read();
                if expired {
                    app.account = AccountState {
                        loaded: true,
                        error: Some("Please sign in with Battle.net.".into()),
                        ..AccountState::default()
                    };
                } else if app.preferences.client == client {
                    if applying_review {
                        app.account.profiles.review = None;
                    }
                    if installing_addons {
                        app.refresh_after_change(cx);
                    }
                    match result {
                        Ok((rows, message, switched, review)) => {
                            app.account.profiles.review =
                                review.filter(|_| app.page == crate::Page::Account);
                            app.account.profiles.show_review_files = false;
                            app.account.profiles.edit = None;
                            app.account.profiles.message = message;
                            match rows {
                                Ok(rows) => {
                                    app.remember_cloud_profiles(client, rows);
                                }
                                Err(error) => {
                                    app.account.profiles.error = Some(error.to_string());
                                }
                            }
                            if switched {
                                app.account.sync_conflict = None;
                                app.account.sync_message = None;
                                let selected = app
                                    .root(client)
                                    .and_then(|root| app.selected_local_account(client, root));
                                app.load_settings_backups(selected, cx);
                                app.refresh_account(None, cx);
                            }
                        }
                        Err(error) => {
                            let message = error.to_string();
                            app.show_notification(message.clone(), true, cx);
                            app.account.error = Some(message);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start_cloud_name(
        &mut self,
        profile: Option<CloudProfile>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = profile
            .as_ref()
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        let input = cx.new(crate::search_input::SearchInput::new);
        input.update(cx, |input, cx| {
            input.set_limit(40);
            input.set(&value, cx);
        });
        cx.subscribe(
            &input,
            |app, _, change: &crate::search_input::Changed, cx| {
                if let Some(Edit::Name { value, .. }) = app.account.profiles.edit.as_mut() {
                    *value = change.0.clone();
                    cx.notify();
                }
            },
        )
        .detach();
        self.account.profiles.name_input = Some(input);
        self.account.profiles.edit = Some(Edit::Name { profile, value });
        self.account.profiles.message = None;
        cx.notify();
    }

    pub(super) fn dismiss_cloud_profiles(&mut self) -> bool {
        let state = &mut self.account.profiles;
        if state.review.take().is_some() {
            return true;
        }
        let open = std::mem::take(&mut state.manage);
        state.edit = None;
        state.name_input = None;
        open
    }

    pub(crate) fn cloud_profile_manager_open(&self) -> bool {
        self.page == crate::Page::Account && self.account.profiles.manage
    }

    fn manager_controls(&self, cx: &Context<Self>) -> Vec<FocusHandle> {
        let state = &self.account.profiles;
        let Some(focus) = &state.focus else {
            return vec![];
        };
        if self.account.busy.is_some() {
            return if matches!(state.edit, Some(Edit::Name { .. })) {
                state
                    .name_input
                    .as_ref()
                    .map(|input| input.read(cx).focus.clone())
                    .into_iter()
                    .collect()
            } else if state.edit.is_none() && state.error.is_some() {
                vec![focus.controls[17].clone()]
            } else {
                vec![]
            };
        }
        let mut controls = vec![];
        match &state.edit {
            Some(Edit::Name { value, .. }) => {
                if let Some(input) = &state.name_input {
                    controls.push(input.read(cx).focus.clone());
                }
                controls.push(focus.controls[16].clone());
                if !value.trim().is_empty() {
                    controls.push(focus.controls[17].clone());
                }
            }
            Some(Edit::Delete(_)) => {
                controls.extend([focus.controls[16].clone(), focus.controls[17].clone()]);
            }
            None => {
                let active = self
                    .account
                    .history_scope
                    .as_ref()
                    .map(|scope| scope.profile);
                if state.client == Some(self.preferences.client) {
                    for (index, profile) in state.rows.iter().take(8).enumerate() {
                        controls.push(focus.controls[index * 2].clone());
                        if profile.profile().ok().is_some_and(|id| {
                            matches!(id, crate::settings_backup::Profile::Custom(_))
                                && Some(id) != active
                        }) {
                            controls.push(focus.controls[index * 2 + 1].clone());
                        }
                    }
                }
                controls.push(focus.controls[16].clone());
                if state.error.is_some() || state.client == Some(self.preferences.client) {
                    controls.push(focus.controls[17].clone());
                }
            }
        }
        controls
    }

    pub(crate) fn update_cloud_manager_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.cloud_profile_manager_open() {
            if let Some(focus) = self.account.profiles.focus.take() {
                focus
                    .previous
                    .as_ref()
                    .filter(|_| self.page == crate::Page::Account)
                    .unwrap_or(&self.root_focus)
                    .focus(window);
            }
            return;
        }
        let mode = match self.account.profiles.edit {
            None => 0,
            Some(Edit::Name { .. }) => 1,
            Some(Edit::Delete(_)) => 2,
        };
        let opening = self.account.profiles.focus.is_none();
        let input = self
            .account
            .profiles
            .name_input
            .as_ref()
            .map(|input| input.read(cx).focus.clone());
        let focus = self
            .account
            .profiles
            .focus
            .get_or_insert_with(|| ManagerFocus {
                previous: window.focused(cx),
                controls: (0..18).map(|_| cx.focus_handle().tab_stop(true)).collect(),
                input: None,
                mode,
            });
        let changed = focus.mode != mode;
        let old_control_focused = focus.controls.iter().any(|item| item.is_focused(window))
            || focus
                .input
                .as_ref()
                .is_some_and(|input| input.is_focused(window));
        focus.mode = mode;
        focus.input = input;
        let controls = self.manager_controls(cx);
        if opening && let Some(first) = controls.first() {
            first.focus(window);
        } else if changed && old_control_focused {
            // SyncProfiles replaces its form/list without rerunning useDialog's
            // initial-focus effect. The removed DOM control leaves body focused.
            self.root_focus.focus(window);
        }
    }

    pub(crate) fn cycle_cloud_manager_focus(
        &self,
        backwards: bool,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        let controls = self.manager_controls(cx);
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

    pub(crate) fn close_cloud_profile_manager(&mut self) {
        self.invalidate_profile_list_read();
        self.account.profiles.manage = false;
        self.account.profiles.edit = None;
        self.account.profiles.name_input = None;
        self.account.profiles.focus = None;
    }

    pub(crate) fn cloud_manager_key(
        &self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> bool {
        if !self.cloud_profile_manager_open() || event.keystroke.key != "tab" {
            return false;
        }
        self.cycle_cloud_manager_focus(event.keystroke.modifiers.shift, window, cx);
        true
    }

    pub(crate) fn dismiss_cloud_manager(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.cloud_profile_manager_open() {
            return false;
        }
        if self.account.busy.is_none() {
            self.dismiss_cloud_profiles();
            cx.notify();
        }
        true
    }

    pub(super) fn cloud_profiles_panel(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let busy = self.account.busy.is_some();
        div().mt(px(-8.)).mb(px(18.)).child(
            palette
                .text_button("manage-cloud-profiles", "Manage profiles")
                .line_height(px(13.2))
                .border_0()
                .when(busy, |button| button.opacity(0.45).cursor_default())
                .when(!busy, |button| {
                    crate::outlined_button(
                        button,
                        palette,
                        &self.account_focus.profiles_manage,
                        cx.listener(|app, _: &(), _, cx| {
                            app.account.profiles.manage = true;
                            app.refresh_cloud_profile_list(cx);
                            cx.notify();
                        }),
                    )
                }),
        )
    }

    pub(crate) fn cloud_profile_manager_dialog(
        &self,
        palette: Palette,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Div> {
        if !self.cloud_profile_manager_open() {
            return None;
        }
        let state = &self.account.profiles;
        let focus = state.focus.as_ref()?;
        let busy = self.account.busy.is_some();
        let button = |button: gpui::Stateful<gpui::Div>, index: usize, enabled: bool| {
            button
                .when(enabled && !busy, |button| {
                    crate::button_outline(button, palette, &focus.controls[index]).tab_stop(true)
                })
                .when(!enabled || busy, |button| {
                    button.opacity(0.45).cursor_default()
                })
        };
        let cancel = {
            button(
                palette
                    .button("cancel-cloud-profile-edit", "Cancel")
                    .debug_selector(|| "cancel-cloud-profile-edit".into()),
                16,
                true,
            )
            .on_click(cx.listener(|app, _, _, cx| {
                if app.account.busy.is_none() {
                    app.account.profiles.edit = None;
                    app.account.profiles.name_input = None;
                    cx.notify();
                }
            }))
        };
        let title = match &state.edit {
            Some(Edit::Name {
                profile: Some(_), ..
            }) => "Rename profile".to_owned(),
            Some(Edit::Name { .. }) => "New profile".to_owned(),
            Some(Edit::Delete(profile)) => format!("Delete {}?", profile.name),
            None => format!("{} profiles", self.preferences.client.label()),
        };
        let mut panel = super::dialog_panel("cloud-profile-manager", palette)
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .mt(px(20.))
                    .text_size(px(24.))
                    .line_height(px(29.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(crate::typography::tracked(title, 24., 1.2, -0.7)),
            );
        if let Some(error) = state.error.as_ref().filter(|_| state.edit.is_none()) {
            panel = panel.child(
                palette
                    .label(error.clone())
                    .text_size(px(12.))
                    .line_height(px(22.8))
                    .mt(px(13.))
                    .mb(px(25.)),
            );
        }
        match &state.edit {
            Some(Edit::Name { profile, value }) => {
                let profile = profile.clone();
                let value = value.clone();
                let input = state.name_input.as_ref()?;
                panel = panel
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .mb(px(18.))
                            .text_size(px(14.))
                            .font_weight(gpui::FontWeight(650.))
                            .child("Name")
                            .child(
                                div()
                                    .w(px(176.))
                                    .h(px(18.))
                                    .px(px(2.))
                                    .flex()
                                    .items_center()
                                    .child(crate::search_input::field(input, "", palette.muted, cx))
                                    .on_key_down(cx.listener(
                                        |app, event: &gpui::KeyDownEvent, _, cx| {
                                            if event.keystroke.key == "enter" {
                                                if let Some(Edit::Name { profile, value }) =
                                                    &app.account.profiles.edit
                                                    && !value.trim().is_empty()
                                                {
                                                    app.cloud_profiles_work(
                                                        Work::Save(profile.clone(), value.clone()),
                                                        cx,
                                                    );
                                                }
                                                cx.stop_propagation();
                                            }
                                        },
                                    )),
                            ),
                    )
                    .child(
                        div().flex().justify_end().gap(px(9.)).child(cancel).child(
                            button(
                                palette.primary_button(
                                    "save-cloud-profile-name",
                                    if profile.is_some() {
                                        "Save name"
                                    } else {
                                        "Create profile"
                                    },
                                ),
                                17,
                                !value.trim().is_empty(),
                            )
                            .on_click(cx.listener(
                                move |app, _, _, cx| {
                                    if !value.trim().is_empty() {
                                        app.cloud_profiles_work(
                                            Work::Save(profile.clone(), value.clone()),
                                            cx,
                                        );
                                    }
                                },
                            )),
                        ),
                    );
            }
            Some(Edit::Delete(profile)) => {
                let profile = profile.clone();
                panel = panel.child(palette.label("This permanently removes the profile and its cloud backups. Local WoW files stay on your computers.")
                    .text_size(px(12.)).line_height(px(22.8)).mt(px(13.)).mb(px(25.)))
                    .child(div().flex().justify_end().gap(px(9.)).child(cancel).child(
                        button(palette.primary_button("confirm-cloud-profile-delete", "Delete profile"), 17, true)
                            .on_click(cx.listener(move |app, _, _, cx| app.cloud_profiles_work(Work::Delete(profile.clone()), cx)))));
            }
            None => {
                let active = self
                    .account
                    .history_scope
                    .as_ref()
                    .map(|scope| scope.profile);
                let mut list = div()
                    .id("cloud-profiles-list")
                    .mt(px(16.))
                    .mb(px(16.))
                    .max_h(window.viewport_size().height * 0.42)
                    .overflow_y_scroll();
                if state.client != Some(self.preferences.client) && state.error.is_none() {
                    list = list.child(
                        palette
                            .label("Loading profiles…")
                            .text_size(px(12.))
                            .py(px(13.)),
                    );
                }
                for (index, profile) in state
                    .rows
                    .iter()
                    .take(8)
                    .filter(|_| state.client == Some(self.preferences.client))
                    .enumerate()
                {
                    let (rename, delete) = (profile.clone(), profile.clone());
                    let id = profile.profile().ok();
                    let mut actions = div().flex().items_center().gap(px(9.)).child(
                        button(
                            palette
                                .text_button(("rename-cloud-profile", index), "Rename")
                                .py(px(5.))
                                .line_height(px(13.2))
                                .border_0(),
                            index * 2,
                            true,
                        )
                        .on_click(cx.listener(
                            move |app, _, window, cx| {
                                if app.account.busy.is_none() {
                                    app.start_cloud_name(Some(rename.clone()), window, cx);
                                }
                            },
                        )),
                    );
                    if id.is_some_and(|id| {
                        matches!(id, crate::settings_backup::Profile::Custom(_))
                            && Some(id) != active
                    }) {
                        actions = actions.child(
                            button(
                                palette
                                    .text_button(("delete-cloud-profile", index), "Delete")
                                    .py(px(5.))
                                    .line_height(px(13.2))
                                    .border_0(),
                                index * 2 + 1,
                                true,
                            )
                            .on_click(cx.listener(
                                move |app, _, _, cx| {
                                    if app.account.busy.is_none() {
                                        app.account.profiles.edit =
                                            Some(Edit::Delete(delete.clone()));
                                        cx.notify();
                                    }
                                },
                            )),
                        );
                    }
                    list = list.child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(13.))
                            .py(px(16.))
                            .border_b_1()
                            .border_color(palette.line)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .child(profile.name.clone()),
                                    )
                                    .when(id == active && id.is_some(), |name| {
                                        name.child(
                                            div()
                                                .ml(px(8.))
                                                .px(px(5.))
                                                .py(px(2.))
                                                .border_1()
                                                .border_color(palette.line)
                                                .rounded(px(4.))
                                                .bg(rgb(palette.surface))
                                                .text_size(px(9.))
                                                .line_height(px(10.8))
                                                .text_color(rgb(palette.muted))
                                                .child("This computer"),
                                        )
                                    }),
                            )
                            .child(actions),
                    );
                }
                let retry = state.error.is_some();
                let primary = palette
                    .primary_button(
                        "new-cloud-profile",
                        if retry { "Retry" } else { "New profile" },
                    )
                    .debug_selector(|| "new-cloud-profile".into());
                let primary = if retry {
                    crate::outlined_button(
                        primary,
                        palette,
                        &focus.controls[17],
                        cx.listener(|app, _: &(), _, cx| app.refresh_cloud_profile_list(cx)),
                    )
                } else {
                    button(primary, 17, state.client == Some(self.preferences.client)).on_click(
                        cx.listener(|app, _, window, cx| {
                            if app.account.busy.is_none()
                                && app.account.profiles.client == Some(app.preferences.client)
                            {
                                app.start_cloud_name(None, window, cx);
                            }
                        }),
                    )
                };
                panel = panel.child(list).child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(9.))
                        .child(
                            button(palette.button("close-cloud-profiles", "Done"), 16, true)
                                .on_click(cx.listener(|app, _, _, cx| {
                                    if app.account.busy.is_none() {
                                        app.dismiss_cloud_profiles();
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(primary),
                );
            }
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
                        .id("cloud-profile-manager-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .bg(gpui::rgba(0x050b08b3))
                        .occlude()
                        .on_click(cx.listener(|app, _, _, cx| {
                            if app.account.busy.is_none() {
                                app.dismiss_cloud_profiles();
                                cx.notify();
                            }
                        })),
                )
                .child(panel),
        )
    }
}
