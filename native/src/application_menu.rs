use gpui::{App, Context, KeyBinding, Menu, MenuItem, SystemMenuType, Window, prelude::*};

gpui::actions!(
    astral_window,
    [
        Hide,
        HideOthers,
        ShowAll,
        CloseWindow,
        Minimize,
        Zoom,
        FullScreen,
        BringToFront
    ]
);

pub fn install(cx: &mut App) {
    crate::search_input::bind_menu_keys(cx);
    let mut app = vec![MenuItem::action("Quit Astral", crate::Quit)];
    if cfg!(target_os = "macos") {
        app = vec![
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Astral", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Astral", crate::Quit),
        ];
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.bind_keys([
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("ctrl-cmd-f", FullScreen, None),
        ]);
    }
    let mut menus = vec![Menu {
        name: "Astral".into(),
        items: app,
    }];
    if cfg!(target_os = "macos") {
        menus.push(Menu {
            name: "File".into(),
            items: vec![MenuItem::action("Close Window", CloseWindow)],
        });
    }
    menus.push(crate::search_input::edit_menu());
    if cfg!(target_os = "macos") {
        menus.push(Menu {
            name: "View".into(),
            items: vec![MenuItem::action("Toggle Full Screen", FullScreen)],
        });
        menus.push(Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
                MenuItem::separator(),
                MenuItem::action("Bring All to Front", BringToFront),
            ],
        });
    }
    cx.set_menus(menus);
}

pub fn window_actions(element: gpui::Div, cx: &Context<crate::Astral>) -> gpui::Div {
    element
        .on_action(cx.listener(|_, _: &CloseWindow, window, cx| {
            // Electron's Close Window follows the same keep-alive path as its
            // titlebar close button. Never remove the view and its timers.
            crate::tray::hide_window(window, cx);
        }))
        .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
        .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
        .on_action(cx.listener(|_, _: &FullScreen, window, _| window.toggle_fullscreen()))
        .on_action(cx.listener(|_, _: &BringToFront, _: &mut Window, cx| {
            for handle in cx.windows() {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
            cx.activate(true);
        }))
}
