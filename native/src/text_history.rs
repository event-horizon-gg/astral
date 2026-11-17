use gpui::SharedString;
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub text: SharedString,
    pub anchor: usize,
    pub caret: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Typing,
    Backspace,
    ForwardDelete,
    Separate,
}

struct Edit {
    before: Snapshot,
    after: Snapshot,
}

#[derive(Default)]
pub struct History {
    undo: VecDeque<Edit>,
    redo: Vec<Edit>,
    group: Option<Kind>,
    composition: Option<Snapshot>,
}

impl History {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn close_group(&mut self) {
        self.group = None;
    }

    pub fn composing(&self) -> bool {
        self.composition.is_some()
    }

    pub fn can_undo(&self) -> bool {
        !self.composing() && !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.composing() && !self.redo.is_empty()
    }

    pub fn begin_composition(&mut self, before: Snapshot) {
        if self.composition.is_none() {
            self.close_group();
            self.composition = Some(before);
        }
    }

    pub fn commit_composition(&mut self, after: Snapshot) {
        if let Some(before) = self.composition.take() {
            self.record(before, after, Kind::Separate);
        }
    }

    pub fn record(&mut self, before: Snapshot, after: Snapshot, kind: Kind) {
        let (before, kind) = match self.composition.take() {
            Some(original) => (original, Kind::Separate),
            None => (before, kind),
        };
        if before.text == after.text {
            return;
        }
        self.redo.clear();
        // Like the reference browser, typing and forward-delete can extend an
        // open edit. Backward-delete starts a new group after other edit kinds.
        let coalesce = kind != Kind::Separate
            && self
                .group
                .is_some_and(|previous| kind != Kind::Backspace || previous == kind)
            && self.undo.back().is_some_and(|edit| edit.after == before);
        if coalesce {
            self.undo.back_mut().unwrap().after = after;
        } else {
            self.push(Edit { before, after });
        }
        self.group = (kind != Kind::Separate).then_some(kind);
    }

    fn push(&mut self, edit: Edit) {
        // Blink's undo stack uses the same 1,000-action bound; coalesced typing
        // consumes one action rather than one entry per character.
        if self.undo.len() == 1000 {
            self.undo.pop_front();
        }
        self.undo.push_back(edit);
    }

    pub fn undo(&mut self) -> Option<Snapshot> {
        if self.composing() {
            return None;
        }
        self.close_group();
        let edit = self.undo.pop_back()?;
        let snapshot = edit.before.clone();
        self.redo.push(edit);
        Some(snapshot)
    }

    pub fn redo(&mut self) -> Option<Snapshot> {
        if self.composing() {
            return None;
        }
        self.close_group();
        let edit = self.redo.pop()?;
        let snapshot = edit.after.clone();
        self.push(edit);
        Some(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Snapshot {
        Snapshot {
            text: text.to_owned().into(),
            anchor: text.len(),
            caret: text.len(),
        }
    }

    #[test]
    fn continuous_typing_undoes_as_one_action_and_redo_restores_the_edit_selection() {
        let mut history = History::default();
        for (before, after) in [("", "D"), ("D", "De"), ("De", "Details")] {
            history.record(at(before), at(after), Kind::Typing);
        }
        assert_eq!(history.undo(), Some(at("")));
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), Some(at("Details")));
        assert_eq!(history.redo(), None);
    }

    #[test]
    fn consecutive_backspaces_are_separate_from_typing_and_a_new_edit_discards_redo() {
        let mut history = History::default();
        history.record(at(""), at("Details"), Kind::Typing);
        history.record(at("Details"), at("Detail"), Kind::Backspace);
        history.record(at("Detail"), at("Detai"), Kind::Backspace);
        assert_eq!(history.undo(), Some(at("Details")));
        assert_eq!(history.undo(), Some(at("")));
        assert_eq!(history.redo(), Some(at("Details")));
        history.record(at("Details"), at("Details!"), Kind::Typing);
        assert_eq!(history.redo(), None);
        assert_eq!(history.undo(), Some(at("Details")));
    }

    #[test]
    fn selection_changes_and_paste_preserve_unicode_and_reversed_ranges() {
        let mut history = History::default();
        history.record(at(""), at("a🐻é"), Kind::Typing);
        history.close_group();
        let selected = Snapshot {
            text: "a🐻é".into(),
            anchor: 7,
            caret: 1,
        };
        history.record(selected.clone(), at("aWigs"), Kind::Separate);
        assert_eq!(history.undo(), Some(selected));
        assert_eq!(history.redo(), Some(at("aWigs")));
    }

    #[test]
    fn ime_candidates_do_not_become_individual_undo_steps() {
        let mut history = History::default();
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        history.record(at(""), at("g"), Kind::Typing);
        assert!(history.can_undo());
        history.begin_composition(at("g"));
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        history.begin_composition(at("gに"));
        assert_eq!(history.undo(), None);
        history.record(at("gに"), at("g日本"), Kind::Typing);
        assert_eq!(history.undo(), Some(at("g")));
        assert!(history.can_redo());
        assert_eq!(history.redo(), Some(at("g日本")));
        assert!(!history.can_redo());
        history.begin_composition(at("g日本"));
        history.commit_composition(at("g日本語"));
        history.commit_composition(at("g日本語"));
        assert_eq!(history.undo(), Some(at("g日本")));
    }

    #[test]
    fn external_replacement_clears_both_directions_of_history() {
        let mut history = History::default();
        history.record(at(""), at("search"), Kind::Typing);
        history.undo();
        history.clear();
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), None);
    }

    #[test]
    fn history_drops_old_actions_at_the_reference_limit() {
        let mut history = History::default();
        for number in 0..1001 {
            history.record(
                at(&number.to_string()),
                at(&(number + 1).to_string()),
                Kind::Separate,
            );
        }
        for number in (1..1001).rev() {
            assert_eq!(history.undo(), Some(at(&number.to_string())));
        }
        assert_eq!(history.undo(), None);
    }
}
