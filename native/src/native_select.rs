//! Popup choices use the existing native-menu dependency. Never call this
//! adapter on GPUI TestWindow: its native handles are deliberately unavailable.
use gpui::{App, Pixels, Point, Task, Window};

/// Closed HTML-select keyboard behavior. The OS popup owns its own events
/// after opening; this only handles keys while the field has focus.
#[derive(Debug, PartialEq)]
pub(crate) enum KeyResult {
    Ignore,
    Handled,
    Open,
    Select(usize),
}

pub(crate) fn closed_key(
    key: &gpui::Keystroke,
    mac: bool,
    labels: &[&str],
    current: usize,
    search: &mut crate::type_ahead::TypeAhead,
    now: std::time::Instant,
) -> KeyResult {
    let m = key.modifiers;
    let name = key.key.as_str();
    if ((mac || m.alt) && matches!(name, "up" | "down")) || (name == "f4" && !m.alt && !m.control) {
        return KeyResult::Open;
    }
    if !m.control && !m.alt && !m.platform && !m.function {
        if name == "enter" {
            return if mac {
                KeyResult::Handled
            } else {
                KeyResult::Open
            };
        }
        if name == "space" && !search.active(now) {
            return KeyResult::Open;
        }
    }
    if !mac && !m.modified() && !labels.is_empty() {
        let end = labels.len() - 1;
        let current = current.min(end);
        let next = match name {
            "down" | "right" => Some((current + 1).min(end)),
            "up" | "left" => Some(current.saturating_sub(1)),
            "pagedown" => Some(current.saturating_add(3).min(end)),
            "pageup" => Some(current.saturating_sub(3)),
            "home" => Some(0),
            "end" => Some(end),
            _ => None,
        };
        if let Some(next) = next {
            return KeyResult::Select(next);
        }
    }
    // Space extends a live prefix instead of reopening the popup.
    let text = if name == "space" && !m.control && !m.alt && !m.platform && !m.function {
        Some(" ")
    } else {
        crate::type_ahead::printable(key)
    };
    if let Some(text) = text {
        return search
            .find(text, labels, current, now)
            .map_or(KeyResult::Handled, KeyResult::Select);
    }
    KeyResult::Ignore
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn choose(
    window: &Window,
    cx: &App,
    labels: Vec<String>,
    selected: usize,
    position: Point<Pixels>,
) -> Result<Task<Option<usize>>, String> {
    use tray_icon::menu::{CheckMenuItem, Menu};
    let target = Target::new(window)?;
    let menu = Menu::new();
    let prefix = format!("astral-select-{}-", uuid::Uuid::new_v4());
    for (index, label) in labels.iter().enumerate() {
        // muda parses mnemonic ampersands on both platforms; filenames are literals.
        let label = label.replace('&', "&&");
        let item = CheckMenuItem::with_id(
            format!("{prefix}{index}"),
            label,
            true,
            selected == index,
            None,
        );
        menu.append(&item).map_err(|error| error.to_string())?;
    }
    let events = crate::menu_events::subscribe();
    let count = labels.len();
    Ok(cx.foreground_executor().spawn(async move {
        // Run outside an entity/App update borrow: native menu tracking can
        // pump the platform's event loop until selection or cancellation.
        target.show(&menu, position);
        while let Ok(id) = events.try_recv() {
            if let Some(index) = id
                .strip_prefix(&prefix)
                .and_then(|index| index.parse::<usize>().ok())
            {
                if index < count {
                    return Some(index);
                }
            }
        }
        None
    }))
}

#[cfg(target_os = "macos")]
struct Target {
    view: objc2::rc::Retained<objc2_app_kit::NSView>,
    window: objc2::rc::Retained<objc2_app_kit::NSWindow>,
}

#[cfg(target_os = "macos")]
impl Target {
    fn new(window: &Window) -> Result<Self, String> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let raw = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
        let RawWindowHandle::AppKit(raw) = raw.as_raw() else {
            return Err("The package picker has no Mac window.".into());
        };
        // Called on the foreground thread while GPUI's borrowed view is live.
        // Retain both objects across the deferred native tracking operation.
        let view = unsafe {
            objc2::rc::Retained::retain(raw.ns_view.as_ptr().cast::<objc2_app_kit::NSView>())
        }
        .ok_or_else(|| "The package picker window closed.".to_owned())?;
        let window = view
            .window()
            .ok_or_else(|| "The package picker window closed.".to_owned())?;
        Ok(Self { view, window })
    }

    fn show(&self, menu: &tray_icon::menu::Menu, position: Point<Pixels>) {
        use tray_icon::menu::{ContextMenu, dpi::LogicalPosition};
        if !self.window.isVisible() {
            return;
        }
        // Retained objects stay live on the foreground thread through tracking.
        unsafe {
            menu.show_context_menu_for_nsview(
                (&*self.view as *const objc2_app_kit::NSView).cast(),
                Some(
                    LogicalPosition::new(
                        f64::from(f32::from(position.x)),
                        f64::from(f32::from(position.y)),
                    )
                    .into(),
                ),
            );
        }
    }
}

