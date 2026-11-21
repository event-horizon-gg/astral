//! Actual Astral Render on GPUI's TestPlatform: no native window, credentials,
//! operating-system clipboard, service connection or installed game directory.
use super::*;
use gpui::{KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext, point};

pub(crate) fn deferred<T>() -> (impl std::future::Future<Output = T>, impl FnOnce(T)) {
    use std::{
        cell::RefCell,
        rc::Rc,
        task::{Poll, Waker},
    };
    let result = Rc::new(RefCell::new(None));
    let wake = Rc::new(RefCell::new(None::<Waker>));
    let future = std::future::poll_fn({
        let result = result.clone();
        let wake = wake.clone();
        move |cx| {
            *wake.borrow_mut() = Some(cx.waker().clone());
            result
                .borrow_mut()
                .take()
                .map_or(Poll::Pending, Poll::Ready)
        }
    });
    (future, move |value| {
        *result.borrow_mut() = Some(value);
        if let Some(wake) = wake.borrow_mut().take() {
            wake.wake();
        }
    })
}

pub(crate) fn press(cx: &mut VisualTestContext, key: &str) {
    // GPUI's simulate_keystrokes emits key-down/IME input only; real button
    // activation occurs on key-up, so exercise both halves of the OS event.
    cx.run_until_parked();
    cx.simulate_keystrokes(key);
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse(key).unwrap(),
    });
    cx.run_until_parked();
}

fn addon() -> Addon {
    Addon {
        name: "Fixture addon".into(),
        folder: "FixtureAddon".into(),
        version: "1".into(),
        category: "Other".into(),
        source: "Manual",
        modules: vec!["FixtureAddon".into()],
        artwork: None,
    }
}

#[gpui::test]
fn header_account_button_receives_click_above_the_addon_drawer(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut app = Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        );
        app.addons = vec![addon()];
        app.selected = Some("FixtureAddon".into());
        app
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    cx.run_until_parked();
    cx.simulate_click(point(px(948.), px(38.)), Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.menu == Some(ToolbarMenu::Account)));
    assert_eq!(
        view.read_with(cx, |app, _| app.selected.clone()).as_deref(),
        Some("FixtureAddon")
    );
}

#[gpui::test]
fn brand_navigation_preserves_library_search(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut app = Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        );
        app.set_query("saved search", cx);
        app
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    cx.run_until_parked();
    let x = if cfg!(target_os = "macos") { 110. } else { 45. };
    cx.simulate_click(point(px(x), px(38.)), Modifiers::default());
    assert_eq!(
        view.read_with(cx, |app, _| app.query.clone()),
        "saved search"
    );
}

#[gpui::test]
fn tab_order_and_enter_activate_the_brand_as_a_button(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut app = Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        );
        app.library_query = "saved search".into();
        app.page = Page::Discover;
        app
    });
    view.update_in(cx, |app, window, _| app.root_focus.focus(window));
    press(cx, "tab");
    view.update_in(cx, |app, window, _| {
        assert!(app.brand_focus.is_focused(window))
    });
    press(cx, "enter");
    assert!(view.read_with(cx, |app, _| app.page == Page::Library));
    assert_eq!(
        view.read_with(cx, |app, _| app.query.clone()),
        "saved search"
    );
    view.update_in(cx, |app, window, _| {
        assert!(app.brand_focus.is_focused(window))
    });
    press(cx, "tab");
    view.update_in(cx, |app, window, cx| {
        assert!(!app.brand_focus.is_focused(window));
        assert!(app.header_focus.contains_focused(window, cx));
    });
}

#[gpui::test]
fn header_navigation_keeps_quick_find_until_a_result_is_chosen(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut app = Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        );
        app.page = Page::Discover;
        app.quick_find = Some(0);
        app
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    cx.run_until_parked();
    let x = if cfg!(target_os = "macos") { 110. } else { 45. };
    cx.simulate_click(point(px(x), px(38.)), Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.page == Page::Library));
    assert_eq!(view.read_with(cx, |app, _| app.quick_find), Some(0));
    view.update_in(cx, |app, window, _| app.quick_find_rows[1].focus(window));
    press(cx, "enter");
    assert_eq!(view.read_with(cx, |app, _| app.quick_find), None);
    assert!(view.read_with(cx, |app, _| app.page == QUICK_PAGES[1].0));
}

