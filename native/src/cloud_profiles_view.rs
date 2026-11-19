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

#[derive(Default)]
pub(super) struct State {
    client: Option<crate::Client>,
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
    Refresh,
    Save(Option<CloudProfile>, String),
    Delete(CloudProfile),
    Review(CloudProfile),
    Apply(CloudProfile, ProfileReview),
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

impl Astral {
    pub(super) fn remember_cloud_profiles(
        &mut self,
        client: crate::Client,
        rows: Vec<CloudProfile>,
    ) {
        if self.preferences.client == client {
            self.account.profiles.client = Some(client);
            self.account.profiles.rows = rows;
        }
    }

    pub(crate) fn cloud_picker_profiles(&self) -> Option<&[CloudProfile]> {
        (self.account.session.is_some()
            && self.account.profiles.client == Some(self.preferences.client))
        .then_some(self.account.profiles.rows.as_slice())
    }

    pub(crate) fn review_cloud_picker(
        &mut self,
        profile: CloudProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.profile_review_focus.return_to = window.focused(cx);
        self.cloud_profiles_work(Work::Review(profile), cx);
    }

    pub(super) fn seed_cloud_profile_fixture(&mut self) {
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
        if let Some((profile, review)) = self.account.profiles.review.take() {
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
        let operation_feedback = !matches!(&work, Work::Refresh);
        let installing_addons =
            matches!(&work, Work::Apply(_, review) if !review.packages.is_empty());
        if !matches!(&work, Work::Refresh) && self.account.profiles.client != Some(client) {
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
                    Work::Refresh => (None, false),
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
                if expired {
                    app.account = AccountState {
                        loaded: true,
                        error: Some("Please sign in with Battle.net.".into()),
                        ..AccountState::default()
                    };
                } else if app.preferences.client == client {
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
                                    app.account.profiles.rows = rows;
                                    app.account.profiles.client = Some(client);
                                }
                                Err(error) => {
                                    let message = error.to_string();
                                    if operation_feedback {
                                        app.show_notification(message.clone(), true, cx);
                                    }
                                    app.account.error = Some(message);
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
                        Err(error) => app.account.error = Some(error.to_string()),
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
        window: &mut Window,
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
        input.read(cx).focus.focus(window);
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
            return vec![];
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
                if self.account.error.is_some() || state.client == Some(self.preferences.client) {
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
        let focus = self
            .account
            .profiles
            .focus
            .get_or_insert_with(|| ManagerFocus {
                previous: window.focused(cx),
                controls: (0..18).map(|_| cx.focus_handle().tab_stop(true)).collect(),
                mode,
            });
        let changed = focus.mode != mode;
        focus.mode = mode;
        let controls = self.manager_controls(cx);
        if (changed || !controls.iter().any(|focus| focus.is_focused(window)))
            && let Some(first) = controls.first()
        {
            first.focus(window);
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
        self.account.profiles.manage = false;
        self.account.profiles.edit = None;
        self.account.profiles.name_input = None;
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
                    crate::keyboard_button(
                        button,
                        palette,
                        cx.listener(|app, _: &(), _, cx| {
                            app.account.profiles.manage = true;
                            app.cloud_profiles_work(Work::Refresh, cx);
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
                    button
                        .track_focus(&focus.controls[index])
                        .tab_stop(true)
                        .focus(move |style| style.border_color(rgb(palette.accent)))
                })
                .when(!enabled || busy, |button| {
                    button.opacity(0.45).cursor_default()
                })
        };
        let cancel = {
            button(
                palette.button("cancel-cloud-profile-edit", "Cancel"),
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
        if let Some(error) = &self.account.error {
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
                if state.client != Some(self.preferences.client) && self.account.error.is_none() {
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
                let retry = self.account.error.is_some();
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
                        .child(
                            button(
                                palette.primary_button(
                                    "new-cloud-profile",
                                    if retry { "Retry" } else { "New profile" },
                                ),
                                17,
                                retry || state.client == Some(self.preferences.client),
                            )
                            .on_click(cx.listener(
                                move |app, _, window, cx| {
                                    if app.account.busy.is_some() {
                                        return;
                                    }
                                    if retry {
                                        app.cloud_profiles_work(Work::Refresh, cx);
                                    } else if app.account.profiles.client
                                        == Some(app.preferences.client)
                                    {
                                        app.start_cloud_name(None, window, cx);
                                    }
                                },
                            )),
                        ),
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
