use crate::{Astral, Client, native_select, type_ahead};
use gpui::{Bounds, Context, Pixels, Window};

pub(crate) struct State {
    pub choices: Vec<Client>,
    pub bounds: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    search: type_ahead::TypeAhead,
    pending: Option<uuid::Uuid>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            choices: vec![Client::Retail],
            bounds: Default::default(),
            search: Default::default(),
            pending: None,
        }
    }
}

impl State {
    pub fn invalidate(&mut self) {
        self.pending = None;
        self.search.reset();
    }
    pub fn index(&self, current: Client) -> usize {
        self.choices
            .iter()
            .position(|choice| *choice == current)
            .unwrap_or(0)
    }
}

// Match main.cjs state(): offer the existing sibling client folders, or Retail
// when neither is present. Run on the background inventory executor.
pub(crate) fn choices(root: &std::path::Path) -> Vec<Client> {
    let mut clients: Vec<_> = Client::ALL
        .into_iter()
        .filter(|client| {
            root.parent()
                .is_some_and(|parent| parent.join(client.folder()).exists())
        })
        .collect();
    if clients.is_empty() {
        clients.push(Client::Retail);
    }
    clients
}

impl Astral {
    pub(crate) fn choose_client(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture || self.addon_controls_busy() || self.client_picker.pending.is_some() {
            return;
        }
        let Some(bounds) = self.client_picker.bounds.get() else {
            return;
        };
        let labels = self
            .client_picker
            .choices
            .iter()
            .map(|client| client.label().to_owned())
            .collect();
        match native_select::choose(
            window,
            cx,
            labels,
            self.client_picker.index(self.preferences.client),
            native_select::Anchor {
                bounds,
                font_size: gpui::px(if window.viewport_size().width <= gpui::px(780.) {
                    11.
                } else {
                    12.
                }),
                appearance: native_select::Appearance {
                    background: crate::Palette::new(self.preferences.light_theme).panel,
                    text: crate::Palette::new(self.preferences.light_theme).text,
                    padding_start: gpui::px(0.),
                },
            },
        ) {
            Ok(choice) => self.await_client_choice(choice, window, cx),
            Err(error) => self.show_notification(error, true, cx),
        }
    }

