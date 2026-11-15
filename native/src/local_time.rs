use time::{OffsetDateTime, UtcOffset};

#[cfg(target_os = "macos")]
pub fn offset_at(date: OffsetDateTime) -> Option<UtcOffset> {
    let timestamp = date.unix_timestamp();
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    // localtime_r initializes the caller's buffer on success.
    if unsafe { libc::localtime_r(&timestamp, local.as_mut_ptr()) }.is_null() {
        return None;
    }
    let seconds = i32::try_from(unsafe { local.assume_init() }.tm_gmtoff).ok()?;
    UtcOffset::from_whole_seconds(seconds).ok()
}

#[cfg(target_os = "windows")]
pub fn offset_at(date: OffsetDateTime) -> Option<UtcOffset> {
    use time::{Date, Month};
    use windows_sys::Win32::{
        Foundation::SYSTEMTIME, System::Time::SystemTimeToTzSpecificLocalTimeEx,
    };

    let utc = SYSTEMTIME {
        wYear: date.year().try_into().ok()?,
        wMonth: u16::from(u8::from(date.month())),
        wDay: u16::from(date.day()),
        wHour: u16::from(date.hour()),
        wMinute: u16::from(date.minute()),
        wSecond: u16::from(date.second()),
        ..Default::default()
    };
    let mut local = SYSTEMTIME::default();
    // A null time zone selects the current system zone. Both time buffers are valid for the call.
    if unsafe { SystemTimeToTzSpecificLocalTimeEx(std::ptr::null(), &utc, &mut local) } == 0 {
        return None;
    }
    let month = Month::try_from(u8::try_from(local.wMonth).ok()?).ok()?;
    let wall_time =
        Date::from_calendar_date(i32::from(local.wYear), month, local.wDay.try_into().ok()?)
            .ok()?
            .with_hms(
                local.wHour.try_into().ok()?,
                local.wMinute.try_into().ok()?,
                local.wSecond.try_into().ok()?,
            )
            .ok()?
            .assume_utc();
    UtcOffset::from_whole_seconds(
        (wall_time.unix_timestamp() - date.unix_timestamp())
            .try_into()
            .ok()?,
    )
    .ok()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn offset_at(_: OffsetDateTime) -> Option<UtcOffset> {
    None
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn local_offset_matches_the_system_for_winter_and_summer() {
    for timestamp in [1_767_225_600, 1_782_864_000] {
        let date = OffsetDateTime::from_unix_timestamp(timestamp).unwrap();
        let output = std::process::Command::new("/bin/date")
            .args(["-r", &timestamp.to_string(), "+%z"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let offset = offset_at(date).unwrap();
        let seconds = offset.whole_seconds().unsigned_abs();
        let sign = if offset.is_negative() { '-' } else { '+' };
        let actual = format!("{sign}{:02}{:02}", seconds / 3600, seconds % 3600 / 60);
        assert_eq!(actual, std::str::from_utf8(&output.stdout).unwrap().trim());
    }
}

#[cfg(all(test, target_os = "windows"))]
#[test]
fn local_offset_matches_the_system_for_winter_and_summer() {
    let system_root = std::env::var_os("SystemRoot").unwrap();
    let powershell = std::path::PathBuf::from(system_root)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let output = std::process::Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command",
            "1767225600, 1782864000 | ForEach-Object { [int][DateTimeOffset]::FromUnixTimeSeconds($_).ToLocalTime().Offset.TotalSeconds }"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let expected: Vec<i32> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .split_whitespace()
        .map(|seconds| seconds.parse().unwrap())
        .collect();
    let actual: Vec<i32> = [1_767_225_600, 1_782_864_000]
        .into_iter()
        .map(|timestamp| {
            offset_at(OffsetDateTime::from_unix_timestamp(timestamp).unwrap())
                .unwrap()
                .whole_seconds()
        })
        .collect();
    assert_eq!(actual, expected);
}

/// Use the application's preferred language and system time zone, matching the
/// browser reference's medium date and short time without hard-coding a clock.
#[cfg(target_os = "macos")]
pub fn format_medium_short(date: OffsetDateTime) -> Option<String> {
    format_date(date, false)
}

#[cfg(target_os = "macos")]
pub fn format_activity_date(date: OffsetDateTime) -> Option<String> {
    format_date(date, true)
}

#[cfg(target_os = "macos")]
fn format_date(date: OffsetDateTime, activity: bool) -> Option<String> {
    use core_foundation_sys::{
        array::{CFArrayGetCount, CFArrayGetValueAtIndex},
        base::{CFRelease, CFTypeRef},
        date_formatter::{
            CFDateFormatterCreate, CFDateFormatterCreateStringWithAbsoluteTime,
            kCFDateFormatterMediumStyle, kCFDateFormatterNoStyle, kCFDateFormatterShortStyle,
        },
        locale::{
            CFLocaleCopyCurrent, CFLocaleCopyPreferredLanguages, CFLocaleCreate,
            CFLocaleGetIdentifier,
        },
        string::{
            CFStringCreateCopy, CFStringGetCString, CFStringGetLength, CFStringRef,
            kCFStringEncodingUTF8,
        },
    };
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }
    fn owned(pointer: CFTypeRef) -> Option<Owned> {
        (!pointer.is_null()).then_some(Owned(pointer))
    }
    unsafe fn text(string: CFStringRef) -> Option<String> {
        let _string = owned(string.cast())?;
        let length = usize::try_from(unsafe { CFStringGetLength(string) })
            .ok()?
            .checked_mul(4)?
            .checked_add(1)?;
        let mut bytes = vec![0; length];
        if unsafe {
            CFStringGetCString(
                string,
                bytes.as_mut_ptr().cast(),
                length.try_into().ok()?,
                kCFStringEncodingUTF8,
            )
        } == 0
        {
            return None;
        }
        let end = bytes.iter().position(|byte| *byte == 0)?;
        String::from_utf8(bytes[..end].to_vec()).ok()
    }
    unsafe {
        let languages = CFLocaleCopyPreferredLanguages();
        let languages_guard = owned(languages.cast());
        let locale = if languages_guard.is_some() && CFArrayGetCount(languages) > 0 {
            CFLocaleCreate(
                std::ptr::null(),
                CFArrayGetValueAtIndex(languages, 0).cast(),
            )
        } else {
            CFLocaleCopyCurrent()
        };
        let _locale = owned(locale.cast())?;
        let absolute = date.unix_timestamp() as f64 - 978_307_200.;
        if activity {
            let language = text(CFStringCreateCopy(
                std::ptr::null(),
                CFLocaleGetIdentifier(locale),
            ))?;
            let local = date.checked_to_offset(offset_at(date)?)?;
            return crate::native_dates::format_activity_date(local, &language);
        }
        let part = |date_style, time_style| -> Option<String> {
            let formatter = CFDateFormatterCreate(std::ptr::null(), locale, date_style, time_style);
            let _formatter = owned(formatter.cast())?;
            text(CFDateFormatterCreateStringWithAbsoluteTime(
                std::ptr::null(),
                formatter,
                absolute,
            ))
        };
        let value = format!(
            "{}, {}",
            part(kCFDateFormatterMediumStyle, kCFDateFormatterNoStyle)?,
            part(kCFDateFormatterNoStyle, kCFDateFormatterShortStyle)?
        );
        Some(value.replace('\u{202f}', " ").replace('\u{00a0}', " "))
    }
}

#[cfg(target_os = "windows")]
pub fn format_medium_short(date: OffsetDateTime) -> Option<String> {
    let local = date.checked_to_offset(offset_at(date)?)?;
    crate::localized_dates::format_medium_short(local, &preferred_language()?)
}

#[cfg(target_os = "windows")]
pub fn format_activity_date(date: OffsetDateTime) -> Option<String> {
    let local = date.checked_to_offset(offset_at(date)?)?;
    crate::native_dates::format_activity_date(local, &preferred_language()?)
}

#[cfg(target_os = "windows")]
fn preferred_language() -> Option<String> {
    use windows_sys::Win32::Globalization::{
        GetUserDefaultLocaleName, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME,
    };
    let (mut count, mut length) = (0, 0);
    // The first query returns the MULTI_SZ buffer size, including its terminators.
    unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            std::ptr::null_mut(),
            &mut length,
        );
    }
    if (2..=32768).contains(&length) {
        let mut languages = vec![0_u16; length as usize];
        if unsafe {
            GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &mut count,
                languages.as_mut_ptr(),
                &mut length,
            )
        } != 0
        {
            let end = languages.iter().position(|unit| *unit == 0)?;
            if end > 0 {
                return String::from_utf16(&languages[..end]).ok();
            }
        }
    }
    let mut locale = [0_u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(locale.as_mut_ptr(), locale.len() as i32) };
    (length > 1)
        .then(|| String::from_utf16(&locale[..length as usize - 1]).ok())
        .flatten()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn format_medium_short(_: OffsetDateTime) -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn format_activity_date(_: OffsetDateTime) -> Option<String> {
    None
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
#[test]
fn activity_date_matches_the_electron_intl_reference() {
    let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") else {
        return;
    };
    let dates = [1794690000_i64, 1782864000_i64, 1782896700_i64];
    let output = std::process::Command::new(node).args(["-e", "console.log(JSON.stringify([1794690000,1782864000,1782896700].map(t=>new Date(t*1000).toLocaleString(undefined,{month:'short',day:'numeric',hour:'2-digit',minute:'2-digit'}))))"]).output().unwrap();
    assert!(output.status.success());
    let expected: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
    let actual: Vec<String> = dates
        .into_iter()
        .map(|timestamp| {
            format_activity_date(OffsetDateTime::from_unix_timestamp(timestamp).unwrap()).unwrap()
        })
        .collect();
    assert_eq!(
        actual,
        expected
            .into_iter()
            .map(|value| value.replace('\u{202f}', " ").replace('\u{00a0}', " "))
            .collect::<Vec<_>>()
    );
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
#[test]
fn medium_date_and_short_time_match_the_electron_intl_reference() {
    let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") else {
        return;
    };
    let dates = [1794690000_i64, 1782864000_i64];
    let output = std::process::Command::new(node).args(["-e", "console.log(JSON.stringify([1794690000,1782864000].map(t=>new Date(t*1000).toLocaleString(undefined,{dateStyle:'medium',timeStyle:'short'}))))"]).output().unwrap();
    assert!(output.status.success());
    let expected: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
    let actual: Vec<_> = dates
        .into_iter()
        .map(|timestamp| {
            format_medium_short(OffsetDateTime::from_unix_timestamp(timestamp).unwrap()).unwrap()
        })
        .collect();
    assert_eq!(actual, expected);
}