#[gpui::test]
fn quick_find_preserves_the_addon_drawer_when_opened_and_when_a_page_is_chosen(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut app = Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        );
        app.addons = vec![addon()];
        app.selected = Some("FixtureAddon".into());
        app
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    view.update_in(cx, |app, window, _| app.brand_focus.focus(window));
    // Brand → Library → Discover → client selector → Quick find.
    for _ in 0..4 {
        press(cx, "tab");
    }
    press(cx, "enter");
    assert!(view.read_with(cx, |app, _| app.quick_find.is_some()));
    // The header search action opens the palette; only the shortcut toggles it.
    let trigger = cx.debug_bounds("quick-find-trigger").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.quick_find.is_some()));
    // Main's command dialog uses ordinary browser Tab order, not the
    // separate components' useDialog trap or a roving arrow-key list.
    let opener = cx.update(|window, cx| window.focused(cx).unwrap());
    press(cx, "down");
    cx.update(|window, _| assert!(opener.is_focused(window)));
    press(cx, "tab");
    view.update_in(cx, |app, window, cx| {
        assert!(!opener.is_focused(window));
        assert!(app.header_focus.contains_focused(window, cx));
    });
    assert_eq!(
        view.read_with(cx, |app, _| app.selected.clone()).as_deref(),
        Some("FixtureAddon")
    );
    view.update_in(cx, |app, window, _| app.quick_find_rows[1].focus(window));
    press(cx, "enter");
    assert!(view.read_with(cx, |app, _| app.page == QUICK_PAGES[1].0));
    assert_eq!(view.read_with(cx, |app, _| app.quick_find), None);
    assert_eq!(
        view.read_with(cx, |app, _| app.selected.clone()).as_deref(),
        Some("FixtureAddon")
    );
}

#[gpui::test]
fn client_selector_cannot_change_installation_during_release_checks(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut app = Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        );
        app.checking = true;
        app
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    cx.run_until_parked();
    let trigger = cx.debug_bounds("client-selector").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.menu.is_none()));
    // A stale callback must observe the same guard as the disabled control.
    view.update(cx, |app, cx| app.change_client(Client::Forever, cx));
    assert!(view.read_with(cx, |app, _| app.preferences.client == Client::Retail));
    view.update_in(cx, |app, window, _| app.brand_focus.focus(window));
    for _ in 0..3 {
        press(cx, "tab");
    }
    press(cx, "enter");
    // Disabled selects are skipped by Tab, so Quick find is the next action.
    assert!(view.read_with(cx, |app, _| app.quick_find.is_some()));
}

#[gpui::test]
fn focusing_the_brand_does_not_change_its_layout(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        )
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    view.update_in(cx, |app, window, _| app.root_focus.focus(window));
    cx.run_until_parked();
    let before = cx.debug_bounds("astral-brand").unwrap();
    press(cx, "tab");
    view.update_in(cx, |app, window, _| {
        assert!(app.brand_focus.is_focused(window));
    });
    assert_eq!(cx.debug_bounds("astral-brand").unwrap(), before);
}

#[gpui::test]
fn clicking_outside_tools_dismisses_it_and_activates_the_clicked_control(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut app = Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        );
        app.addons = vec![addon()];
        app
    });
    cx.simulate_resize(size(px(1250.), px(700.)));
    cx.run_until_parked();
    let trigger = cx.debug_bounds("tools-trigger").unwrap().center();
    cx.simulate_click(trigger, Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.menu == Some(ToolbarMenu::Tools)));
    let manual = cx.debug_bounds("manual").unwrap().center();
    cx.simulate_click(manual, Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.menu.is_none()));
    assert!(view.read_with(cx, |app, _| app.filter == Filter::Manual));
    cx.simulate_click(trigger, Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.menu == Some(ToolbarMenu::Tools)));
    // The owning trigger must still toggle the menu closed, not dismiss on
    // mouse-down and then accidentally reopen on click.
    cx.simulate_click(trigger, Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.menu.is_none()));
    cx.simulate_click(trigger, Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.menu.is_some()));
    cx.simulate_click(point(px(630.), px(10.)), Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.menu.is_none()));
}

