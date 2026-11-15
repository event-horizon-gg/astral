use std::{
    ffi::{CString, c_void},
    ptr,
};
use time::OffsetDateTime;

type Handle = *mut *mut c_void;

mod platform {
    use super::Handle;
    #[cfg_attr(target_os = "macos", link(name = "icucore"))]
    #[cfg_attr(target_os = "windows", link(name = "icu", kind = "raw-dylib"))]
    unsafe extern "C" {
        pub fn udatpg_open(locale: *const u8, status: *mut i32) -> Handle;
        pub fn udatpg_close(generator: Handle);
        pub fn udatpg_getBestPatternWithOptions(
            generator: Handle,
            skeleton: *const u16,
            length: i32,
            options: i32,
            output: *mut u16,
            capacity: i32,
            status: *mut i32,
        ) -> i32;
        pub fn udat_open(
            time_style: i32,
            date_style: i32,
            locale: *const u8,
            zone: *const u16,
            zone_length: i32,
            pattern: *const u16,
            pattern_length: i32,
            status: *mut i32,
        ) -> Handle;
        pub fn udat_close(formatter: Handle);
        pub fn udat_format(
            formatter: *const *const std::ffi::c_void,
            timestamp: f64,
            output: *mut u16,
            capacity: i32,
            position: *mut std::ffi::c_void,
            status: *mut i32,
        ) -> i32;
    }
}

struct Owned {
    handle: Handle,
    close: unsafe extern "C" fn(Handle),
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { (self.close)(self.handle) };
    }
}

fn output(mut write: impl FnMut(*mut u16, i32, *mut i32) -> i32) -> Option<Vec<u16>> {
    let mut bytes = vec![0_u16; 128];
    loop {
        let mut status = 0;
        let length = write(
            bytes.as_mut_ptr(),
            bytes.len().try_into().ok()?,
            &mut status,
        );
        if status == 15 && length as usize >= bytes.len() && length < 16384 {
            // U_BUFFER_OVERFLOW_ERROR
            bytes.resize(length as usize + 1, 0);
            continue;
        }
        if status > 0 || length < 0 || length as usize > bytes.len() {
            return None;
        }
        bytes.truncate(length as usize);
        return Some(bytes);
    }
}

/// Use the system ICU pattern generator for the same skeleton requested by
/// Electron Intl. An explicit offset keeps the saved instant in the current
/// system time zone, including the correct summer/winter offset.
pub(crate) fn format_activity_date(date: OffsetDateTime, language: &str) -> Option<String> {
    let locale = CString::new(language).ok()?;
    let offset = date.offset().whole_seconds();
    let zone: Vec<u16> = format!(
        "GMT{}{:02}:{:02}",
        if offset < 0 { '-' } else { '+' },
        offset.unsigned_abs() / 3600,
        offset.unsigned_abs() % 3600 / 60
    )
    .encode_utf16()
    .collect();
    unsafe {
        let mut status = 0;
        let handle = platform::udatpg_open(locale.as_ptr().cast(), &mut status);
        if handle.is_null() {
            return None;
        }
        let generator = Owned {
            handle,
            close: platform::udatpg_close,
        };
        if status > 0 {
            return None;
        }
        let best = |skeleton: &str| {
            let skeleton: Vec<u16> = skeleton.encode_utf16().collect();
            output(|buffer, capacity, status| {
                platform::udatpg_getBestPatternWithOptions(
                    generator.handle,
                    skeleton.as_ptr(),
                    skeleton.len() as i32,
                    2048,
                    buffer,
                    capacity,
                    status,
                )
            })
        };
        let render = |skeleton: &str| -> Option<String> {
            let pattern = best(skeleton)?;
            let mut status = 0;
            let handle = platform::udat_open(
                -2,
                -2,
                locale.as_ptr().cast(),
                zone.as_ptr(),
                zone.len() as i32,
                pattern.as_ptr(),
                pattern.len() as i32,
                &mut status,
            );
            if handle.is_null() {
                return None;
            }
            let formatter = Owned {
                handle,
                close: platform::udat_close,
            };
            if status > 0 {
                return None;
            }
            let value = output(|buffer, capacity, status| {
                platform::udat_format(
                    formatter.handle.cast(),
                    date.unix_timestamp() as f64 * 1000.,
                    buffer,
                    capacity,
                    ptr::null_mut(),
                    status,
                )
            })?;
            String::from_utf16(&value).ok()
        };
        let date_text = render("MMMd")?;
        let time_text = render("jjmm")?;
        combine_components(date, language, &date_text, &time_text)
    }
}