#[cfg(target_os = "windows")]
struct Target(isize);

#[cfg(target_os = "windows")]
impl Target {
    fn new(window: &Window) -> Result<Self, String> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let raw = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
        let RawWindowHandle::Win32(raw) = raw.as_raw() else {
            return Err("The package picker has no Windows window.".into());
        };
        Ok(Self(raw.hwnd.get()))
    }

    fn show(&self, menu: &tray_icon::menu::Menu, position: Point<Pixels>) {
        use tray_icon::menu::{ContextMenu, dpi::LogicalPosition};
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
        let hwnd = self.0 as *mut std::ffi::c_void;
        // A close/hide may have occurred before this foreground task ran.
        unsafe {
            if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 {
                return;
            }
            menu.show_context_menu_for_hwnd(
                self.0,
                Some(
                    LogicalPosition::new(
                        f64::from(f32::from(position.x)),
                        f64::from(f32::from(position.y)),
                    )
                    .into(),
                ),
            );
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn choose(
    _: &Window,
    _: &App,
    _: Vec<String>,
    _: usize,
    _: Point<Pixels>,
) -> Result<Task<Option<usize>>, String> {
    Err("The package picker is unavailable on this platform.".into())
}

#[cfg(test)]
mod keyboard_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn platform_navigation_clamps_and_popup_keys_do_not_commit_a_choice() {
        let labels = [
            "Choose an addon ZIP",
            "Alpha.zip",
            "Beta.zip",
            "Gamma.zip",
            "Omega.zip",
        ];
        let mut search = crate::type_ahead::TypeAhead::default();
        let now = Instant::now();
        let mut selection = 0;
        for (key, expected) in [
            ("down", 1),
            ("pagedown", 4),
            ("right", 4),
            ("pageup", 1),
            ("up", 0),
            ("left", 0),
            ("end", 4),
            ("home", 0),
        ] {
            let result = closed_key(
                &gpui::Keystroke::parse(key).unwrap(),
                false,
                &labels,
                selection,
                &mut search,
                now,
            );
            assert_eq!(result, KeyResult::Select(expected));
            selection = expected;
        }
        for mac in [true, false] {
            for key in ["alt-down", "alt-up", "f4", "space"] {
                assert_eq!(
                    closed_key(
                        &gpui::Keystroke::parse(key).unwrap(),
                        mac,
                        &labels,
                        2,
                        &mut search,
                        now
                    ),
                    KeyResult::Open
                );
            }
            assert_eq!(
                closed_key(
                    &gpui::Keystroke::parse("ctrl-b").unwrap(),
                    mac,
                    &labels,
                    2,
                    &mut search,
                    now
                ),
                KeyResult::Ignore
            );
            assert_eq!(
                closed_key(
                    &gpui::Keystroke::parse("enter").unwrap(),
                    mac,
                    &labels,
                    2,
                    &mut search,
                    now
                ),
                if mac {
                    KeyResult::Handled
                } else {
                    KeyResult::Open
                }
            );
        }
        assert_eq!(
            closed_key(
                &gpui::Keystroke::parse("down").unwrap(),
                true,
                &labels,
                2,
                &mut search,
                now
            ),
            KeyResult::Open
        );
        assert_eq!(
            closed_key(
                &gpui::Keystroke::parse("left").unwrap(),
                true,
                &labels,
                2,
                &mut search,
                now
            ),
            KeyResult::Ignore
        );
    }

    #[test]
    fn typed_spaces_extend_a_live_prefix_then_expire_to_popup_activation() {
        let labels = [
            "Choose an addon ZIP",
            "Alpha classic.zip",
            "Alpha retail.zip",
        ];
        for mac in [true, false] {
            let mut search = crate::type_ahead::TypeAhead::default();
            let now = Instant::now();
            let mut selection = 0;
            for (step, key) in ["a", "l", "p", "h", "a", "space", "r"]
                .into_iter()
                .enumerate()
            {
                let result = closed_key(
                    &gpui::Keystroke::parse(key).unwrap(),
                    mac,
                    &labels,
                    selection,
                    &mut search,
                    now + Duration::from_millis(step as u64 * 10),
                );
                let KeyResult::Select(next) = result else {
                    panic!("{key}: {result:?}")
                };
                selection = next;
            }
            assert_eq!(selection, 2);
            assert_eq!(
                closed_key(
                    &gpui::Keystroke::parse("space").unwrap(),
                    mac,
                    &labels,
                    selection,
                    &mut search,
                    now + Duration::from_secs(2)
                ),
                KeyResult::Open
            );
        }
    }
}
