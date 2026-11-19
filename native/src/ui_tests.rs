//! Actual Astral Render on GPUI's TestPlatform: no native window, credentials,
//! operating-system clipboard, service connection or installed game directory.
use super::*;
use gpui::{KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext, point};

fn press(cx: &mut VisualTestContext, key: &str) {
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
