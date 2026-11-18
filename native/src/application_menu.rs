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
    } else if cfg!(target_os = "windows") {
        cx.bind_keys([
            KeyBinding::new("ctrl-w", CloseWindow, None),
            KeyBinding::new("ctrl-m", Minimize, None),
            KeyBinding::new("f11", FullScreen, None),
        ]);
    }
    let mut menus = vec![Menu {
        name: if cfg!(target_os = "macos") {
            "Astral"
        } else {
            "File"
        }
        .into(),
        items: app,
    }];
    if cfg!(target_os = "macos") {
        menus.push(Menu {
            name: "File".into(),
            items: vec![MenuItem::action("Close Window", CloseWindow)],
        });
    }
    menus.push(crate::search_input::edit_menu());
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        menus.push(Menu {
            name: "View".into(),
            items: vec![MenuItem::action("Toggle Full Screen", FullScreen)],
        });
    }
    if cfg!(target_os = "macos") {
        menus.push(Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
                MenuItem::separator(),
                MenuItem::action("Bring All to Front", BringToFront),
            ],
        });
    } else if cfg!(target_os = "windows") {
        menus.push(Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Close Window", CloseWindow),
            ],
        });
    }
    cx.set_menus(menus);
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use gpui::{Action, Global, OwnedMenuItem, WindowHandle};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use tray_icon::menu::{
        Menu as NativeMenu, MenuItem as NativeItem, PredefinedMenuItem, Submenu,
    };

    struct Item {
        id: String,
        native: NativeItem,
        action: Box<dyn Action>,
    }

    struct MenuBar {
        // Retain the menu/subclass for the lifetime of the actual app window.
        _menu: NativeMenu,
        items: Vec<Item>,
    }
    impl Global for MenuBar {}

    pub fn install(handle: WindowHandle<crate::Astral>, cx: &mut App) -> Result<(), String> {
        if cx.has_global::<MenuBar>() {
            return Ok(());
        }
        let menu = NativeMenu::new();
        let mut items = Vec::new();
        for definition in cx.get_menus().unwrap_or_default() {
            let label = match definition.name.as_ref() {
                "File" => "&File",
                "Edit" => "&Edit",
                "View" => "&View",
                "Window" => "&Window",
                name => name,
            };
            let submenu = Submenu::new(label, true);
            for definition in definition.items {
                match definition {
                    OwnedMenuItem::Separator => submenu.append(&PredefinedMenuItem::separator()),
                    OwnedMenuItem::Action { name, action, .. } => {
                        let id = format!("astral-app-menu-{}", items.len());
                        // GPUI handles accelerators, avoiding a competing Win32
                        // message loop/TranslateAccelerator pass. Tabs here are
                        // only the conventional visible shortcut label.
                        let shortcut = shortcut_label(action.as_ref());
                        let text = if shortcut.is_empty() {
                            name
                        } else {
                            format!("{name}\t{shortcut}")
                        };
                        let native = NativeItem::with_id(id.clone(), text, false, None);
                        submenu.append(&native).map_err(|error| error.to_string())?;
                        items.push(Item { id, native, action });
                        Ok(())
                    }
                    // Current Windows definitions have no nested/system menus.
                    // Fail explicitly if that changes instead of hiding items.
                    _ => return Err("Unsupported Windows application submenu.".into()),
                }
                .map_err(|error| error.to_string())?;
            }
            menu.append(&submenu).map_err(|error| error.to_string())?;
        }
        handle
            .update(cx, |_, window, _| {
                let raw =
                    HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
                let RawWindowHandle::Win32(hwnd) = raw.as_raw() else {
                    return Err("No Windows handle for the application menu.".into());
                };
                // Called on GPUI's foreground thread while its borrowed HWND is
                // live. muda owns attachment, subclass dispatch and teardown.
                unsafe { menu.init_for_hwnd(hwnd.hwnd.get()) }.map_err(|error| error.to_string())
            })
            .map_err(|error| error.to_string())??;
        cx.set_global(MenuBar { _menu: menu, items });
        let events = crate::menu_events::subscribe();
        cx.spawn(async move |cx| {
            while let Ok(id) = events.recv().await {
                if cx
                    .update(|cx| {
                        let action = cx
                            .global::<MenuBar>()
                            .items
                            .iter()
                            .find(|item| item.id == id)
                            .map(|item| item.action.boxed_clone());
                        if let Some(action) = action {
                            if cx.is_action_available(action.as_ref()) {
                                cx.dispatch_action(action.as_ref());
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        handle
            .update(cx, |_, window, _| window.refresh())
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn shortcut_label(action: &dyn Action) -> &'static str {
        let action = action.as_any();
        if action.is::<crate::Quit>() {
            "Ctrl+Q"
        } else if action.is::<crate::search_input::Undo>() {
            "Ctrl+Z"
        } else if action.is::<crate::search_input::Redo>() {
            "Ctrl+Y"
        } else if action.is::<crate::search_input::Cut>() {
            "Ctrl+X"
        } else if action.is::<crate::search_input::Copy>() {
            "Ctrl+C"
        } else if action.is::<crate::search_input::Paste>() {
            "Ctrl+V"
        } else if action.is::<crate::search_input::PasteMatchStyle>() {
            "Ctrl+Shift+V"
        } else if action.is::<crate::search_input::SelectAll>() {
            "Ctrl+A"
        } else if action.is::<FullScreen>() {
            "F11"
        } else if action.is::<Minimize>() {
            "Ctrl+M"
        } else if action.is::<CloseWindow>() {
            "Ctrl+W"
        } else {
            ""
        }
    }

    pub fn refresh(window: &Window, cx: &mut App) {
        let Some(menu) = cx.try_global::<MenuBar>() else {
            return;
        };
        let items: Vec<_> = menu
            .items
            .iter()
            .map(|item| (item.native.clone(), item.action.boxed_clone()))
            .collect();
        for (item, action) in items {
            let enabled = action.as_any().is::<crate::Quit>()
                || window.is_action_available(action.as_ref(), cx);
            if item.is_enabled() != enabled {
                item.set_enabled(enabled);
            }
        }
    }
}

pub fn install_window(
    handle: gpui::WindowHandle<crate::Astral>,
    cx: &mut App,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows::install(handle, cx)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (handle, cx);
        Ok(())
    }
}

pub fn refresh_window_menu(window: &Window) {
    #[cfg(target_os = "windows")]
    window.on_next_frame(|window, cx| windows::refresh(window, cx));
    #[cfg(not(target_os = "windows"))]
    let _ = window;
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
