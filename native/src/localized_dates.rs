use icu_datetime::{
    DateTimeFormatter,
    fieldsets::YMDT,
    input::{Date, DateTime, Time},
    options::TimePrecision,
};
use icu_locale_core::Locale;
use time::OffsetDateTime;
/// `wall_time` already carries the system-zone offset for the instant. Calendar
/// and clock conventions come from the application's preferred UI language.
pub(crate) fn format_medium_short(wall_time: OffsetDateTime, language: &str) -> Option<String> {
    let locale: Locale = language.parse().ok()?;
    let formatter = DateTimeFormatter::try_new(
        locale.into(),
        YMDT::medium().with_time_precision(TimePrecision::Minute),
    )
    .ok()?;
    let input = DateTime {
        date: Date::try_new_iso(
            wall_time.year(),
            u8::from(wall_time.month()),
            wall_time.day(),
        )
        .ok()?,
        time: Time::try_new(wall_time.hour(), wall_time.minute(), 0, 0).ok()?,
    };
    Some(
        formatter
            .format(&input)
            .to_string()
            .replace('\u{202f}', " ")
            .replace('\u{00a0}', " "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_dates_match_electron_for_day_month_order_and_hour_cycles() {
        let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") else {
            return;
        };
        let languages = [
            "en-US", "en-GB", "pt-PT", "pt-BR", "de-DE", "fr-FR", "ja-JP",
        ];
        let script = r#"
            const languages = JSON.parse(process.argv[1]);
            const values = languages.flatMap(locale => [1794690000, 1782864000].map(timestamp => {
                return new Date(timestamp * 1000)
                    .toLocaleString(locale, { dateStyle: 'medium', timeStyle: 'short', timeZone: 'UTC' })
                    .replaceAll(String.fromCharCode(0x202f), ' ')
                    .replaceAll(String.fromCharCode(0xa0), ' ');
            }));
            console.log(JSON.stringify(values));
        "#;
        let output = std::process::Command::new(node)
            .args(["-e", script, &serde_json::to_string(&languages).unwrap()])
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
                [1794690000_i64, 1782864000_i64]
                    .into_iter()
                    .map(move |timestamp| {
                        format_medium_short(
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