#[gpui::test]
fn folder_choice_owns_the_operation_until_cancel_error_or_selection(cx: &mut TestAppContext) {
    use std::{cell::RefCell, rc::Rc, task::Poll};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("_forever_");
    std::fs::create_dir_all(root.join("Interface/AddOns")).unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        )
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    let response = Rc::new(RefCell::new(None));
    let wake = Rc::new(RefCell::new(None));
    let picker = std::future::poll_fn({
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
    view.update_in(cx, |app, window, cx| {
        app.header_button_focus[5].focus(window);
        let picker = native_directory::restore_focus(picker, window, cx);
        app.await_root_choice(Client::Retail, picker, cx)
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |app, _| app.root_choice_pending));
    let trigger = cx.debug_bounds("client-selector").unwrap().center();
    cx.simulate_click(trigger, Modifiers::default());
    assert!(view.read_with(cx, |app, _| app.menu.is_none()));
    view.update(cx, |app, cx| {
        app.change_client(Client::Forever, cx);
        // A second request cannot replace the first picker's result.
        app.await_root_choice(
            Client::Retail,
            std::future::ready(Ok(Some(root.clone()))),
            cx,
        );
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |app, _| app.preferences.client == Client::Retail));
    *response.borrow_mut() = Some(Ok(None));
    wake.borrow_mut().take().unwrap().wake();
    cx.run_until_parked();
    assert!(view.read_with(cx, |app, _| !app.root_choice_pending
        && app.folder_error.is_none()));
    view.update_in(cx, |app, window, _| {
        assert!(app.header_button_focus[5].is_focused(window))
    });
    assert!(view.read_with(cx, |app, _| {
        app.preferences.folders.iter().all(Option::is_none)
    }));
    view.update(cx, |app, cx| {
        app.await_root_choice(
            Client::Retail,
            std::future::ready(Err("Fixture picker failed".into())),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |app, _| !app.root_choice_pending
        && app.folder_error.is_some()));
    view.update(cx, |app, cx| {
        app.await_root_choice(
            Client::Retail,
            std::future::ready(Ok(Some(root.clone()))),
            cx,
        )
    });
    cx.run_until_parked();
    view.read_with(cx, |app, _| {
        assert!(!app.root_choice_pending);
        assert!(app.folder_error.is_none());
        assert!(app.preferences.client == Client::Forever);
        assert_eq!(app.root(Client::Forever), Some(&root));
        assert!(app.root(Client::Retail).is_none());
    });
}

#[gpui::test]
fn focusing_header_controls_preserves_their_bounds_in_both_themes(cx: &mut TestAppContext) {
    for light in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = Astral::new(
                [None, None],
                Some(directory.path()),
                Some(Client::Retail),
                false,
                cx,
            );
            app.preferences.light_theme = light;
            app
        });
        cx.simulate_resize(size(px(1250.), px(700.)));
        view.update_in(cx, |app, window, _| app.root_focus.focus(window));
        cx.run_until_parked();
        let selectors = [
            "library",
            "discover",
            "client-selector",
            "quick-find-trigger",
            "appearance-trigger",
            "tools-trigger",
            "account-trigger",
        ];
        let before: Vec<_> = selectors
            .iter()
            .map(|selector| cx.debug_bounds(selector).unwrap())
            .collect();
        for index in 0..selectors.len() {
            view.update_in(cx, |app, window, _| {
                app.header_button_focus[index].focus(window)
            });
            cx.run_until_parked();
            for (selector, expected) in selectors.iter().zip(&before) {
                assert_eq!(
                    cx.debug_bounds(selector).unwrap(),
                    *expected,
                    "{selector} moved when focusing {} in light={light}",
                    selectors[index]
                );
            }
        }
    }
}