fn combine_components(
    date: OffsetDateTime,
    language: &str,
    date_text: &str,
    time_text: &str,
) -> Option<String> {
    use icu_datetime::{
        DateTimeFormatter,
        fieldsets::{MD, MDT, T},
        input::{Date, DateTime, Time},
        options::TimePrecision,
    };
    let locale: icu_locale_core::Locale = language.replace('_', "-").parse().ok()?;
    let input = DateTime {
        date: Date::try_new_iso(date.year(), u8::from(date.month()), date.day()).ok()?,
        time: Time::try_new(date.hour(), date.minute(), 0, 0).ok()?,
    };
    // The system ICU supplies the explicitly requested component fields. ICU4X
    // supplies the standard CLDR combination, avoiding Apple's prose date/time
    // glue. Match whole components, including units and directional marks.
    let formatter = DateTimeFormatter::try_new(
        locale.clone().into(),
        MDT::medium().with_time_precision(TimePrecision::Minute),
    )
    .ok()?;
    let date_formatter = DateTimeFormatter::try_new(locale.clone().into(), MD::medium()).ok()?;
    let time_formatter = DateTimeFormatter::try_new(
        locale.into(),
        T::short().with_time_precision(TimePrecision::Minute),
    )
    .ok()?;
    let mut text = formatter.format(&input).to_string();
    let date_part = date_formatter.format(&input.date).to_string();
    let time_part = time_formatter.format(&input.time).to_string();
    if date_part.is_empty() || time_part.is_empty() {
        return None;
    }
    let date_start = text.find(&date_part)?;
    let time_start = text.find(&time_part)?;
    let mut ranges = [
        (date_start..date_start + date_part.len(), date_text),
        (time_start..time_start + time_part.len(), time_text),
    ];
    ranges.sort_by_key(|(range, _)| range.start);
    if ranges[0].0.end > ranges[1].0.start {
        return None;
    }
    for (range, value) in ranges.into_iter().rev() {
        text.replace_range(range, value);
    }
    Some(text.replace('\u{202f}', " ").replace('\u{00a0}', " "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activity_dates_match_electron_with_two_digit_hours_and_numeric_days() {
        let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") else {
            return;
        };
        let languages = [
            "en-US", "en-GB", "pt-PT", "pt-BR", "de-DE", "fr-FR", "ja-JP",
        ];
        let dates = [1794690000_i64, 1782864000_i64, 1782896700_i64];
        let script = r#"
            const [languages, dates] = process.argv.slice(1).map(JSON.parse);
            console.log(JSON.stringify(languages.flatMap(locale => dates.map(t => new Date(t*1000)
                .toLocaleString(locale,{month:'short',day:'numeric',hour:'2-digit',minute:'2-digit',timeZone:'UTC'})
                .replaceAll(String.fromCharCode(0x202f),' ').replaceAll(String.fromCharCode(0xa0),' ')))));
        "#;
        let output = std::process::Command::new(node)
            .args([
                "-e",
                script,
                &serde_json::to_string(&languages).unwrap(),
                &serde_json::to_string(&dates).unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        let actual: Vec<_> = languages
            .into_iter()
            .flat_map(|language| {
                dates.into_iter().map(move |timestamp| {
                    format_activity_date(
                        OffsetDateTime::from_unix_timestamp(timestamp).unwrap(),
                        language,
                    )
                    .unwrap()
                })
            })
            .collect();
        assert_eq!(actual, expected);
    }
}
