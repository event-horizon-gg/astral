//! Prefix selection for the browser-style profile picker. No timer or app
//! process is needed: the next keystroke expires an old search session.
use std::time::{Duration, Instant};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

#[derive(Default)]
pub struct TypeAhead {
    last: Option<Instant>,
    text: String,
}

impl TypeAhead {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn find(
        &mut self,
        input: &str,
        labels: &[&str],
        current: usize,
        now: Instant,
    ) -> Option<usize> {
        if input.is_empty() || input.chars().any(char::is_control) {
            return None;
        }
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) > Duration::from_secs(1))
        {
            self.text.clear();
        }
        self.last = Some(now);
        self.text.push_str(input);
        // Repeating the same character cycles names; a longer prefix refines
        // the currently highlighted name. This follows normal HTML select use.
        let first = self.text.chars().next()?;
        let cycling = self.text.chars().all(|ch| ch == first);
        let prefix = folded(if cycling { input } else { &self.text });
        if prefix.is_empty() || labels.is_empty() {
            return None;
        }
        let start = current.saturating_add(usize::from(cycling)) % labels.len();
        (0..labels.len())
            .map(|step| (start + step) % labels.len())
            .find(|index| folded(labels[*index].trim_start()).starts_with(&prefix))
    }
}

fn folded(text: &str) -> String {
    text.nfd()
        .filter(|ch| !is_combining_mark(*ch))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Modified letter keys belong to app shortcuts or international text input.
pub fn printable(key: &gpui::Keystroke) -> Option<&str> {
    let m = key.modifiers;
    if m.control || m.alt || m.platform || m.function {
        return None;
    }
    key.key_char
        .as_deref()
        .or_else(|| (key.key.chars().count() == 1).then_some(key.key.as_str()))
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_letters_cycle_and_a_prefix_refines_the_current_option() {
        let mut search = TypeAhead::default();
        let now = Instant::now();
        let labels = ["Regular", "Raid", "Raid healing", "Handheld"];
        assert_eq!(search.find("r", &labels, 0, now), Some(1));
        assert_eq!(search.find("r", &labels, 1, now), Some(2));
        assert_eq!(search.find("r", &labels, 2, now), Some(0));
        search.reset();
        assert_eq!(search.find("r", &labels, 0, now), Some(1));
        assert_eq!(search.find("a", &labels, 1, now), Some(1));
        assert_eq!(search.find("i", &labels, 1, now), Some(1));
        assert_eq!(search.find("d h", &labels, 1, now), Some(2));
    }

    #[test]
    fn expired_sessions_start_a_new_prefix_without_a_background_timer() {
        let mut search = TypeAhead::default();
        let now = Instant::now();
        let labels = ["Regular", "Raid", "Handheld"];
        assert_eq!(search.find("r", &labels, 0, now), Some(1));
        assert_eq!(
            search.find("h", &labels, 1, now + Duration::from_millis(1001)),
            Some(2)
        );
        assert_eq!(
            search.find("a", &labels, 2, now + Duration::from_millis(1100)),
            Some(2)
        );
        search.reset();
        assert_eq!(
            search.find("r", &labels, 2, now + Duration::from_millis(1101)),
            Some(0)
        );
    }

    #[test]
    fn names_match_without_case_accents_or_leading_whitespace() {
        let mut search = TypeAhead::default();
        let now = Instant::now();
        let labels = ["Regular", "\u{a0} Éclipse", "João", "日本"];
        assert_eq!(search.find("ec", &labels, 0, now), Some(1));
        search.reset();
        assert_eq!(search.find("JOA", &labels, 0, now), Some(2));
        search.reset();
        assert_eq!(search.find("日", &labels, 0, now), Some(3));
    }

    #[test]
    fn an_unmatched_prefix_does_not_change_selection_or_fall_back_to_last_letter() {
        let mut search = TypeAhead::default();
        let now = Instant::now();
        assert_eq!(search.find("z", &["Regular", "Handheld"], 0, now), None);
        assert_eq!(search.find("h", &["Regular", "Handheld"], 0, now), None);
        search.reset();
        assert_eq!(search.find("h", &[], 0, now), None);
    }

    #[test]
    fn printable_keys_exclude_shortcuts_navigation_and_altgr() {
        let key = |name: &str, key_char, modifiers| gpui::Keystroke {
            key: name.into(),
            key_char,
            modifiers,
        };
        let plain = gpui::Modifiers::default();
        assert_eq!(printable(&key("h", None, plain)), Some("h"));
        assert_eq!(
            printable(&key(
                "e",
                Some("É".into()),
                gpui::Modifiers {
                    shift: true,
                    ..plain
                }
            )),
            Some("É")
        );
        assert_eq!(printable(&key("down", None, plain)), None);
        assert_eq!(
            printable(&key(
                "h",
                None,
                gpui::Modifiers {
                    control: true,
                    alt: true,
                    ..plain
                }
            )),
            None
        );
        assert_eq!(
            printable(&key(
                "h",
                None,
                gpui::Modifiers {
                    platform: true,
                    ..plain
                }
            )),
            None
        );
    }
}