#[gpui::test]
fn button_focus_indication_follows_mouse_keyboard_and_script_focus(cx: &mut TestAppContext) {
    for light in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = Astral::new(
                [None, None],
                Some(directory.path()),
                Some(Client::Retail),
                false,
                cx,
            );
            app.preferences.light_theme = light;
            app
        });
        cx.simulate_resize(size(px(1250.), px(700.)));
        view.update_in(cx, |app, window, _| app.root_focus.focus(window));
        cx.run_until_parked();
        let appearance = cx.debug_bounds("appearance-trigger").unwrap().center();
        cx.simulate_click(appearance, Modifiers::default());
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(app.header_button_focus[4].is_focused(window));
            assert!(!focus_visible::indicated(
                &app.header_button_focus[4],
                window,
                cx
            ));
        });
        press(cx, "tab");
        view.update_in(cx, |app, window, cx| {
            assert!(focus_visible::indicated(
                &app.header_button_focus[5],
                window,
                cx
            ));
        });
        press(cx, "shift-tab");
        // Clicking the already keyboard-focused button keeps its last focus
        // source, just as Chromium's unchanged-element focus update does.
        cx.simulate_click(appearance, Modifiers::default());
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(focus_visible::indicated(
                &app.header_button_focus[4],
                window,
                cx
            ));
        });
        let brand = cx.debug_bounds("astral-brand").unwrap().center();
        cx.simulate_click(brand, Modifiers::default());
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(!focus_visible::indicated(&app.brand_focus, window, cx));
            app.header_button_focus[4].focus(window);
            assert!(!focus_visible::indicated(
                &app.header_button_focus[4],
                window,
                cx
            ));
        });
        press(cx, "ctrl-a");
        view.update_in(cx, |app, window, cx| {
            assert!(!focus_visible::indicated(
                &app.header_button_focus[4],
                window,
                cx
            ));
        });
        press(cx, "a");
        view.update_in(cx, |app, window, cx| {
            assert!(focus_visible::indicated(
                &app.header_button_focus[4],
                window,
                cx
            ));
        });
    }
}

#[gpui::test]
fn pointer_focus_mode_is_scoped_to_the_window(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (first, first_cx) = cx.add_window_view(|_, cx| {
        Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        )
    });
    let mut first_cx = first_cx.clone();
    let (second, second_cx) = cx.add_window_view(|_, cx| {
        Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Forever),
            false,
            cx,
        )
    });
    first_cx.simulate_resize(size(px(1250.), px(700.)));
    first_cx.run_until_parked();
    let appearance = first_cx
        .debug_bounds("appearance-trigger")
        .unwrap()
        .center();
    first_cx.simulate_click(appearance, Modifiers::default());
    first_cx.run_until_parked();
    second.update_in(second_cx, |app, window, cx| {
        app.brand_focus.focus(window);
        assert!(focus_visible::indicated(&app.brand_focus, window, cx));
    });
    first.update_in(&mut first_cx, |app, window, cx| {
        assert!(!focus_visible::indicated(
            &app.header_button_focus[4],
            window,
            cx
        ));
    });
}

#[gpui::test]
fn folder_choice_still_releases_ownership_if_the_window_closes(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        )
    });
    let (picker, finish) = deferred();
    view.update_in(cx, |app, window, cx| {
        app.brand_focus.focus(window);
        let picker = native_directory::restore_focus(picker, window, cx);
        app.await_root_choice(Client::Retail, picker, cx);
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |app, _| app.root_choice_pending));
    cx.update(|window, _| window.remove_window());
    finish(Ok(None));
    cx.run_until_parked();
    assert!(view.read_with(cx, |app, _| !app.root_choice_pending
        && app.folder_error.is_none()));
}

#[gpui::test]
fn edit_menu_delete_is_selection_only_and_undoable(cx: &mut TestAppContext) {
    use crate::search_input::{DeleteSelection, SelectAll, Undo};
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        )
    });
    view.update_in(cx, |app, window, cx| {
        app.set_query("Moon 🌙", cx);
        app.search_focus.focus(window);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "untouched clipboard".into(),
        ));
    });
    cx.run_until_parked();
    cx.dispatch_action(SelectAll);
    cx.run_until_parked();
    assert!(cx.update(|window, cx| {
        window
            .available_actions(cx)
            .iter()
            .any(|action| action.as_any().is::<DeleteSelection>())
    }));
    cx.dispatch_action(DeleteSelection);
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |app, _| app.query.clone()), "");
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("untouched clipboard".into())
    );
    assert!(!cx.update(|window, cx| {
        window
            .available_actions(cx)
            .iter()
            .any(|action| action.as_any().is::<DeleteSelection>())
    }));
    cx.dispatch_action(Undo);
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |app, _| app.query.clone()), "Moon 🌙");
    press(cx, "right");
    assert!(!cx.update(|window, cx| {
        window
            .available_actions(cx)
            .iter()
            .any(|action| action.as_any().is::<DeleteSelection>())
    }));
    cx.dispatch_action(DeleteSelection);
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |app, _| app.query.clone()), "Moon 🌙");
}

