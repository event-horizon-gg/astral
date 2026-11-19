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
