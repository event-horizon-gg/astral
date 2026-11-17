use std::io;

#[cfg(target_os = "macos")]
mod platform {
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };
    use std::{io, sync::Mutex};

    static KEYCHAIN_ACCESS: Mutex<()> = Mutex::new(());

    const SERVICE: &str = "com.joaoseixas.astral-native";
    const ACCOUNT: &str = "Battle.net session";
    const ITEM_NOT_FOUND: i32 = -25300;

    fn unavailable() -> io::Error {
        io::Error::other("Unlock macOS Keychain, then try again.")
    }

    fn background_read<T>(read: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        // This flag is process-wide. Serialize every Astral Keychain operation
        // so an explicit sign-in cannot inherit a background read's policy.
        let _access = KEYCHAIN_ACCESS.lock().map_err(|_| unavailable())?;
        let allowed = SecKeychain::user_interaction_allowed().map_err(|_| unavailable())?;
        // Keep an existing noninteractive policy unchanged. The framework guard
        // restores interaction on drop, including when the read returns an error.
        let _policy = if allowed {
            Some(SecKeychain::disable_user_interaction().map_err(|_| unavailable())?)
        } else {
            None
        };
        read()
    }

    pub fn load() -> io::Result<Option<Vec<u8>>> {
        background_read(|| match get_generic_password(SERVICE, ACCOUNT) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(None),
            Err(_) => Err(unavailable()),
        })
    }

    pub fn save(bytes: &[u8]) -> io::Result<()> {
        let _access = KEYCHAIN_ACCESS.lock().map_err(|_| unavailable())?;
        set_generic_password(SERVICE, ACCOUNT, bytes).map_err(|_| unavailable())
    }

    pub fn remove() -> io::Result<()> {
        let _access = KEYCHAIN_ACCESS.lock().map_err(|_| unavailable())?;
        delete()
    }

    pub fn remove_background() -> io::Result<()> {
        background_read(delete)
    }

    fn delete() -> io::Result<()> {
        match delete_generic_password(SERVICE, ACCOUNT) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(()),
            Err(_) => Err(unavailable()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn background_keychain_reads_restore_interaction_after_success_and_failure() {
            let previous = SecKeychain::user_interaction_allowed().unwrap();
            let check = || {
                assert!(!SecKeychain::user_interaction_allowed().unwrap());
            };
            background_read(|| {
                check();
                Ok(())
            })
            .unwrap();
            assert_eq!(SecKeychain::user_interaction_allowed().unwrap(), previous);
            let failed: io::Result<()> = background_read(|| {
                check();
                Err(io::Error::other("Synthetic credential error"))
            });
            assert!(failed.is_err());
            assert_eq!(SecKeychain::user_interaction_allowed().unwrap(), previous);
            if previous {
                let _existing_policy = SecKeychain::disable_user_interaction().unwrap();
                background_read(|| {
                    check();
                    Ok(())
                })
                .unwrap();
                assert!(!SecKeychain::user_interaction_allowed().unwrap());
            }
            assert_eq!(SecKeychain::user_interaction_allowed().unwrap(), previous);
            // No actual credential or Keychain contents are queried in this test.
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{io, ptr, slice};
    use windows_sys::Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{
            CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
            CredReadW, CredWriteW,
        },
    };

    fn target() -> Vec<u16> {
        "Astral Native/Battle.net session\0"
            .encode_utf16()
            .collect()
    }

    fn last_error() -> io::Error {
        io::Error::last_os_error()
    }

    fn unavailable() -> io::Error {
        io::Error::other("Windows Credential Manager is unavailable. Try again.")
    }

    pub fn load() -> io::Result<Option<Vec<u8>>> {
        let mut result = ptr::null_mut();
        // CredReadW allocates the credential and its blob together.
        if unsafe { CredReadW(target().as_ptr(), CRED_TYPE_GENERIC, 0, &mut result) } == 0 {
            return if last_error().raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                Ok(None)
            } else {
                Err(unavailable())
            };
        }
        let bytes = unsafe {
            let credential = &*result;
            let bytes = if credential.CredentialBlob.is_null() || credential.CredentialBlobSize == 0
            {
                Vec::new()
            } else {
                slice::from_raw_parts(
                    credential.CredentialBlob,
                    credential.CredentialBlobSize as usize,
                )
                .to_vec()
            };
            CredFree(result.cast());
            bytes
        };
        Ok(Some(bytes))
    }

    pub fn save(bytes: &[u8]) -> io::Result<()> {
        let mut target = target();
        let mut username: Vec<u16> = "Battle.net\0".encode_utf16().collect();
        let mut bytes = bytes.to_vec();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: bytes.len() as u32,
            CredentialBlob: bytes.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: username.as_mut_ptr(),
            ..Default::default()
        };
        // Target, username, and blob remain allocated for the call.
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            return Err(unavailable());
        }
        Ok(())
    }

    pub fn remove() -> io::Result<()> {
        if unsafe { CredDeleteW(target().as_ptr(), CRED_TYPE_GENERIC, 0) } != 0
            || last_error().raw_os_error() == Some(ERROR_NOT_FOUND as i32)
        {
            Ok(())
        } else {
            Err(unavailable())
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use std::io;
    pub fn load() -> io::Result<Option<Vec<u8>>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn save(_: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn remove() -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub fn load() -> io::Result<Option<crate::cloud::Session>> {
    platform::load()?
        .map(|bytes| {
            let session: crate::cloud::Session = serde_json::from_slice(&bytes)
                .map_err(|_| io::Error::other("Saved sign-in is unreadable. Sign in again."))?;
            session.validate().map_err(io::Error::other)?;
            Ok(session)
        })
        .transpose()
}

pub fn save(session: &crate::cloud::Session) -> io::Result<()> {
    session.validate().map_err(io::Error::other)?;
    platform::save(&serde_json::to_vec(session)?)
}

pub use platform::remove;
#[cfg(not(target_os = "macos"))]
pub use platform::remove as remove_background;
#[cfg(target_os = "macos")]
pub use platform::remove_background;