#[gpui::test]
fn popup_open_errors_are_visible_and_release_the_waiting_result(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (_, cx) = cx.add_window_view(|_, cx| {
        Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        )
    });
    let result = std::rc::Rc::new(std::cell::RefCell::new(Some(Some(123))));
    let saved = result.clone();
    cx.update(|window, cx| {
        let task = native_select::report_errors(
            std::future::ready(Err("fixture popup failure".into())),
            window,
            cx,
        );
        cx.foreground_executor()
            .spawn(async move {
                *saved.borrow_mut() = Some(task.await);
            })
            .detach();
    });
    cx.run_until_parked();
    assert_eq!(*result.borrow(), Some(None));
    assert!(cx.debug_bounds("notification").is_some());
}

#[gpui::test]
fn library_filters_empty_action_and_drawer_keep_bounds_and_input_origin(cx: &mut TestAppContext) {
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
            app.addons = vec![addon()];
            app
        });
        cx.simulate_resize(size(px(1250.), px(900.)));
        cx.run_until_parked();
        for (index, selector, expected) in [
            (1, "filter-updates", Filter::Updates),
            (2, "manual", Filter::Manual),
            (0, "all", Filter::All),
        ] {
            let before = cx.debug_bounds(selector).unwrap();
            view.update_in(cx, |app, window, _| {
                app.library_focus.filters[index].focus(window)
            });
            cx.run_until_parked();
            assert_eq!(before, cx.debug_bounds(selector).unwrap());
            press(cx, "enter");
            assert!(view.read_with(cx, |app, _| app.filter == expected));
            assert_eq!(before, cx.debug_bounds(selector).unwrap());
            view.update_in(cx, |app, window, _| app.brand_focus.focus(window));
            cx.simulate_click(before.center(), Modifiers::default());
            cx.run_until_parked();
            view.update_in(cx, |app, window, cx| {
                assert!(!focus_visible::indicated(
                    &app.library_focus.filters[index],
                    window,
                    cx
                ));
            });
            press(cx, "space");
            view.update_in(cx, |app, window, cx| {
                assert!(app.filter == expected);
                assert!(focus_visible::indicated(
                    &app.library_focus.filters[index],
                    window,
                    cx
                ));
            });
            assert_eq!(before, cx.debug_bounds(selector).unwrap());
        }
        view.update_in(cx, |app, _, cx| {
            app.filter = Filter::Updates;
            app.set_query("missing addon", cx);
            cx.notify();
        });
        cx.run_until_parked();
        let empty = cx.debug_bounds("library-empty-action").unwrap();
        view.update_in(cx, |app, window, _| app.library_focus.empty.focus(window));
        cx.run_until_parked();
        assert_eq!(empty, cx.debug_bounds("library-empty-action").unwrap());
        press(cx, "enter");
        assert!(view.read_with(cx, |app, _| app.filter == Filter::All
            && app.query.is_empty()));
        // The source header sits above the drawer's top-right close control.
        // Exercise that control through the keyboard; do not change stacking here.
        for key in ["enter", "space"] {
            view.update_in(cx, |app, window, cx| {
                app.selected = Some("FixtureAddon".into());
                app.library_focus.close.focus(window);
                cx.notify();
            });
            cx.run_until_parked();
            let close = cx.debug_bounds("close-details").unwrap();
            press(cx, key);
            assert!(view.read_with(cx, |app, _| app.selected.is_none()));
            view.update_in(cx, |app, _, cx| {
                app.selected = Some("FixtureAddon".into());
                cx.notify();
            });
            cx.run_until_parked();
            assert_eq!(close, cx.debug_bounds("close-details").unwrap());
        }
        view.update_in(cx, |app, window, cx| {
            let source = app
                .catalog
                .iter()
                .find(|source| source.supports(Client::Retail) && source.repo.is_some())
                .unwrap();
            let mut managed = addon();
            managed.folder = source.folder.clone();
            app.selected = Some(managed.folder.clone());
            app.addons = vec![managed];
            app.brand_focus.focus(window);
            cx.notify();
        });
        cx.run_until_parked();
        let toggle = cx.debug_bounds("addon-auto-update").unwrap();
        cx.simulate_click(toggle.center(), Modifiers::default());
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(app.excluded_from_daily(app.selected.as_deref().unwrap()));
            assert!(!focus_visible::indicated(
                &app.library_focus.daily,
                window,
                cx
            ));
        });
        assert_eq!(toggle, cx.debug_bounds("addon-auto-update").unwrap());
        press(cx, "space");
        view.update_in(cx, |app, window, cx| {
            assert!(!app.excluded_from_daily(app.selected.as_deref().unwrap()));
            assert!(focus_visible::indicated(
                &app.library_focus.daily,
                window,
                cx
            ));
        });
        assert_eq!(toggle, cx.debug_bounds("addon-auto-update").unwrap());
    }
}

