//! Native folder selection with Electron's title/default path/button label.
//! Never construct a panel on GPUI TestWindow; inject its result instead.
#[cfg(target_os = "windows")]
#[path = "native_directory_windows.rs"]
mod windows_backend;

use gpui::{App, Task, Window};
use std::{future::Future, path::PathBuf};

const TITLE: &str = "Select your WoW game folder";
const PROMPT: &str = "Select folder";
type Choice = Result<Option<PathBuf>, String>;

pub(crate) fn restore_focus(
    picker: impl Future<Output = Choice> + 'static,
    window: &Window,
    cx: &App,
) -> Task<Choice> {
    let previous = window.focused(cx);
    let owner = window.window_handle();
    let mut app = cx.to_async();
    cx.foreground_executor().spawn(async move {
        let choice = picker.await;
        // Best effort only: closing a window must not prevent the controller
        // from receiving its result and releasing operation ownership.
        let _ = owner.update(&mut app, |_, window, _| {
            if let Some(previous) = previous {
                previous.focus(window);
            }
        });
        choice
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn choose(
    directory: Option<PathBuf>,
    window: &Window,
    cx: &App,
) -> Result<Task<Choice>, String> {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSOpenPanel, NSView};
    use objc2_foundation::{NSString, NSURL};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{
        ffi::{CStr, CString, OsString},
        os::unix::ffi::{OsStrExt, OsStringExt},
        ptr::NonNull,
    };
    let raw = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
    let RawWindowHandle::AppKit(raw) = raw.as_raw() else {
        return Err("The folder chooser has no Mac window.".into());
    };
    let view = unsafe { Retained::retain(raw.ns_view.as_ptr().cast::<NSView>()) }
        .ok_or("The folder chooser window closed.")?;
    let parent = view.window().ok_or("The folder chooser window closed.")?;
    Ok(cx.foreground_executor().spawn(async move {
        let mtm = objc2::MainThreadMarker::new()
            .ok_or("Folder selection must run on the main thread.")?;
        if !parent.isVisible() {
            return Ok(None);
        }
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setTitle(Some(&NSString::from_str(TITLE)));
        panel.setPrompt(Some(&NSString::from_str(PROMPT)));
        panel.setCanChooseFiles(false);
        panel.setCanChooseDirectories(true);
        panel.setAllowsMultipleSelection(false);
        panel.setCanCreateDirectories(false);
        panel.setResolvesAliases(true);
        if let Some(directory) = directory {
            let path = CString::new(directory.as_os_str().as_bytes())
                .map_err(|_| "Invalid folder path.")?;
            let url = unsafe {
                NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                    NonNull::new(path.as_ptr().cast_mut()).unwrap(),
                    true,
                    None,
                )
            };
            panel.setDirectoryURL(Some(&url));
        }
        let (send, receive) = async_channel::bounded(1);
        // The future owns the panel. Its callback uses a weak reference so a
        // platform-retained completion block cannot form a retain cycle.
        let chosen = objc2::rc::Weak::from_retained(&panel);
        let complete = block2::RcBlock::new(move |response| {
            let Some(chosen) = chosen.load() else {
                let _ = send.try_send(Err("The folder chooser window closed.".into()));
                return;
            };
            let result = if response == NSModalResponseOK {
                chosen
                    .URL()
                    .ok_or_else(|| "The selected folder has no file URL.".to_owned())
                    .map(|url| {
                        let bytes =
                            unsafe { CStr::from_ptr(url.fileSystemRepresentation().as_ptr()) }
                                .to_bytes();
                        Some(PathBuf::from(OsString::from_vec(bytes.to_vec())))
                    })
            } else if response == NSModalResponseCancel {
                Ok(None)
            } else {
                Err("The folder chooser could not be displayed.".into())
            };
            chosen.orderOut(None);
            let _ = send.try_send(result);
        });
        panel.beginSheetModalForWindow_completionHandler(&parent, &complete);
        let choice = receive
            .recv()
            .await
            .map_err(|_| "The folder chooser closed without a result.".to_owned())?;
        drop(panel);
        choice
    }))
}

#[cfg(target_os = "windows")]
pub(crate) fn choose(
    directory: Option<PathBuf>,
    window: &Window,
    cx: &App,
) -> Result<Task<Choice>, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let raw = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
    let RawWindowHandle::Win32(raw) = raw.as_raw() else {
        return Err("The folder chooser has no Windows window.".into());
    };
    let parent = raw.hwnd.get();
    Ok(cx.foreground_executor().spawn(async move {
        let (send, receive) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("astral-folder-picker".into())
            .spawn(move || {
                let result =
                    windows_backend::choose(parent, directory).map_err(|error| error.to_string());
                let _ = send.try_send(result);
            })
            .map_err(|error| error.to_string())?;
        receive
            .recv()
            .await
            .map_err(|_| "The folder chooser closed without a result.".to_owned())?
    }))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub(crate) fn choose(_: Option<PathBuf>, _: &Window, _: &App) -> Result<Task<Choice>, String> {
    Err("Folder selection is unavailable on this platform.".into())
}
