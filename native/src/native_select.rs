//! Popup choices use the existing native-menu dependency. Never call this
//! adapter on GPUI TestWindow: its native handles are deliberately unavailable.
use gpui::{App, Bounds, Pixels, Task, Window};

#[derive(Clone, Copy)]
pub(crate) struct Anchor {
    pub bounds: Bounds<Pixels>,
    pub font_size: Pixels,
    // Cocoa supplies its menu appearance; Chromium's Windows list carries
    // the computed field/option styles instead.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub appearance: Appearance,
}

#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct Appearance {
    pub background: u32,
    pub text: u32,
    pub padding_start: Pixels,
}

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
    closed_key_with_disabled(key, mac, labels, current, search, now, &[])
}

pub(crate) fn closed_key_with_disabled(
    key: &gpui::Keystroke,
    mac: bool,
    labels: &[&str],
    current: usize,
    search: &mut crate::type_ahead::TypeAhead,
    now: std::time::Instant,
    disabled: &[usize],
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
        let current = current.min(labels.len() - 1);
        let movement = match name {
            "down" | "right" => Some((current as isize, 1, 1)),
            "up" | "left" => Some((current as isize, -1, 1)),
            "pagedown" => Some((current as isize, 1, 3)),
            "pageup" => Some((current as isize, -1, 3)),
            "home" => Some((-1, 1, 1)),
            "end" => Some((labels.len() as isize, -1, 1)),
            _ => None,
        };
        if let Some((mut index, direction, mut steps)) = movement {
            let mut next = (!disabled.contains(&current)).then_some(current);
            loop {
                index += direction;
                if index < 0 || index >= labels.len() as isize {
                    break;
                }
                steps -= 1;
                if !disabled.contains(&(index as usize)) {
                    next = Some(index as usize);
                    if steps <= 0 {
                        break;
                    }
                }
            }
            return next.map_or(KeyResult::Handled, KeyResult::Select);
        }
    }
    // Space extends a live prefix instead of reopening the popup.
    let text = if name == "space" && !m.control && !m.alt && !m.platform && !m.function {
        Some(" ")
    } else {
        crate::type_ahead::printable(key)
    };
    if let Some(text) = text {
        let searchable: Vec<_> = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                if disabled.contains(&index) {
                    ""
                } else {
                    *label
                }
            })
            .collect();
        return search
            .find(text, &searchable, current, now)
            .map_or(KeyResult::Handled, KeyResult::Select);
    }
    KeyResult::Ignore
}

