//! Typed Win32 ownership and monitor geometry, separate from GPUI for API checks.
use windows_sys::Win32::{Foundation::POINT, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

pub(crate) struct Geometry {
    pub origin: (f32, f32),
    pub work: (f32, f32, f32, f32),
    pub highlight: u32,
    pub highlight_text: u32,
    pub disabled_text: u32,
    pub border: u32,
}

fn rgb(color: u32) -> u32 {
    ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff)
}

pub(crate) fn geometry(owner: isize, scale: f32) -> Result<Geometry, String> {
    let hwnd = owner as *mut std::ffi::c_void;
    if !scale.is_finite() || scale <= 0. {
        return Err("Invalid selection popup scale.".into());
    }
    // The caller captured a live parent HWND on its UI thread. Recheck before
    // reading or attaching: closing/hiding the owner must not create an orphan.
    unsafe {
        if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 {
            return Err("The selection popup's window closed.".into());
        }
        let mut origin = POINT::default();
        if ClientToScreen(hwnd, &mut origin) == 0 {
            return Err("Couldn't position the selection popup.".into());
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return Err("Couldn't read the popup's monitor work area.".into());
        }
        let rect = info.rcWork;
        Ok(Geometry {
            origin: (origin.x as f32 / scale, origin.y as f32 / scale),
            work: (
                rect.left as f32 / scale,
                rect.top as f32 / scale,
                (rect.right - rect.left) as f32 / scale,
                (rect.bottom - rect.top) as f32 / scale,
            ),
            highlight: rgb(GetSysColor(COLOR_HIGHLIGHT)),
            highlight_text: rgb(GetSysColor(COLOR_HIGHLIGHTTEXT)),
            disabled_text: rgb(GetSysColor(COLOR_GRAYTEXT)),
            border: rgb(GetSysColor(COLOR_WINDOWFRAME)),
        })
    }
}

pub(crate) fn attach(popup: isize, owner: isize) -> Result<(), String> {
    let child = popup as *mut std::ffi::c_void;
    let parent = owner as *mut std::ffi::c_void;
    unsafe {
        if IsWindow(child) == 0 || IsWindow(parent) == 0 || IsWindowVisible(parent) == 0 {
            return Err("The selection popup's window closed.".into());
        }
        SetWindowLongPtrW(child, GWLP_HWNDPARENT, owner);
        if GetWindowLongPtrW(child, GWLP_HWNDPARENT) != owner {
            return Err("Couldn't attach the selection popup to Astral.".into());
        }
    }
    Ok(())
}
