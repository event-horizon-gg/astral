use super::{PROMPT, TITLE};
use std::path::PathBuf;

pub(super) fn choose(
    parent: isize,
    directory: Option<PathBuf>,
) -> windows::core::Result<Option<PathBuf>> {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };
    use windows::{
        Win32::{
            Foundation::{ERROR_CANCELLED, HWND},
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
            },
            UI::Shell::{
                FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog,
                IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
            },
        },
        core::{HRESULT, HSTRING, PCWSTR},
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    unsafe {
        if windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(parent as *mut _) == 0 {
            return Err(windows::core::Error::new(
                HRESULT::from_win32(1400),
                "The Astral window closed.",
            ));
        }
        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
        let _apartment = Apartment;
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetOptions(
            dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST,
        )?;
        dialog.SetTitle(&HSTRING::from(TITLE))?;
        dialog.SetOkButtonLabel(&HSTRING::from(PROMPT))?;
        if let Some(directory) = directory {
            let mut wide: Vec<_> = directory.as_os_str().encode_wide().collect();
            wide.push(0);
            if let Ok(item) =
                SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None)
            {
                let _ = dialog.SetFolder(&item);
            }
        }
        if let Err(error) = dialog.Show(Some(HWND(parent as *mut _))) {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error);
        }
        let item = dialog.GetResult()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let result = PathBuf::from(OsString::from_wide(path.as_wide()));
        CoTaskMemFree(Some(path.0.cast()));
        Ok(Some(result))
    }
}