/// Select fields handle activation on key-down, unlike GPUI buttons.
pub(crate) fn suppress_button_keys(event: &gpui::KeyUpEvent, window: &mut Window, cx: &mut App) {
    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
        window.prevent_default();
        cx.stop_propagation();
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn choose(
    window: &Window,
    cx: &App,
    labels: Vec<String>,
    selected: usize,
    anchor: Anchor,
) -> Result<Task<Option<usize>>, String> {
    choose_with_disabled(window, cx, labels, selected, anchor, Vec::new())
}

#[cfg(target_os = "macos")]
pub(crate) fn choose_with_disabled(
    window: &Window,
    cx: &App,
    labels: Vec<String>,
    selected: usize,
    anchor: Anchor,
    disabled: Vec<usize>,
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
            !disabled.contains(&index),
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
        target.show(&menu, selected, anchor);
        while let Ok(id) = events.try_recv() {
            if let Some(index) = id
                .strip_prefix(&prefix)
                .and_then(|index| index.parse::<usize>().ok())
            {
                if index < count && !disabled.contains(&index) {
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

    fn show(&self, menu: &tray_icon::menu::Menu, selected: usize, anchor: Anchor) {
        use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
        use objc2_app_kit::{NSFont, NSMenu, NSPopUpButtonCell, NSView};
        use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
        use tray_icon::menu::ContextMenu;
        if !self.window.isVisible() {
            return;
        }
        let mtm = MainThreadMarker::from(&*self.view);
        // muda owns the item targets/delegate. Keep that menu alive through
        // synchronous Cocoa tracking without holding an entity/App borrow.
        let Some(native_menu) = (unsafe { Retained::retain(menu.ns_menu().cast::<NSMenu>()) })
        else {
            return;
        };
        let view_bounds = self.view.bounds();
        let frame = cocoa_frame(
            anchor.bounds,
            (view_bounds.origin.x, view_bounds.origin.y),
            view_bounds.size.height,
            self.view.isFlipped(),
        );
        let control = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(
                NSPoint::new(frame.0, frame.1),
                NSSize::new(frame.2, frame.3),
            ),
        );
        self.view.addSubview(&control);
        let control = AttachedControl(control);
        // Chromium uses a popup cell, not a context menu: the selected row
        // aligns over the field and Cocoa constrains it to the field's frame.
        let cell = NSPopUpButtonCell::initTextCell_pullsDown(
            NSPopUpButtonCell::alloc(mtm),
            &NSString::from_str(""),
            false,
        );
        cell.setMenu(Some(&native_menu));
        cell.setAutoenablesItems(false);
        cell.selectItemAtIndex(if selected < native_menu.numberOfItems() as usize {
            selected as isize
        } else {
            -1
        });
        cell.setFont(Some(&NSFont::menuFontOfSize(f64::from(f32::from(
            anchor.font_size,
        )))));
        cell.attachPopUpWithFrame_inView(control.0.bounds(), &control.0);
        cell.performClickWithFrame_inView(control.0.bounds(), &control.0);
    }
}

#[cfg(target_os = "macos")]
struct AttachedControl(objc2::rc::Retained<objc2_app_kit::NSView>);

#[cfg(target_os = "macos")]
impl Drop for AttachedControl {
    fn drop(&mut self) {
        self.0.removeFromSuperview();
    }
}

// GPUI measures logical pixels from the content's top-left; AppKit views can
// use either vertical direction and a nonzero bounds origin.
#[cfg(any(target_os = "macos", test))]
fn cocoa_frame(
    bounds: Bounds<Pixels>,
    origin: (f64, f64),
    height: f64,
    flipped: bool,
) -> (f64, f64, f64, f64) {
    let x = f64::from(f32::from(bounds.left()));
    let y = if flipped {
        f64::from(f32::from(bounds.top()))
    } else {
        height - f64::from(f32::from(bounds.bottom()))
    };
    (
        origin.0 + x,
        origin.1 + y,
        f64::from(f32::from(bounds.size.width)),
        f64::from(f32::from(bounds.size.height)),
    )
}

#[cfg(target_os = "windows")]
#[path = "select_window_windows.rs"]
mod windows_platform;

#[cfg(target_os = "windows")]
pub(crate) fn choose_with_disabled(
    window: &Window,
    cx: &App,
    labels: Vec<String>,
    selected: usize,
    anchor: Anchor,
    disabled: Vec<usize>,
) -> Result<Task<Option<usize>>, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let raw = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
    let RawWindowHandle::Win32(raw) = raw.as_raw() else {
        return Err("The selection popup has no Windows window.".into());
    };
    let owner = raw.hwnd.get();
    let geometry = windows_platform::geometry(owner, window.scale_factor())?;
    let mut font = window.text_style().font();
    font.weight = gpui::FontWeight::NORMAL;
    let font_id = window.text_system().resolve_font(&font);
    let row_height = (window.text_system().ascent(font_id, anchor.font_size)
        + window.text_system().descent(font_id, anchor.font_size))
    .ceil()
    .max(anchor.font_size);
    let mut desired_width = anchor.bounds.size.width;
    for label in &labels {
        let line = window.text_system().shape_line(
            label.clone().into(),
            anchor.font_size,
            &[gpui::TextRun {
                len: label.len(),
                font: font.clone(),
                color: gpui::rgb(anchor.appearance.text).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        desired_width =
            desired_width.max(line.width + anchor.appearance.padding_start + gpui::px(4.));
    }
    let screen_anchor = Bounds::new(
        gpui::point(
            gpui::px(geometry.origin.0) + anchor.bounds.left(),
            gpui::px(geometry.origin.1) + anchor.bounds.top(),
        ),
        anchor.bounds.size,
    );
    let work = Bounds::new(
        gpui::point(gpui::px(geometry.work.0), gpui::px(geometry.work.1)),
        gpui::size(gpui::px(geometry.work.2), gpui::px(geometry.work.3)),
    );
    let desired_height = if labels.is_empty() {
        gpui::px(8.)
    } else {
        row_height * labels.len().min(20) as f32 + gpui::px(2.)
    };
    let mut bounds = crate::select_list::placement(
        screen_anchor,
        gpui::size(desired_width, desired_height),
        work,
    );
    if row_height * labels.len() as f32 + gpui::px(2.) > bounds.size.height {
        bounds = crate::select_list::placement(
            screen_anchor,
            gpui::size(desired_width + gpui::px(9.), desired_height),
            work,
        );
    }
    let style = crate::select_list::Style {
        font,
        font_size: anchor.font_size,
        row_height,
        padding_start: anchor.appearance.padding_start,
        background: anchor.appearance.background,
        text: anchor.appearance.text,
        border: geometry.border,
        highlight: geometry.highlight,
        highlight_text: geometry.highlight_text,
        disabled_text: geometry.disabled_text,
    };
    let options = gpui::WindowOptions {
        window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
        titlebar: None,
        kind: gpui::WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        focus: true,
        display_id: window.display(cx).map(|display| display.id()),
        ..Default::default()
    };
    let task = crate::select_list::open(
        labels,
        disabled,
        selected,
        style,
        options,
        window.window_handle(),
        move |popup| {
            let raw = HasWindowHandle::window_handle(popup).map_err(|error| error.to_string())?;
            let RawWindowHandle::Win32(raw) = raw.as_raw() else {
                return Err("The selection popup has no Windows handle.".into());
            };
            windows_platform::attach(raw.hwnd.get(), owner)
        },
        cx,
    );
    Ok(report_errors(task, window, cx))
}

// Popup-window creation is asynchronous. Report a real failure in the owning
// app before returning an empty result to release its pending operation.
#[cfg(any(target_os = "windows", all(test, feature = "headless-ui-tests")))]
pub(crate) fn report_errors(
    result: impl std::future::Future<Output = Result<Option<usize>, String>> + 'static,
    window: &Window,
    cx: &App,
) -> Task<Option<usize>> {
    let owner = window.window_handle();
    let mut app = cx.to_async();
    cx.foreground_executor().spawn(async move {
        match result.await {
            Ok(choice) => choice,
            Err(error) => {
                let _ = owner.update(&mut app, |root, _, cx| {
                    if let Ok(view) = root.downcast::<crate::Astral>() {
                        view.update(cx, |view, cx| view.show_notification(error, true, cx));
                    }
                });
                None
            }
        }
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn choose(
    _: &Window,
    _: &App,
    _: Vec<String>,
    _: usize,
    _: Anchor,
) -> Result<Task<Option<usize>>, String> {
    Err("The package picker is unavailable on this platform.".into())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn choose_with_disabled(
    _: &Window,
    _: &App,
    _: Vec<String>,
    _: usize,
    _: Anchor,
    _: Vec<usize>,
) -> Result<Task<Option<usize>>, String> {
    Err("The picker is unavailable on this platform.".into())
}

#[cfg(test)]
mod keyboard_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn popup_anchor_preserves_field_size_in_both_cocoa_coordinate_directions() {
        let bounds = Bounds::new(
            gpui::point(gpui::px(14.25), gpui::px(400.5)),
            gpui::size(gpui::px(220.75), gpui::px(35.5)),
        );
        assert_eq!(
            cocoa_frame(bounds, (3., 7.), 600., true),
            (17.25, 407.5, 220.75, 35.5)
        );
        assert_eq!(
            cocoa_frame(bounds, (3., 7.), 600., false),
            (17.25, 171., 220.75, 35.5)
        );
    }

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
    fn disabled_options_are_skipped_by_navigation_and_prefix_search() {
        let labels = ["Select account", "Alpha", "Beta", "Gamma"];
        let mut search = crate::type_ahead::TypeAhead::default();
        let now = Instant::now();
        for (key, current, expected) in [
            ("down", 0, KeyResult::Select(1)),
            ("up", 1, KeyResult::Select(1)),
            ("home", 3, KeyResult::Select(1)),
            ("pagedown", 0, KeyResult::Select(3)),
            ("pageup", 3, KeyResult::Select(1)),
            ("b", 1, KeyResult::Handled),
        ] {
            search.reset();
            assert_eq!(
                closed_key_with_disabled(
                    &gpui::Keystroke::parse(key).unwrap(),
                    false,
                    &labels,
                    current,
                    &mut search,
                    now,
                    &[0, 2]
                ),
                expected
            );
        }
        for key in ["down", "home", "end", "s"] {
            search.reset();
            assert_eq!(
                closed_key_with_disabled(
                    &gpui::Keystroke::parse(key).unwrap(),
                    false,
                    &labels[..1],
                    0,
                    &mut search,
                    now,
                    &[0]
                ),
                KeyResult::Handled
            );
        }
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
