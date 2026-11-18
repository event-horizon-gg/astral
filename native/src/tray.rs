use gpui::{App, Window};

pub fn show(window: &mut Window, cx: &mut App) {
    #[cfg(target_os = "windows")]
    if let Some(hwnd) = native_handle(window) {
        // GPUI's activation brings a window forward but does not unhide one
        // hidden through ShowWindow. SW_SHOW preserves its maximized state.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
            );
        }
    }
    window.activate_window();
    cx.activate(true);
}

pub fn reopen(app: &mut crate::Astral, window: &mut Window, cx: &mut App) {
    show(window, cx);
    if window.focused(cx).is_none() {
        app.root_focus.focus(window);
    }
}

#[cfg(target_os = "windows")]
fn native_handle(window: &Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as _),
        _ => None,
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform {
    use super::*;
    use crate::Astral;
    use gpui::{Global, WindowHandle};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
        menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    const OPEN: &str = "astral-open";
    const QUIT: &str = "astral-quit";
    const TRAY: &str = "astral-main";

    struct StatusIcon(#[allow(dead_code)] TrayIcon);
    impl Global for StatusIcon {}

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Action {
        Open,
        Quit,
    }

    fn menu_action(id: &str) -> Option<Action> {
        match id {
            OPEN => Some(Action::Open),
            QUIT => Some(Action::Quit),
            _ => None,
        }
    }

    fn click_action(event: &TrayIconEvent, windows: bool) -> Option<Action> {
        if windows
            && event.id().as_ref() == TRAY
            && matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            )
        {
            Some(Action::Open)
        } else {
            None
        }
    }

    fn icon() -> Result<Icon, String> {
        let image = image::load_from_memory(include_bytes!("../../assets/icon.png"))
            .map_err(|error| error.to_string())?
            .resize_exact(18, 18, image::imageops::FilterType::Lanczos3)
            .into_rgba8();
        Icon::from_rgba(image.into_raw(), 18, 18).map_err(|error| error.to_string())
    }

    pub(super) fn hide(window: &Window, cx: &App) -> bool {
        #[cfg(target_os = "macos")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let _ = cx;
            if objc2::MainThreadMarker::new().is_none() {
                return false;
            }
            let Ok(raw) = HasWindowHandle::window_handle(window) else {
                return false;
            };
            let RawWindowHandle::AppKit(handle) = raw.as_raw() else {
                return false;
            };
            // GPUI owns this NSView, and the borrowed window handle keeps it
            // alive. AppKit access stays on the foreground/main thread.
            let view = unsafe { handle.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
            let Some(window) = view.window() else {
                return false;
            };
            window.orderOut(None);
            true
        }
        #[cfg(target_os = "windows")]
        {
            let _ = cx;
            let Some(hwnd) = super::native_handle(window) else {
                return false;
            };
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                    hwnd,
                    windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE,
                );
            }
            true
        }
    }

    pub fn install(
        window: WindowHandle<Astral>,
        background: bool,
        cx: &mut App,
    ) -> Result<(), String> {
        if cx.has_global::<StatusIcon>() {
            return Ok(());
        }
        let menu = Menu::new();
        let open = MenuItem::with_id(OPEN, "Open Astral", true, None);
        let separator = PredefinedMenuItem::separator();
        let quit = MenuItem::with_id(QUIT, "Quit Astral", true, None);
        menu.append_items(&[&open, &separator, &quit])
            .map_err(|error| error.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_id(TRAY)
            .with_icon(icon()?)
            .with_tooltip("Astral")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .build()
            .map_err(|error| error.to_string())?;
        cx.set_global(StatusIcon(tray));
        #[cfg(target_os = "macos")]
        if background {
            let main = objc2::MainThreadMarker::new()
                .ok_or("The tray must be created on the main thread.")?;
            objc2_app_kit::NSApplication::sharedApplication(main)
                .setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = background;
        let (sender, receiver) = async_channel::unbounded();
        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(action) = menu_action(event.id.as_ref()) {
                let _ = menu_sender.try_send(action);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            // Electron's macOS menu owns left clicks; selecting Open Astral
            // opens the window. On Windows, a left click opens it directly.
            if let Some(action) = click_action(&event, cfg!(target_os = "windows")) {
                let _ = sender.try_send(action);
            }
        }));
        window
            .update(cx, |_, window, cx| {
                window.on_window_should_close(cx, |window, cx| {
                    // Keep the view and its timers alive. Quit remains a separate
                    // application action and does not use the window close path.
                    !hide(window, cx)
                });
            })
            .map_err(|error| error.to_string())?;
        cx.spawn(async move |cx| {
            while let Ok(action) = receiver.recv().await {
                if cx
                    .update(|cx| match action {
                        Action::Open => {
                            let _ = window.update(cx, |app, window, cx| {
                                reopen(app, window, cx);
                            });
                        }
                        Action::Quit => cx.quit(),
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn tray_gestures_follow_the_platform_menu_behavior() {
            let click = |id: &str, button, button_state| TrayIconEvent::Click {
                id: id.into(),
                position: (0., 0.).into(),
                rect: Default::default(),
                button,
                button_state,
            };
            let open = click(TRAY, MouseButton::Left, MouseButtonState::Up);
            assert_eq!(click_action(&open, true), Some(Action::Open));
            assert_eq!(
                click_action(&open, false),
                None,
                "Mac left click opens the menu, not the window"
            );
            for event in [
                click(TRAY, MouseButton::Right, MouseButtonState::Up),
                click(TRAY, MouseButton::Left, MouseButtonState::Down),
                click("other-tray", MouseButton::Left, MouseButtonState::Up),
            ] {
                assert_eq!(click_action(&event, true), None);
            }
            assert_eq!(menu_action(OPEN), Some(Action::Open));
            assert_eq!(menu_action(QUIT), Some(Action::Quit));
            assert_eq!(menu_action("other-menu"), None);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use platform::install;

pub fn hide_window(window: &Window, cx: &App) -> bool {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        platform::hide(window, cx)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (window, cx);
        false
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn install(_: gpui::WindowHandle<crate::Astral>, _: bool, _: &mut App) -> Result<(), String> {
    Ok(())
}