#[gpui::test]
fn navigation_menu_focus_keeps_geometry_and_routes_keyboard_and_pointer(cx: &mut TestAppContext) {
    for light in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| {
            Astral::new(
                [None, None],
                Some(directory.path()),
                Some(Client::Retail),
                light,
                cx,
            )
        });
        cx.simulate_resize(size(px(1250.), px(900.)));
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            app.header_button_focus[5].focus(window);
            app.toggle_menu(ToolbarMenu::Tools, window, cx);
        });
        cx.run_until_parked();
        // Menu opening keeps focus at its trigger; Tab enters the first row.
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.tools_focus[0].is_focused(window))
        });
        let first = cx.debug_bounds("tools-row-0").unwrap();
        press(cx, "shift-tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.header_button_focus[5].is_focused(window))
        });
        assert_eq!(first, cx.debug_bounds("tools-row-0").unwrap());
        for (index, expected) in [(0, Page::Backups), (1, Page::Sources), (2, Page::Settings)] {
            view.update_in(cx, |app, window, cx| {
                app.menu = Some(ToolbarMenu::Tools);
                app.tools_focus[index].focus(window);
                cx.notify();
            });
            cx.run_until_parked();
            let selector = ["tools-row-0", "tools-row-1", "tools-row-2"][index];
            let before = cx.debug_bounds(selector).unwrap();
            press(cx, "tab");
            press(cx, "shift-tab");
            assert_eq!(before, cx.debug_bounds(selector).unwrap());
            press(cx, "space");
            assert!(view.read_with(cx, |app, _| app.page == expected && app.menu.is_none()));
        }
        view.update_in(cx, |app, window, cx| {
            app.menu = Some(ToolbarMenu::Account);
            app.brand_focus.focus(window);
            cx.notify();
        });
        cx.run_until_parked();
        let profile = cx.debug_bounds("profiles-and-sync").unwrap();
        cx.simulate_click(profile.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(view.read_with(cx, |app, _| app.page == Page::Account && app.menu.is_none()));
        view.update_in(cx, |app, window, cx| {
            app.quick_find = Some(0);
            app.brand_focus.focus(window);
            cx.notify();
        });
        cx.run_until_parked();
        let quick = cx.debug_bounds("quick-page-1").unwrap();
        view.update_in(cx, |app, window, _| app.quick_find_rows[1].focus(window));
        cx.run_until_parked();
        assert_eq!(quick, cx.debug_bounds("quick-page-1").unwrap());
        press(cx, "enter");
        assert!(view.read_with(cx, |app, _| app.page == QUICK_PAGES[1].0
            && app.quick_find.is_none()));
    }
}

#[gpui::test]
fn addon_row_focus_follows_identity_after_reordering(cx: &mut TestAppContext) {
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
            let mut second = addon();
            second.folder = "SecondFixture".into();
            second.name = "Second addon".into();
            app.addons = vec![addon(), second];
            app
        });
        cx.simulate_resize(size(px(1250.), px(900.)));
        cx.run_until_parked();
        let before = cx.debug_bounds("addon-details:FixtureAddon").unwrap();
        view.update_in(cx, |app, window, _| {
            app.addon_row_focus.borrow()["FixtureAddon"][0].focus(window)
        });
        cx.run_until_parked();
        assert_eq!(
            before,
            cx.debug_bounds("addon-details:FixtureAddon").unwrap()
        );
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.addon_row_focus.borrow()["FixtureAddon"][1].is_focused(window));
            assert!(!app.addon_row_focus.borrow()["SecondFixture"][1].is_focused(window));
            app.addon_row_focus.borrow()["SecondFixture"][1].focus(window);
        });
        let action = cx.debug_bounds("addon-action:SecondFixture").unwrap();
        cx.run_until_parked();
        assert_eq!(
            action,
            cx.debug_bounds("addon-action:SecondFixture").unwrap()
        );
        view.update_in(cx, |app, _, cx| {
            app.addons.reverse();
            cx.notify();
        });
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            assert!(app.addon_row_focus.borrow()["SecondFixture"][1].is_focused(window))
        });
        press(cx, "enter");
        assert_eq!(
            view.read_with(cx, |app, _| app.selected.clone()).as_deref(),
            Some("SecondFixture")
        );
        view.update_in(cx, |app, _, cx| {
            app.selected = None;
            app.addons.retain(|addon| addon.folder != "SecondFixture");
            cx.notify();
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |app, _| {
            !app.addon_row_focus.borrow().contains_key("SecondFixture")
        }));
    }
}

