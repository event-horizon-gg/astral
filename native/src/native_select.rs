//! Popup choices use the existing native-menu dependency. Never call this
//! adapter on GPUI TestWindow: its native handles are deliberately unavailable.
use gpui::{App, Pixels, Point, Task, Window};

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