    pub(crate) fn await_client_choice(
        &mut self,
        choice: impl std::future::Future<Output = Option<usize>> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.addon_controls_busy() || self.client_picker.pending.is_some() {
            return;
        }
        let previous = self.preferences.client;
        let root = self.root(previous).cloned();
        let choices = self.client_picker.choices.clone();
        let token = uuid::Uuid::new_v4();
        self.client_picker.pending = Some(token);
        cx.spawn_in(window, async move |this, cx| {
            let choice = choice.await;
            let _ = this.update_in(cx, |app, window, cx| {
                if app.client_picker.pending != Some(token) {
                    return;
                }
                app.client_picker.pending = None;
                if app.preferences.client == previous
                    && app.root(previous) == root.as_ref()
                    && !app.addon_controls_busy()
                {
                    if let Some(target) = choice.and_then(|index| choices.get(index)).copied() {
                        app.select_client_choice(target, cx);
                    }
                    app.header_button_focus[2].focus(window);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_client_choice(&mut self, target: Client, cx: &mut Context<Self>) {
        if self.addon_controls_busy() || self.preferences.client == target {
            return;
        }
        let root = self.root(target).cloned().or_else(|| {
            self.root(self.preferences.client)
                .and_then(|root| root.parent())
                .map(|parent| parent.join(target.folder()))
        });
        let Some(root) = root else {
            return;
        };
        if let Err(error) = crate::library::validate_root(&root, target) {
            self.show_notification(error.to_string(), true, cx);
            return;
        }
        if self.root(target).is_none() {
            self.detected_roots[target.index()] = Some(root);
        }
        self.change_client(target, cx);
    }

    pub(crate) fn client_picker_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.header_button_focus[2].is_focused(window)
            || self.addon_controls_busy()
            || self.client_picker.pending.is_some()
        {
            return false;
        }
        let labels: Vec<_> = self
            .client_picker
            .choices
            .iter()
            .map(|client| client.label())
            .collect();
        let index = self.client_picker.index(self.preferences.client);
        match native_select::closed_key(
            &event.keystroke,
            cfg!(target_os = "macos"),
            &labels,
            index,
            &mut self.client_picker.search,
            std::time::Instant::now(),
        ) {
            native_select::KeyResult::Open => self.choose_client(window, cx),
            native_select::KeyResult::Select(index) => {
                self.select_client_choice(self.client_picker.choices[index], cx)
            }
            native_select::KeyResult::Handled => {}
            native_select::KeyResult::Ignore => return false,
        }
        true
    }
}

#[cfg(all(test, feature = "headless-ui-tests"))]
mod ui_tests {
    use super::*;
    use crate::{
        Page,
        ui_tests::{deferred, press},
    };
    use gpui::{TestAppContext, px, size};

    fn fixture(directory: &std::path::Path, light: bool, cx: &mut Context<Astral>) -> Astral {
        let roots = Client::ALL.map(|client| {
            let root = directory.join(client.folder());
            std::fs::create_dir_all(root.join("Interface/AddOns")).unwrap();
            Some(root)
        });
        Astral::new(roots, Some(directory), Some(Client::Retail), light, cx)
    }

    #[gpui::test]
    fn client_prefix_switches_inventory_without_opening_a_toolbar_menu(cx: &mut TestAppContext) {
        for light in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), light, cx));
            cx.simulate_resize(size(px(1250.), px(700.)));
            cx.run_until_parked();
            view.update_in(cx, |app, window, cx| {
                assert_eq!(app.client_picker.choices.len(), 2);
                app.set_query("old library search", cx);
                app.header_button_focus[2].focus(window);
            });
            press(cx, "f");
            view.update_in(cx, |app, window, _| {
                assert!(app.preferences.client == Client::Forever);
                assert!(app.query.is_empty());
                assert!(app.menu.is_none());
                assert!(app.client_picker.pending.is_none());
                assert!(app.header_button_focus[2].is_focused(window));
            });
            press(cx, "r");
            assert!(view.read_with(cx, |app, _| app.preferences.client == Client::Retail));
            view.update_in(cx, |app, window, cx| {
                app.page = Page::Discover;
                app.set_query("retained discovery search", cx);
                app.header_button_focus[2].focus(window);
            });
            press(cx, "f");
            assert_eq!(
                view.read_with(cx, |app, _| app.query.clone()),
                "retained discovery search"
            );
        }
    }

    #[gpui::test]
    fn client_popup_preserves_cancel_and_rechecks_busy_before_applying(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), false, cx));
        cx.run_until_parked();
        for choice in [None, Some(usize::MAX), Some(0)] {
            view.update_in(cx, |app, window, cx| {
                app.await_client_choice(std::future::ready(choice), window, cx)
            });
            cx.run_until_parked();
            view.update_in(cx, |app, window, _| {
                assert!(app.preferences.client == Client::Retail);
                assert!(app.client_picker.pending.is_none());
                assert!(app.header_button_focus[2].is_focused(window));
            });
        }
        let (choice, finish) = deferred();
        view.update_in(cx, |app, window, cx| {
            app.await_client_choice(choice, window, cx);
            app.checking = true;
            app.brand_focus.focus(window);
        });
        finish(Some(1));
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            assert!(app.preferences.client == Client::Retail);
            assert!(app.client_picker.pending.is_none());
            assert!(app.brand_focus.is_focused(window));
        });
        press(cx, "f");
        assert!(view.read_with(cx, |app, _| app.preferences.client == Client::Retail));
    }

    #[gpui::test]
    fn header_client_field_compacts_at_reference_breakpoints(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| fixture(directory.path(), false, cx));
        for (width, expected) in [(1250., 70.), (780., 60.), (520., 60.)] {
            cx.simulate_resize(size(px(width), px(700.)));
            cx.run_until_parked();
            let bounds = cx.debug_bounds("client-selector").unwrap();
            assert_eq!(bounds.size.width, px(expected));
            view.update_in(cx, |app, window, _| {
                app.header_button_focus[2].focus(window)
            });
            cx.run_until_parked();
            assert_eq!(bounds, cx.debug_bounds("client-selector").unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_choices_follow_existing_siblings_and_have_a_retail_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("_retail_");
        assert!(choices(&root) == vec![Client::Retail]);
        std::fs::create_dir(directory.path().join("_forever_")).unwrap();
        assert!(choices(&root) == vec![Client::Forever]);
        std::fs::create_dir(&root).unwrap();
        assert!(choices(&root) == vec![Client::Retail, Client::Forever]);
    }
}