#[gpui::test]
fn discover_card_focus_is_separate_per_action_and_filters_release_removed_handles(
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
            app.catalog = ["FirstFixture", "SecondFixture"]
                .into_iter()
                .map(|name| {
                    serde_json::from_value(serde_json::json!({
                        "id": name, "name": name, "folder": name,
                        "repo": format!("fixture/{name}")
                    }))
                    .unwrap()
                })
                .collect::<Vec<_>>()
                .into();
            app.addons = ["FirstFixture", "SecondFixture"]
                .into_iter()
                .map(|name| {
                    let mut item = addon();
                    item.folder = name.into();
                    item.name = name.into();
                    item
                })
                .collect();
            app.page = Page::Discover;
            app
        });
        cx.simulate_resize(size(px(1250.), px(900.)));
        cx.run_until_parked();
        let first = cx.debug_bounds("discover-installed:FirstFixture").unwrap();
        view.update_in(cx, |app, window, _| {
            app.discovery_card_focus.borrow()["catalog:FirstFixture"][0].focus(window)
        });
        cx.run_until_parked();
        assert_eq!(
            first,
            cx.debug_bounds("discover-installed:FirstFixture").unwrap()
        );
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(
                app.discovery_card_focus.borrow()["catalog:FirstFixture"][1].is_focused(window)
            );
            assert!(
                !app.discovery_card_focus.borrow()["catalog:SecondFixture"][1].is_focused(window)
            );
        });
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(
                app.discovery_card_focus.borrow()["catalog:SecondFixture"][0].is_focused(window)
            )
        });
        press(cx, "space");
        assert!(view.read_with(cx, |app, _| app.page == Page::Library
            && app.query == "SecondFixture"));
        view.update_in(cx, |app, _, cx| {
            app.page = Page::Discover;
            app.set_query("FirstFixture", cx);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |app, _| app.discovery_card_focus.borrow().len() == 1));
        assert!(cx.debug_bounds("discover-installed:FirstFixture").is_some());
        assert!(view.read_with(cx, |app, _| {
            !app.discovery_card_focus
                .borrow()
                .contains_key("catalog:SecondFixture")
        }));
    }
}

#[gpui::test]
fn utility_controls_keep_bounds_and_sources_keep_their_keyboard_destinations(
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
            app.set_query("fixture search", cx);
            app
        });
        cx.simulate_resize(size(px(1250.), px(1000.)));
        cx.run_until_parked();
        let clear = cx.debug_bounds("clear-search").unwrap();
        view.update_in(cx, |app, window, _| app.search_focus.focus(window));
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.utility_focus.clear_search.is_focused(window))
        });
        assert_eq!(clear, cx.debug_bounds("clear-search").unwrap());
        press(cx, "enter");
        assert!(view.read_with(cx, |app, _| app.query.is_empty()));
        view.update_in(cx, |app, window, cx| {
            app.navigate(Page::Sources, window, cx)
        });
        cx.run_until_parked();
        for (index, selector, url) in [
            (0, "source-link-0", "https://github.com"),
            (1, "source-link-1", "https://addons.wago.io"),
            (2, "source-link-2", "https://www.curseforge.com/wow"),
        ] {
            let before = cx.debug_bounds(selector).unwrap();
            view.update_in(cx, |app, window, _| {
                app.utility_focus.sources[index].focus(window)
            });
            cx.run_until_parked();
            assert_eq!(before, cx.debug_bounds(selector).unwrap());
            press(cx, "enter");
            assert_eq!(cx.opened_url().as_deref(), Some(url));
            assert_eq!(before, cx.debug_bounds(selector).unwrap());
            if index < 2 {
                press(cx, "tab");
                view.update_in(cx, |app, window, _| {
                    assert!(app.utility_focus.sources[index + 1].is_focused(window))
                });
            }
        }
    }
}

