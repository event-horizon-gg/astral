use ::windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile, STGM_READ,
        },
        UI::Shell::{IShellLinkW, SLGP_RAWPATH, ShellLink},
    },
    core::{Interface, PCWSTR},
};
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

pub(super) fn run<T: Send + 'static>(
    task: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    std::thread::spawn(move || {
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(io::Error::other)?;
        }
        let _apartment = Apartment;
        task()
    })
    .join()
    .map_err(|_| io::Error::other("Couldn't prepare backup shortcuts."))?
}

pub(super) fn normalize(path: &Path) -> PathBuf {
    let wide: Vec<_> = path.as_os_str().encode_wide().collect();
    let prefix: Vec<_> = r"\\?\".encode_utf16().collect();
    let unc: Vec<_> = r"\\?\UNC\".encode_utf16().collect();
    if wide.starts_with(&unc) {
        let mut normal: Vec<_> = r"\\".encode_utf16().collect();
        normal.extend_from_slice(&wide[unc.len()..]);
        PathBuf::from(OsString::from_wide(&normal))
    } else if wide.starts_with(&prefix) {
        PathBuf::from(OsString::from_wide(&wide[prefix.len()..]))
    } else {
        path.to_owned()
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub(super) fn create(target: &Path, destination: &Path) -> io::Result<()> {
    unsafe {
        // Persist a regular .lnk file even when its target is a directory.
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(io::Error::other)?;
        link.SetPath(PCWSTR(wide(&normalize(target)).as_ptr()))
            .map_err(io::Error::other)?;
        let persist: IPersistFile = link.cast().map_err(io::Error::other)?;
        let staged = tempfile::Builder::new()
            .prefix(".astral-backup-")
            .suffix(".lnk")
            .tempfile_in(destination.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
        persist
            .Save(PCWSTR(wide(&normalize(staged.path())).as_ptr()), true)
            .map_err(|error| {
                io::Error::other(format!("Couldn't save the backup shortcut. {error}"))
            })?;
        staged
            .persist_noclobber(destination)
            .map_err(|error| error.error)?;
        Ok(())
    }
}

pub(super) fn read(path: &Path) -> io::Result<PathBuf> {
    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(io::Error::other)?;
        let persist: IPersistFile = link.cast().map_err(io::Error::other)?;
        persist
            .Load(PCWSTR(wide(&normalize(path)).as_ptr()), STGM_READ)
            .map_err(|error| {
                io::Error::other(format!("Couldn't read the backup shortcut. {error}"))
            })?;
        let mut buffer = vec![0; 32768];
        link.GetPath(&mut buffer, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .map_err(io::Error::other)?;
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .ok_or_else(|| io::Error::other("Invalid backup shortcut."))?;
        Ok(PathBuf::from(OsString::from_wide(&buffer[..end])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_paths_remove_verbatim_prefix_without_changing_unc_or_unicode() {
        assert_eq!(
            normalize(Path::new(r"\\?\C:\Games\Móonkin")),
            PathBuf::from(r"C:\Games\Móonkin")
        );
        assert_eq!(
            normalize(Path::new(r"\\?\UNC\server\share\backups")),
            PathBuf::from(r"\\server\share\backups")
        );
        assert_eq!(
            normalize(Path::new(r"C:\Games\backups")),
            PathBuf::from(r"C:\Games\backups")
        );
    }
}