#[gpui::test]
fn checkbox_labels_keep_focus_and_only_space_toggles_from_keyboard(cx: &mut TestAppContext) {
    for light in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = Astral::new(
                [None, None],
                Some(directory.path()),
                Some(Client::Retail),
                false,
                cx,
            );
            app.page = Page::Settings;
            app.fixture = false;
            app.preferences.light_theme = light;
            app
        });
        cx.simulate_resize(size(px(1250.), px(1400.)));
        cx.run_until_parked();
        let before = cx.debug_bounds("app-update-auto").unwrap();
        let previous = view.read_with(cx, |app, _| app.preferences.app_update_checks);
        // Clicking the label, rather than the tiny box, must focus and toggle it.
        cx.simulate_click(
            before.origin + point(px(15.), px(15.)),
            Modifiers::default(),
        );
        cx.run_until_parked();
        view.update_in(cx, |app, window, cx| {
            assert!(
                app.settings_focus.checkboxes[2].is_focused(window),
                "bounds={before:?} fixture={} enabled={} has_prefs={} current={:?}",
                app.fixture,
                app.preferences.app_update_checks,
                app.preferences_path.is_some(),
                window.focused(cx)
            );
            assert!(!focus_visible::indicated(
                &app.settings_focus.checkboxes[2],
                window,
                cx
            ));
            assert_eq!(app.preferences.app_update_checks, !previous);
        });
        press(cx, "enter");
        assert_eq!(
            view.read_with(cx, |app, _| app.preferences.app_update_checks),
            !previous
        );
        press(cx, "space");
        view.update_in(cx, |app, window, cx| {
            assert_eq!(app.preferences.app_update_checks, previous);
            assert!(focus_visible::indicated(
                &app.settings_focus.checkboxes[2],
                window,
                cx
            ));
        });
        assert_eq!(cx.debug_bounds("app-update-auto").unwrap(), before);
        // Re-rendering the page doesn't replace the checkbox's focus handle.
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(app.settings_focus.update_check.is_focused(window));
        });
        // Disabled update controls cannot toggle by label or re-enter Tab order.
        view.update(cx, |app, cx| {
            app.fixture = true;
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_click(before.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |app, _| app.preferences.app_update_checks),
            previous
        );
        view.update_in(cx, |app, window, _| {
            app.header_button_focus[6].focus(window)
        });
        press(cx, "tab");
        view.update_in(cx, |app, window, _| {
            assert!(!app.settings_focus.checkboxes[2].is_focused(window));
            assert!(!app.settings_focus.update_check.is_focused(window));
        });
    }
}

#[gpui::test]
fn addon_daily_checkbox_uses_space_without_enter_button_activation(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut app = Astral::new(
            [None, None],
            Some(directory.path()),
            Some(Client::Retail),
            false,
            cx,
        );
        let mut item = addon();
        item.source = "GitHub";
        app.catalog = vec![
            serde_json::from_value(serde_json::json!({
                "id": "FixtureAddon", "name": "Fixture addon", "folder": "FixtureAddon",
                "repo": "fixture/addon"
            }))
            .unwrap(),
        ]
        .into();
        app.addons = vec![item];
        app.selected = Some("FixtureAddon".into());
        app
    });
    cx.simulate_resize(size(px(1250.), px(900.)));
    view.update_in(cx, |app, window, _| app.library_focus.daily.focus(window));
    cx.run_until_parked();
    assert!(!view.read_with(cx, |app, _| app.excluded_from_daily("FixtureAddon")));
    press(cx, "enter");
    assert!(!view.read_with(cx, |app, _| app.excluded_from_daily("FixtureAddon")));
    press(cx, "space");
    assert!(view.read_with(cx, |app, _| app.excluded_from_daily("FixtureAddon")));
    press(cx, "space");
    assert!(!view.read_with(cx, |app, _| app.excluded_from_daily("FixtureAddon")));
}
